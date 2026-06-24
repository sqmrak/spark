// garbage collection against the store through Core::gc. a gen protects
// the trees it references; orphaned trees are swept along with their objects

use super::support::{synthetic_rootfs, write_state, LayerSpec};
use super::{Category, TestContext, Test};
use nexus::Store;

pub struct GarbageCollect;

impl Test for GarbageCollect {
    fn name(&self) -> &str {
        "store: garbage collect"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let state = write_state(
            &layout,
            "overlay",
            &[LayerSpec {
                id: "void",
                class: "shadowed",
                priority: 1,
                libc: "static",
                loader: None,
                extra: "",
            }],
        );
        if tcx.try_ok(state, "wrote state").is_none() {
            return;
        }
        let Some(core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };

        let a = tcx.sandbox.base().join("a");
        let b = tcx.sandbox.base().join("b");
        if tcx.try_ok(synthetic_rootfs(&a, ".marker-a"), "synth rootfs a").is_none()
            || tcx.try_ok(synthetic_rootfs(&b, ".marker-b"), "synth rootfs b").is_none()
        {
            return;
        }
        let store = Store::new(layout.store());
        let Some(ha) = tcx.try_ok(store.import(&a), "imported tree a") else {
            return;
        };
        let Some(hb) = tcx.try_ok(store.import(&b), "imported tree b") else {
            return;
        };

        tcx.exists(&store.tree_path(&ha), "tree a on disk");
        tcx.exists(&store.tree_path(&hb), "tree b on disk");
        let objects_before = count_objects(&layout.store().join("objects"));
        tcx.ok(objects_before > 0, &format!("objects before gc ({objects_before})"));

        let g1 = match core.commit(std::slice::from_ref(&ha)) {
            Ok(g) => {
                tcx.ok(true, &format!("committed gen {g}"));
                g
            }
            Err(e) => {
                tcx.ok(false, &format!("commit: {e}"));
                return;
            }
        };
        if tcx.try_ok(core.activate_gen(g1), &format!("activated gen {g1}")).is_none() {
            return;
        }

        // only tree a is committed; tree b is orphaned and must be swept
        if tcx.try_ok(core.gc(), "gc swept orphaned objects").is_none() {
            return;
        }

        tcx.exists(&store.tree_path(&ha), "tree a survives gc");
        tcx.ok(!store.tree_path(&hb).exists(), "orphaned tree b removed by gc");
        let objects_after = count_objects(&layout.store().join("objects"));
        tcx.ok(
            objects_after <= objects_before,
            &format!("objects dropped: {objects_before} >  {objects_after}"),
        );
        match core.verify_store() {
            Ok(n) => tcx.ok(n > 0, "store verifies after gc"),
            Err(e) => tcx.ok(false, &format!("verify_store after gc: {e}")),
        };

        // commit gen 2 with both trees; gc must sweep nothing
        let Some(hb2) = tcx.try_ok(store.import(&b), "re-imported tree b") else {
            return;
        };
        tcx.ok(hb2 == hb, "re-import yields same hash");
        let g2 = match core.commit(&[ha, hb]) {
            Ok(g) => {
                tcx.ok(true, &format!("committed gen {g}"));
                g
            }
            Err(e) => {
                tcx.ok(false, &format!("commit gen 2: {e}"));
                return;
            }
        };
        if tcx.try_ok(core.activate_gen(g2), &format!("activated gen {g2}")).is_none() {
            return;
        }
        tcx.try_ok(core.gc(), "gc after second commit (nothing orphaned)");
    }
}

fn count_objects(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}
