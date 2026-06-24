// run a closure as "root" inside a fresh user + mount namespace, the way a
// rootless container runtime sets itself up. forked into a child because
// unshare(CLONE_NEWUSER) changes the thread's credential view

pub enum Setup {
    Ran(i32),
    // kernel refused a user namespace (disabled by policy): report as skip
    Unsupported,
    Failed(String),
}

pub fn run_as_root<F: FnOnce() -> i32>(body: F) -> Setup {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Setup::Failed("pipe".into());
    }
    let (rd, wr) = (fds[0], fds[1]);

    // safe: spark forks single-threaded, so the child below may allocate
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(rd);
            libc::close(wr);
        }
        return Setup::Failed("fork".into());
    }

        if pid == 0 {
            unsafe { libc::close(rd) };
            let verdict = match crate::init::enter_ns(false) {
                Ok(()) => b'S',
                Err(crate::init::NamespaceEntry::Unsupported) => b'U',
                Err(crate::init::NamespaceEntry::Other) => b'E',
            };
        unsafe {
            libc::write(wr, [verdict].as_ptr() as *const _, 1);
            libc::close(wr);
        }
        if verdict != b'S' {
            unsafe { libc::_exit(0) };
        }
        let code = body();
        unsafe { libc::_exit(code & 0xff) };
    }

    unsafe { libc::close(wr) };
    let mut byte = [0u8; 1];
    let got = unsafe { libc::read(rd, byte.as_mut_ptr() as *mut _, 1) };
    unsafe { libc::close(rd) };

    let status = crate::proc::wait(pid);

    if got != 1 {
        return Setup::Failed("child gave no setup verdict".into());
    }
    match byte[0] {
        b'U' => Setup::Unsupported,
        b'E' => Setup::Failed("could not enter user namespace".into()),
        b'S' if libc::WIFEXITED(status) => Setup::Ran(libc::WEXITSTATUS(status)),
        b'S' => Setup::Failed("payload terminated by signal".into()),
        _ => Setup::Failed("garbled setup verdict".into()),
    }
}
