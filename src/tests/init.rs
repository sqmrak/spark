// nexus's pid-1 path, booted for real. the boot harness (crate::init) runs the guest
// as pid 1 of a fresh pid namespace, where the kernel applies the genuine init
// semantics a plain mount-namespace test cannot reach

use super::support::{import_layer, open_core, synthetic_rootfs, write_state, LayerSpec};

// microseconds an orphan is left alive before exiting: enough for the parent to
// quit and the kernel to set ppid=1, short enough that the test does not stall
const ORPHAN_LINGER_US: u32 = 80_000;
// microseconds the reaper is given to detect and sweep the orphan
const REAP_WAIT_US: u32 = 300_000;
// microseconds the burst orphans hang around before the short reaper drain
const BURST_LINGER_US: u32 = 50_000;
use super::{Category, TestContext, Test};
use crate::init::{self, Boot, Verdict, VerdictWriter};
use nexus::Layout;
use std::path::Path;

// real root is needed for the mount-heavy init tests: devtmpfs will not mount
// in a user namespace, and the inherited pseudo-mounts cannot be moved there
fn is_real_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

// relay a guest transcript into the test's tally
fn relay(tcx: &mut TestContext, boot: Boot, skip_note: &str) {
    match boot {
        Boot::Unsupported => tcx.wrn(skip_note),
        Boot::Failed(e) => {
            tcx.ok(false, &format!("boot harness: {e}"));
        }
        Boot::Ran(marks) => {
            for m in marks {
                match m {
                    Verdict::Pass(s) => {
                        tcx.ok(true, &s);
                    }
                    Verdict::Fail(s) => {
                        tcx.ok(false, &s);
                    }
                }
            }
        }
    }
}

// reaping orphans is pid 1's defining job, and only inside a pid namespace is
// the guest really pid 1, so only here can nexus's Reaper face a real orphan
pub struct ReapOrphans;

