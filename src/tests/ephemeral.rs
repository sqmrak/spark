// an ephemeral layer gets a writable in-memory (tmpfs) upper, not a
// read-only root. compose one, enter its namespace, and confirm a write at
// / succeeds and reads back  - and that nothing landed on the host store

use super::{Category, TestContext, Test};
use std::os::fd::AsRawFd;

pub struct Ephemeral;

impl Test for Ephemeral {
    fn name(&self) -> &str {
        "namespace: ephemeral upper is writable"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let layer = "void";

        // a minimal synthetic rootfs is enough: we never exec, only setns
        // and write at /
        let src = tcx.sandbox.base().join("src");
        let _ = std::fs::create_dir_all(src.join("usr/bin"));
        let _ = std::fs::write(src.join("ro-file"), "lower");

        let store = nexus::Store::new(layout.store());
        let Some(tree) = tcx.try_ok(store.import(&src), "rootfs imported") else {
            return;
        };
        let lower = layout.store().join(layer);
        if tcx.try_ok(store.checkout(&tree, &lower), "tree checked out").is_none() {
            return;
        }

        let dir = layout.state();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("layers.toml"),
            "[layer.void]\n\
             type = \"shadowed\"\n\
             priority = 1\n\
             ephemeral = true\n\
             [layer.void.libc]\n\
             name = \"static\"\n",
        );
        let _ = std::fs::write(dir.join("nexus.toml"), "backend = \"overlay\"\n");
        let layers = match nexus::load_layers(&dir.join("layers.toml")) {
            Ok(l) => l,
            Err(e) => {
                tcx.ok(false, &format!("load layers: {e}"));
                return;
            }
        };
        let system = match nexus::load_system(&dir.join("nexus.toml")) {
            Ok(s) => s,
            Err(e) => {
                tcx.ok(false, &format!("load system: {e}"));
                return;
            }
        };
        let mut core = nexus::Core::open(layout.clone(), layers, system);
        if tcx.try_ok(core.build(layer), "ephemeral layer composed").is_none() {
            return;
        }

        // child: enter the pinned namespace and write at /
        let ns = layout.ns_file(layer);
        let wrote = match fork_write(&ns) {
            Ok(code) => code == 0,
            Err(e) => {
                tcx.ok(false, &format!("probe: {e}"));
                return;
            }
        };
        tcx.ok(wrote, "write at / succeeds (upper is writable)");

        // the write went to the tmpfs upper, not the on-disk lower
        tcx.ok(!lower.join("probe").exists(), "write did not touch the host store");
    }
}

// fork a child that setns into the mount namespace at `ns` and writes a file
// at /. returns the child's exit code (0 = wrote and read back)
fn fork_write(ns: &std::path::Path) -> Result<i32, String> {
    // safe: spark forks single-threaded, so the child below may allocate
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("fork".into());
    }
    if pid == 0 {
        let code = match std::fs::File::open(ns) {
            Ok(f) => {
                let r = unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWNS) };
                if r != 0 {
                    3
                } else if std::fs::write("/probe", b"x").is_ok()
                    && std::fs::read("/probe").map(|b| b == b"x").unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            Err(_) => 4,
        };
        unsafe { libc::_exit(code) };
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    if libc::WIFEXITED(status) {
        Ok(libc::WEXITSTATUS(status))
    } else {
        Err("child signalled".into())
    }
}
