// the boot harness: fork a guest as a real pid 1 in a fresh pid namespace
// gives genuine pid-1 semantics (orphan reparenting, fatal-signal protection)
// without qemu. fork into a user+pid+mount namespace, fork again so the
// grandchild is pid 1, run the test body, read verdict lines over a pipe
// needs no root

use std::os::fd::RawFd;

// an assertion verdict from the guest; the parent maps each onto a TestContext
// assertion
pub enum Verdict {
    Pass(String),
    Fail(String),
}

// outcome of a boot. Unsupported is a skip (no user namespaces); Failed is the
// harness breaking, distinct from a guest assertion coming back false
pub enum Boot {
    Ran(Vec<Verdict>),
    Unsupported,
    Failed(String),
}

// the handle the guest asserts through; each call writes one line on the pipe
// format() allocates before write, so this is only sound after a fork when
// spark is single-threaded (the parent's allocator mutexes are not held)
pub struct VerdictWriter {
    fd: RawFd,
}

impl VerdictWriter {
    pub fn ok(&self, cond: bool, what: &str) -> bool {
        self.send(if cond { "OK:" } else { "FAIL:" }, what);
        cond
    }

    fn send(&self, prefix: &str, what: &str) {
        let line = format!("{prefix}{}\n", what.replace('\n', " "));
        let bytes = line.as_bytes();
        // SAFETY: writing owned bytes to a pipe fd. write() is atomic for
        // PIPE_BUF-sized writes and async-signal-safe
        unsafe { libc::write(self.fd, bytes.as_ptr() as *const _, bytes.len()) };
    }
}

// run body as pid 1 in a fresh user+pid+mount namespace, collecting its
// verdicts. body runs across a fork and may allocate; the contract every
// spark fork site relies on (spark forks single-threaded)
pub fn boot<F: FnOnce(&VerdictWriter)>(body: F) -> Boot {
    let (vr, vw) = match pipe() {
        Some(p) => p,
        None => return Boot::Failed("pipe".into()),
    };

    crate::proc::assert_fork_safe();
    // SAFETY: single-threaded. child may allocate because the parent's
    // allocator mutexes are not held when we fork
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        // SAFETY: closing valid pipe fds on fork failure
        unsafe { libc::close(vr); libc::close(vw); }
        return Boot::Failed("fork".into());
    }

    if pid == 0 {
        // child: close read end (only the parent reads), write setup verdict
        unsafe { libc::close(vr) };
        let verdict = match enter_ns(true) {
            Ok(()) => b'S',
            Err(NamespaceEntry::Unsupported) => b'U',
            Err(NamespaceEntry::Other) => b'E',
        };
        unsafe {
            libc::write(vw, [verdict].as_ptr() as *const _, 1);
        }
        if verdict != b'S' {
            unsafe { libc::close(vw) };
            unsafe { libc::_exit(0) };
        }

        // fork the grandchild: it inherits vw and runs the test body
        let child = unsafe { libc::fork() };
        if child < 0 {
            unsafe { libc::close(vw) };
            unsafe { libc::_exit(1) };
        }
        if child == 0 {
            // grandchild: pid 1, writes verdicts directly to vw
            let probe = VerdictWriter { fd: vw };
            body(&probe);
            unsafe { libc::_exit(0) };
        }

        // middle child: wait for grandchild, then exit
        unsafe { libc::close(vw) };
        crate::proc::wait(child);
        unsafe { libc::_exit(0) };
    }

    // parent: close write end, read setup byte, then read verdict lines
    unsafe { libc::close(vw) };
    let mut byte = [0u8; 1];
    let got = unsafe { libc::read(vr, byte.as_mut_ptr() as *mut _, 1) };
    if got != 1 {
        unsafe { libc::close(vr) };
        crate::proc::wait(pid);
        return Boot::Failed("child gave no setup verdict".into());
    }
    match byte[0] {
        b'U' => {
            unsafe { libc::close(vr) };
            crate::proc::wait(pid);
            Boot::Unsupported
        }
        b'E' => {
            unsafe { libc::close(vr) };
            crate::proc::wait(pid);
            Boot::Failed("could not enter namespace".into())
        }
        b'S' => {
            // after setup, the grandchild writes verdicts to the same pipe
            let mut lines = String::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = unsafe { libc::read(vr, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 {
                    break;
                }
                lines.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            }
            unsafe { libc::close(vr) };
            crate::proc::wait(pid);

            let mut verdicts = Vec::new();
            for line in lines.lines() {
                if let Some(what) = line.strip_prefix("OK:") {
                    verdicts.push(Verdict::Pass(what.trim().to_string()));
                } else if let Some(what) = line.strip_prefix("FAIL:") {
                    verdicts.push(Verdict::Fail(what.trim().to_string()));
                }
            }
            Boot::Ran(verdicts)
        }
        _ => {
            unsafe { libc::close(vr) };
            crate::proc::wait(pid);
            Boot::Failed("garbled setup verdict".into())
        }
    }
}

pub(crate) enum NamespaceEntry {
    Unsupported,
    Other,
}

// enter a fresh mount namespace, optionally a pid namespace too. root skips
// the user namespace (already has caps); non-root creates one and maps its uid
pub(crate) fn enter_ns(add_pid: bool) -> Result<(), NamespaceEntry> {
    let root = unsafe { libc::geteuid() } == 0;

    let ns_flags = if root {
        if add_pid { libc::CLONE_NEWPID | libc::CLONE_NEWNS } else { libc::CLONE_NEWNS }
    } else {
        if add_pid { libc::CLONE_NEWUSER | libc::CLONE_NEWPID | libc::CLONE_NEWNS }
        else { libc::CLONE_NEWUSER | libc::CLONE_NEWNS }
    };

    // SAFETY: unshare(2) has no memory safety requirements. flags are valid
    if unsafe { libc::unshare(ns_flags) } != 0 {
        let e = std::io::Error::last_os_error();
        return Err(match e.raw_os_error() {
            Some(v) if v == libc::EPERM || v == libc::ENOSPC => NamespaceEntry::Unsupported,
            _ => NamespaceEntry::Other,
        });
    }

    if !root {
        // SAFETY: getuid/getgid are always safe syscalls
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        // SAFETY: getpid is always safe. the pid is used to write procfs files
        let pid = unsafe { libc::getpid() };
        // deny setgroups before writing gid_map (linux 3.19+). absent on older
        // kernels, which is fine
        match std::fs::write(format!("/proc/{pid}/setgroups"), "deny") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(NamespaceEntry::Other),
        }
        if std::fs::write(format!("/proc/{pid}/uid_map"), format!("0 {uid} 1\n")).is_err() {
            return Err(NamespaceEntry::Other);
        }
        if std::fs::write(format!("/proc/{pid}/gid_map"), format!("0 {gid} 1\n")).is_err() {
            return Err(NamespaceEntry::Other);
        }
    }

    // make the new mount ns private so mounts are contained
    // SAFETY: static NUL-terminated strings, null for fstype data and flags
    let rc = unsafe {
        libc::mount(
            c"none".as_ptr(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(NamespaceEntry::Other);
    }
    Ok(())
}

fn pipe() -> Option<(RawFd, RawFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return None;
    }
    Some((fds[0], fds[1]))
}
