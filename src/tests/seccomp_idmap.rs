// seccomp baseline + idmapped mount composed in one layer. the two sandbox
// features must not conflict; a successful warm proves it

use super::support::synthetic_rootfs;
use super::{Category, TestContext, Test};

pub struct SeccompIdmap;

impl Test for SeccompIdmap {
    fn name(&self) -> &str {
        "namespace: seccomp and idmap together"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        if !nexus::idmap_usable() {
            tcx.wrn("idmapped mounts not supported by kernel; skipping");
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
             [layer.void.sandbox]\n\
             idmap = { outside = 100000, count = 65536 }\n\
             seccomp = \"baseline\"\n",
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
            Ok(()) => tcx.ok(true, "layer composed with seccomp+idmap"),
            Err(e) => tcx.ok(false, &format!("build with seccomp+idmap: {e}")),
        };
    }
}
