// embedded freestanding ELF calls mount(2); baseline seccomp must trap it
// with EPERM. run with and without seccomp to prove the filter is the cause

use super::support::{import_layer, synthetic_rootfs, write_state, LayerSpec};
use super::{run_in_layer, Category, TestContext, Test};

// the helper's exit codes (must match assets/seccomp_probe.c)
const ALLOWED: i32 = 0;
const EPERM_TRAP: i32 = 11;

// embedded freestanding ELF; runs in any layer regardless of libc
static PROBE: &[u8] = include_bytes!("../../assets/seccomp_probe");

pub struct Seccomp;

impl Test for Seccomp {
    fn name(&self) -> &str {
        "namespace: seccomp filter"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let src = tcx.sandbox.base().join("src");
        if tcx.try_ok(synthetic_rootfs(&src, ".marker"), "synth rootfs").is_none() {
            return;
        }
        // drop the probe into the layer tree as an executable
        let probe_rel = "usr/bin/seccomp-probe";
        let probe_abs = src.join(probe_rel);
        if std::fs::write(&probe_abs, PROBE).is_err() {
            tcx.ok(false, "write probe into rootfs");
            return;
        }
        if tcx.try_ok(set_exec(&probe_abs), "marked probe executable").is_none() {
            return;
        }

        if import_layer(&layout, "void", &src).is_err() {
            tcx.ok(false, "import layer");
            return;
        }

        // first: no seccomp. the probe's mount(2) is not trapped, so it fails
        // for a different reason (not EPERM-from-seccomp)  - exit != 11
        if tcx.try_ok(write_layer(&layout, false), "wrote state (no seccomp)").is_none() {
            return;
        }
        let Some(mut core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };
        match run_in_layer(&mut core, "void", &[&abs(probe_rel)]) {
            Ok(code) => tcx.ok(
                code != EPERM_TRAP,
                &format!("without seccomp the syscall is not trapped (exit {code})"),
            ),
            Err(e) => tcx.ok(false, &format!("run without seccomp: {e}")),
        };

        // second: seccomp = baseline. mount(2) is denied with EPERM, so the
        // probe reports the trap  - exit 11
        if tcx.try_ok(write_layer(&layout, true), "wrote state (seccomp)").is_none() {
            return;
        }
        let Some(mut core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };
        match run_in_layer(&mut core, "void", &[&abs(probe_rel)]) {
            Ok(code) if code == EPERM_TRAP => {
                tcx.ok(true, "seccomp baseline traps mount(2) with EPERM")
            }
            Ok(code) if code == ALLOWED => {
                tcx.ok(false, "seccomp baseline did NOT trap the syscall (allowed)")
            }
            Ok(code) => tcx.ok(false, &format!("unexpected probe exit under seccomp: {code}")),
            Err(e) => tcx.ok(false, &format!("run under seccomp: {e}")),
        };
    }
}

fn abs(rel: &str) -> String {
    format!("/{rel}")
}

fn set_exec(p: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod {p:?}: {e}"))
}

// write state for the void layer, with or without the seccomp baseline
fn write_layer(layout: &nexus::Layout, seccomp: bool) -> std::io::Result<()> {
    let extra = if seccomp {
        "ephemeral = true\n[layer.void.sandbox]\nseccomp = \"baseline\"\n"
    } else {
        "ephemeral = true\n"
    };
    write_state(
        layout,
        "overlay",
        &[LayerSpec {
            id: "void",
            class: "shadowed",
            priority: 1,
            libc: "static",
            loader: None,
            extra,
        }],
    )
}
