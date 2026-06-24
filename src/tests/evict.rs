// the registry's eviction: warm a layer (pins its namespace), evict it (the
// pin is dropped, ns file gone), then warm again to confirm it rebuilds
// also exercises evict_idle. needs root to compose

use super::support::{import_layer, synthetic_rootfs, write_state, LayerSpec};
use super::{Category, TestContext, Test};

pub struct EvictRebuild;

impl Test for EvictRebuild {
    fn name(&self) -> &str {
        "namespace: evict, rebuild & idle-evict"
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
        if import_layer(&layout, "void", &src).is_err() {
            tcx.ok(false, "import layer");
            return;
        }
        // idle_evict_secs = 0 makes an idle sweep evict immediately, so the
        // evict_idle step is deterministic without sleeping
        let state = write_state(
            &layout,
            "overlay",
            &[LayerSpec {
                id: "void",
                class: "shadowed",
                priority: 1,
                libc: "static",
                loader: None,
                extra: "ephemeral = true\n",
            }],
        );
        if tcx.try_ok(state, "wrote layer state").is_none() {
            return;
        }
        // append the idle setting to nexus.toml
        let nexus_toml = layout.state().join("nexus.toml");
        let idle = std::fs::write(&nexus_toml, "backend = \"overlay\"\nidle_evict_secs = 0\n");
        if tcx.try_ok(idle, "wrote idle_evict setting").is_none() {
            return;
        }

        let Some(mut core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };
        let ns = layout.ns_file("void");

        // first build composes and pins the namespace
        if tcx.try_ok(core.build("void"), "build builds the layer").is_none() {
            return;
        }
        tcx.ok(ns.exists(), "ns file present after build");
        tcx.ok(core.built_count() == 1, "registry holds one built layer");

        // explicit evict drops the pin and unmounts it
        core.evict("void");
        tcx.ok(core.built_count() == 0, "registry empty after evict");

        // rebuild: warm again must recompose and re-pin
        if tcx.try_ok(core.build("void"), "build rebuilds after evict").is_none() {
            return;
        }
        tcx.ok(ns.exists(), "ns file present after rebuild");
        tcx.ok(core.built_count() == 1, "registry holds the rebuilt layer");

        // an idle sweep with idle_evict=0 evicts the (idle) layer
        let n = core.evict_idle();
        tcx.ok(n == 1, &format!("evict_idle reclaimed the idle layer ({n})"));
        tcx.ok(core.built_count() == 0, "registry empty after idle evict");

        // and it can be warmed once more, proving evict left a clean slate
        tcx.ok(core.build("void").is_ok(), "build works again after idle evict");
    }
}
