// resource limits via cgroup v2: configure pids.max on a layer, compose
// it, verify compose succeeds. actual limits apply at run time

use super::support::synthetic_rootfs;
use super::{Category, TestContext, Test};

pub struct Cgroups;

impl Test for Cgroups {
    fn name(&self) -> &str {
        "namespace: cgroup resource limits"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        if !nexus::cgroup_usable() {
            tcx.wrn("cgroup v2 not available; skipping");
            return;
        }
        let layout = tcx.sandbox.layout().clone();
        let layer = "void";

        let src = tcx.sandbox.base().join("src");
        if tcx.try_ok(synthetic_rootfs(&src, ".marker"), "synth rootfs").is_none() {
            return;
        }

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
             [layer.void.libc]\n\
             name = \"static\"\n\
             [layer.void.resources]\n\
             pids_max = 64\n",
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
        let mut core = nexus::Core::open(layout, layers, system);
        match core.build(layer) {
            Ok(()) => tcx.ok(true, "layer composed with pids.max=64"),
            Err(e) => tcx.ok(false, &format!("build with pids.max: {e}")),
        };
    }
}
