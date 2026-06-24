// drive nexus as an ordinary user: spark builds its own user+mount namespace
// (no sudo) and composes a layer inside it, the way a rootless runtime does
// the unprivileged steps (fetch, import, checkout, state) run in the parent;
// compose + run happen in the userns child, whose exit code reports which
// step held

use super::support::write_shadow_state;
use super::userns::{run_as_root, Setup};
use super::{Category, TestContext, Libc, Test};
use crate::fetch::{self, Source};

pub struct RootlessGlibc;
pub struct RootlessMusl;

impl Test for RootlessGlibc {
    fn name(&self) -> &str {
        "rootless: glibc"
    }
    fn category(&self) -> Category {
        Category::Rootless
    }
    fn libc(&self) -> Option<Libc> {
        Some(Libc::Glibc)
    }
    fn run(&self, tcx: &mut TestContext) {
        let src = fetch::void_glibc().expect("fetch void glibc source");
        rootless(tcx, &src, "glibc", "/lib64/ld-linux-x86-64.so.2");
    }
}

impl Test for RootlessMusl {
    fn name(&self) -> &str {
        "rootless: musl"
    }
    fn category(&self) -> Category {
        Category::Rootless
    }
    fn libc(&self) -> Option<Libc> {
        Some(Libc::Musl)
    }
    fn run(&self, tcx: &mut TestContext) {
        let src = fetch::void_musl().expect("fetch void musl source");
        rootless(tcx, &src, "musl", "/lib/ld-musl-x86_64.so.1");
    }
}

const MARKER: &str = ".spark-rootless-root";

// step codes the child returns, mapped back to assertions in the parent
const OK: i32 = 0;
const E_WARM: i32 = 2;
const E_NSFILE: i32 = 3;
const E_RUN: i32 = 4;
const E_ROOT: i32 = 5;

fn rootless(tcx: &mut TestContext, source: &Source, libc_name: &str, loader: &str) {
    tcx.scope(&format!("rootless/{libc_name}"));
    let layout = tcx.sandbox.layout().clone();
    let layer = "void";

    tcx.msg(&format!("fetch {}", source.name));
    let Some(rootfs) = tcx.try_ok(crate::fetch::fetch(source), "rootfs fetched") else {
        return;
    };

    let store = nexus::Store::new(layout.store());
    let Some(tree) = tcx.try_ok(store.import(&rootfs), "rootfs imported") else {
        return;
    };
    let lower = layout.store().join(layer);
    if tcx.try_ok(store.checkout(&tree, &lower), "tree checked out").is_none() {
        return;
    }
    let _ = std::fs::write(lower.join(MARKER), b"rootless");

    if write_shadow_state(&layout, layer, libc_name, loader).is_err() {
        tcx.ok(false, "state written");
        return;
    }
    tcx.ok(true, "state written");

    let layers = match nexus::load_layers(&layout.state().join("layers.toml")) {
        Ok(l) => l,
        Err(e) => {
            tcx.ok(false, &format!("load layers: {e}"));
            return;
        }
    };
    let system = match nexus::load_system(&layout.state().join("nexus.toml")) {
        Ok(s) => s,
        Err(e) => {
            tcx.ok(false, &format!("load system: {e}"));
            return;
        }
    };

    // everything from here composes mounts, so it runs in the userns child
    let ns_file = layout.ns_file(layer);
    let setup = run_as_root(move || {
        let mut core = nexus::Core::open(layout.clone(), layers, system);
        if core.build(layer).is_err() {
            return E_WARM;
        }
        if !ns_file.exists() {
            return E_NSFILE;
        }
        let id = nexus::LayerId::new(layer).expect("valid layer id");
        if !exec_ok(&mut core, &id, &["/bin/true".into()]) {
            return E_RUN;
        }
        let marker = format!("/{MARKER}");
        if !exec_ok(&mut core, &id, &["/usr/bin/test".into(), "-e".into(), marker]) {
            return E_ROOT;
        }
        OK
    });

    match setup {
        Setup::Unsupported => {
            tcx.wrn("unprivileged user namespaces are disabled; skipping");
        }
        Setup::Failed(why) => {
            tcx.ok(false, &format!("userns setup: {why}"));
        }
        Setup::Ran(code) => {
            tcx.ok(code != E_WARM, "composed layer in user namespace");
            tcx.ok(code != E_WARM && code != E_NSFILE, "namespace pinned (rootless)");
            tcx.ok(!matches!(code, E_WARM | E_NSFILE | E_RUN), "ran /bin/true rootless");
            tcx.ok(code == OK, "process rooted in layer (marker at /)");
        }
    }
}

// fork a child that enters the layer and execs; true iff it exits 0
fn exec_ok(core: &mut nexus::Core, id: &nexus::LayerId, argv: &[String]) -> bool {
    // safe: spark forks single-threaded; the child only execs or _exits
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        match core.run(id, argv, &[]) {
            Err(_) => {}
        }
        unsafe { libc::_exit(127) };
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
}
