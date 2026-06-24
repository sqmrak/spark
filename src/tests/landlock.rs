// landlock filesystem sandbox: compose a layer with read/write path
// rules, verify the sandbox config parses and compose succeeds

use super::support::synthetic_rootfs;
use super::{Category, TestContext, Test};

pub struct Landlock;

impl Test for Landlock {
    fn name(&self) -> &str {
        "namespace: landlock sandbox"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
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
             read = [\"/bin\", \"/usr\"]\n\
             write = [\"/tmp\"]\n",
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
            Ok(()) => tcx.ok(true, "layer composed with landlock sandbox"),
            Err(e) => tcx.ok(false, &format!("build with landlock: {e}")),
        };
    }
}