impl Test for ReapOrphans {
    fn name(&self) -> &str {
        "init: reaps an orphan"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        let boot = init::boot(reap_orphans_guest);
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn reap_orphans_guest(p: &VerdictWriter) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");

    let Some(reaper) = spawn_reaper(p) else {
        return;
    };
    spawn_orphan(ORPHAN_LINGER_US);

    // give the grandchild time to be orphaned, die, and be swept
    // safe: a plain sleep
    unsafe { libc::usleep(REAP_WAIT_US) };
    reaper.stop();
    p.ok(no_children_left(), "no unreaped children remain (the orphan was collected)");
}

// spawn the reaper, recording the outcome; None when the os refused the
// thread, so the caller stops rather than asserting against a dead reaper
fn spawn_reaper(p: &VerdictWriter) -> Option<nexus::Reaper> {
    match nexus::Reaper::spawn() {
        Ok(r) => Some(r),
        Err(e) => {
            p.ok(false, &format!("reaper spawned: {e}"));
            None
        }
    }
}

// fork a child that exits at once after forking a grandchild, so the
// grandchild outlives its parent and reparents to pid 1. the child only forks,
// sleeps and _exits (async-signal-safe), so it is safe even with the reaper
// thread already running
fn spawn_orphan(linger_us: u32) {
    // safe: both children only sleep and _exit, async-signal-safe even with
    // the reaper thread running
    let child = unsafe { libc::fork() };
    if child == 0 {
        let grandchild = unsafe { libc::fork() };
        if grandchild == 0 {
            unsafe { libc::usleep(linger_us) };
            unsafe { libc::_exit(0) };
        }
        unsafe { libc::_exit(0) };
    }
}

// true when no child is left: a survivor would be returned by a non-blocking
// reap instead of the -1/ECHILD we expect here
fn no_children_left() -> bool {
    crate::proc::try_reap() == -1
}

// pid 1 must field a crowd of orphans, not just one. a burst exercises the
// reaper's sweep loop under load while the guest is the namespace's pid 1
pub struct ReaperBurst;

impl Test for ReaperBurst {
    fn name(&self) -> &str {
        "init: reaps an orphan burst"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        let boot = init::boot(reaper_burst_guest);
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn reaper_burst_guest(p: &VerdictWriter) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");

    let Some(reaper) = spawn_reaper(p) else {
        return;
    };
    for _ in 0..16 {
        spawn_orphan(BURST_LINGER_US);
    }

    // safe: a plain sleep
    unsafe { libc::usleep(REAP_WAIT_US) };
    reaper.stop();
    p.ok(no_children_left(), "all 16 orphans were collected");
}

// the boot harness really hands the guest a fresh, isolated pid namespace: it is pid
// 1, has no visible parent, and a fork lands in a low pid space restarted at 1
pub struct Isolation;

impl Test for Isolation {
    fn name(&self) -> &str {
        "init: fresh pid namespace"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        let boot = init::boot(isolation_guest);
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn isolation_guest(p: &VerdictWriter) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");
    // pid 1's parent lives in the outer namespace, so it is invisible here
    p.ok(unsafe { libc::getppid() } == 0, "pid 1 has no visible parent");

    // a fresh pid space restarts numbering, so the first child is a low pid;
    // on the host it would be in the thousands
    // safe: the child only _exits
    let child = unsafe { libc::fork() };
    if child == 0 {
        unsafe { libc::_exit(0) };
    }
    p.ok(child > 1 && child < 100, "a forked child gets a low pid (fresh pid space)");
    crate::proc::wait(child);
}

// nexus masks the fatal signals up front and keeps SIGCHLD so pid 1 survives
// boot and can still reap. as the real pid 1, a SIGTERM is the honest test
pub struct SignalDiscipline;

impl Test for SignalDiscipline {
    fn name(&self) -> &str {
        "init: signal discipline"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        let boot = init::boot(signal_discipline_guest);
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn signal_discipline_guest(p: &VerdictWriter) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");
    p.ok(nexus::block_signals().is_ok(), "block_signals applied");

    let masked = is_blocked(libc::SIGTERM) && is_blocked(libc::SIGINT) && is_blocked(libc::SIGHUP);
    p.ok(masked, "fatal signals masked");
    p.ok(!is_blocked(libc::SIGCHLD), "SIGCHLD kept unblocked so pid 1 can reap");

    // blocked, a self-SIGTERM stays pending instead of killing us; reaching
    // the next line proves pid 1 stayed up
    // safe: raising a signal at ourselves
    unsafe { libc::raise(libc::SIGTERM) };
    p.ok(true, "pid 1 survives a SIGTERM while it is masked");
}

// whether `sig` is in the calling thread's current block mask
fn is_blocked(sig: i32) -> bool {
    let mut cur: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe { libc::sigprocmask(libc::SIG_BLOCK, std::ptr::null(), &mut cur) };
    unsafe { libc::sigismember(&cur, sig) == 1 }
}

// the first thing nexus does as pid 1: bring up /proc, /sys, /dev and a
// runtime tmpfs. devtmpfs makes this real-root only, which is why it lives
// behind the gate rather than in the rootless set
pub struct EarlyMounts;

impl Test for EarlyMounts {
    fn name(&self) -> &str {
        "init: early mounts"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        if !is_real_root() {
            tcx.wrn("needs real root (devtmpfs), skipping early_mounts");
            return;
        }
        let layout = tcx.sandbox.layout().clone();
        let boot = init::boot(move |p| early_mounts_guest(p, &layout));
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn early_mounts_guest(p: &VerdictWriter, layout: &Layout) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");
    if let Err(e) = nexus::early_mounts(layout, &nexus::System::default().pseudo) {
        p.ok(false, &format!("early_mounts failed: {e}"));
        return;
    }
    p.ok(is_mounted("/proc"), "/proc mounted");
    p.ok(is_mounted("/sys"), "/sys mounted");
    p.ok(is_mounted("/dev"), "/dev mounted (devtmpfs)");
    p.ok(is_mounted(&layout.run().display().to_string()), "runtime tmpfs mounted");
}

// switch_root is the pivot off the initramfs: carry the pseudo-mounts over,
// make a new tree the root, drop the old one. exercised on a throwaway tmpfs
// root so nothing on the host is touched. real-root only (it moves mounts)
pub struct SwitchRoot;

impl Test for SwitchRoot {
    fn name(&self) -> &str {
        "init: switch root"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        if !is_real_root() {
            tcx.wrn("needs real root (mount moves), skipping switch_root");
            return;
        }
        let layout = tcx.sandbox.layout().clone();
        let boot = init::boot(move |p| switch_root_guest(p, &layout));
        relay(tcx, boot, "user namespaces unavailable, skipping pid-1 boot");
    }
}

fn switch_root_guest(p: &VerdictWriter, layout: &Layout) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");
    // switch_root carries the live pseudo-mounts across, so bring them up first
    if let Err(e) = nexus::early_mounts(layout, &nexus::System::default().pseudo) {
        p.ok(false, &format!("early_mounts failed: {e}"));
        return;
    }

    let new_root = layout.root().join("newroot");
    let _ = std::fs::create_dir_all(&new_root);
    if !mount_tmpfs(&new_root) {
        p.ok(false, "could not mount the new-root tmpfs");
        return;
    }
    // a marker placed in the new root; after the pivot it must sit at /
    let _ = std::fs::write(new_root.join(".spark-newroot"), b"spark");

    // switch_root ends in chroot("."), so the caller must already stand in the
    // new root, the classic `cd newroot; mount --move . /; chroot .` idiom
    if std::env::set_current_dir(&new_root).is_err() {
        p.ok(false, "chdir into the new root");
        return;
    }

    match nexus::switch_root(&new_root.display().to_string(), &nexus::System::default().pseudo) {
        Ok(()) => p.ok(true, "switch_root pivoted onto the new root"),
        Err(e) => {
            p.ok(false, &format!("switch_root failed: {e}"));
            return;
        }
    };
    p.ok(Path::new("/.spark-newroot").exists(), "landed in the new root (marker at /)");
    p.ok(Path::new("/proc/self").exists(), "/proc carried into the new root");
}

// a freestanding init that just calls exit(0). the synthetic layer carries no
// libc, so a dynamic /bin/true would exec-fail on its missing loader; a static
// blob runs with nothing underneath, matching the layer's loader:None
#[cfg(target_arch = "x86_64")]
const STATIC_INIT: &[u8] = &[
    0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x31, 0xff, 0xb8, 0xe7, 0x00, 0x00, 0x00, 0x0f,
    0x05,
];
#[cfg(target_arch = "aarch64")]
const STATIC_INIT: &[u8] = &[
    0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x00, 0xb7, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xd2, 0xa8, 0x0b, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4,
];

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn write_static_init(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, STATIC_INIT)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

// run() skips unsupported arches before boot_guest is ever reached; this keeps
// it compiling there
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn write_static_init(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("no static init blob for this arch"))
}

// the whole init path end to end: Core::boot as pid 1 runs block_signals,
// the reaper, early_mounts, layer selection and compose, the cgroup scope,
// then handoff exec's the native init. real-root only (compose + devtmpfs)
pub struct BootToHandoff;

impl Test for BootToHandoff {
    fn name(&self) -> &str {
        "init: full boot"
    }

    fn category(&self) -> Category {
        Category::Init
    }

    fn run(&self, tcx: &mut TestContext) {
        if !is_real_root() {
            tcx.wrn("needs real root (compose + devtmpfs), skipping full boot");
            return;
        }
        if !cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
            tcx.wrn("no static init blob for this arch, skipping full boot");
            return;
        }
        let layout = tcx.sandbox.layout().clone();
        match init::boot(move |p| boot_guest(p, &layout)) {
            Boot::Unsupported => tcx.wrn("user namespaces unavailable, skipping pid-1 boot"),
            Boot::Failed(e) => {
                tcx.ok(false, &format!("boot harness: {e}"));
            }
            // boot only returns on failure; a clean run exec's native init and
            // leaves no marks, so an empty transcript is the success signal
            Boot::Ran(marks) if marks.is_empty() => {
                tcx.ok(true, "Core::boot ran the full path and handed off to native init");
            }
            Boot::Ran(marks) => {
                for m in marks {
                    match m {
                        Verdict::Pass(s) => {
                            tcx.ok(true, &s);
                        }
                        Verdict::Fail(s) => {
                            tcx.ok(false, &s);
                        }
                    }
                }
            }
        }
    }
}

fn boot_guest(p: &VerdictWriter, layout: &Layout) {
    p.ok(unsafe { libc::getpid() } == 1, "guest is pid 1");

    // a minimal synthetic layer whose /init is a freestanding static binary
    // standing in for the native init. a static layer carries no loader, so it
    // is trivially healthy and selection picks it without a network fetch
    let src = layout.root().join("src");
    if let Err(e) = synthetic_rootfs(&src, ".marker") {
        p.ok(false, &format!("synth rootfs: {e}"));
        return;
    }
    if let Err(e) = write_static_init(&src.join("init")) {
        p.ok(false, &format!("write static init: {e}"));
        return;
    }
    if let Err(e) = import_layer(layout, "base", &src) {
        p.ok(false, &format!("import layer: {e}"));
        return;
    }
    let spec = LayerSpec {
        id: "base",
        class: "shadowed",
        priority: 1,
        libc: "static",
        loader: None,
        extra: "",
    };
    if let Err(e) = write_state(layout, "overlay", &[spec]) {
        p.ok(false, &format!("write state: {e}"));
        return;
    }
    let mut core = match open_core(layout) {
        Ok(c) => c,
        Err(e) => {
            p.ok(false, &format!("open core: {e}"));
            return;
        }
    };

    // on success boot exec's /init and never returns here; any return is a
    // failure, so reaching past this point always records one
    match core.boot("/init") {
        Ok(()) => {
            p.ok(false, "boot returned without handing off");
        }
        Err(e) => {
            p.ok(false, &format!("boot: {e}"));
        }
    }
}

// whether `target` is a mount point, per /proc/self/mountinfo field 5
fn is_mounted(target: &str) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|s| s.lines().any(|l| l.split_whitespace().nth(4) == Some(target)))
        .unwrap_or(false)
}

// mount a fresh tmpfs at `at`, so it can serve as a pivot target
fn mount_tmpfs(at: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(at.as_os_str().as_bytes()) else {
        return false;
    };
    let tmpfs = c"tmpfs";
    // safe: mounting a fresh tmpfs at our own path, valid c-strings
    unsafe { libc::mount(tmpfs.as_ptr(), path.as_ptr(), tmpfs.as_ptr(), 0, std::ptr::null()) == 0 }
}
