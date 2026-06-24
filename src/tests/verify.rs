// the store integrity check via Core::verify_store(): an intact store
// verifies, a tampered object is caught, and a corrupt store surfaces a
// clean error at warm rather than a panic. unprivileged

use super::support::{first_object, synthetic_rootfs, write_state, LayerSpec};
use super::{Category, TestContext, Test};
use nexus::{Error, Store};

pub struct VerifyStore;

impl Test for VerifyStore {
    fn name(&self) -> &str {
        "store: integrity check"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let src = tcx.sandbox.base().join("src");
        if tcx.try_ok(synthetic_rootfs(&src, ".marker"), "synth rootfs").is_none() {
            return;
        }
        let store = Store::new(layout.store());
        let Some(tree) = tcx.try_ok(store.import(&src), "imported tree") else {
            return;
        };
        let lower = layout.store().join("void");
        if tcx.try_ok(store.checkout(&tree, &lower), "tree checked out").is_none() {
            return;
        }
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
        if tcx.try_ok(state, "wrote layer state").is_none() {
            return;
        }
        let Some(core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };

        // an intact store verifies and reports a positive object count
        match core.verify_store() {
            Ok(n) => tcx.ok(n > 0, &format!("verify_store passes ({n} objects)")),
            Err(e) => tcx.ok(false, &format!("verify_store on intact store: {e}")),
        };

        // verify_tree checks only the objects referenced by one tree  - cheaper
        // than full verify_store, used as a pre-compose gate
        match store.verify_tree(&tree) {
            Ok(n) => tcx.ok(n > 0, &format!("verify_tree passes ({n} files)")),
            Err(e) => tcx.ok(false, &format!("verify_tree on intact tree: {e}")),
        };

        // verify_tree on a bogus hash returns Corrupt
        let bogus = nexus::ObjectHash::new("f".repeat(64));
        match store.verify_tree(&bogus) {
            Err(Error::Corrupt { .. }) => tcx.ok(true, "verify_tree on bogus hash >  Corrupt"),
            other => tcx.ok(false, &format!("verify_tree bogus: expected Corrupt, got {other:?}")),
        };

        // tamper with one object: its content no longer matches its name
        let obj = first_object(&layout.store().join("objects"));
        let Some(obj) = obj else {
            tcx.ok(false, "found an object to tamper");
            return;
        };
        let _ = std::fs::write(&obj, b"CORRUPTED-CONTENT");

        // verify_store now returns Corrupt, not a panic
        match core.verify_store() {
            Err(Error::Corrupt { .. }) => tcx.ok(true, "verify_store catches tampering (Corrupt)"),
            other => tcx.ok(false, &format!("expected Corrupt, got {other:?}")),
        };

        // verify_tree on the tampered tree also catches it
        match store.verify_tree(&tree) {
            Err(Error::Corrupt { .. }) => tcx.ok(true, "verify_tree catches tampering (Corrupt)"),
            other => {
                tcx.ok(false, &format!("verify_tree tampered: expected Corrupt, got {other:?}"))
            }
        };

        // the corruption is real and silent at the store layer: a checkout
        // copies the bad bytes through without complaint (overlay would mount
        // them just the same). verify_store is the *pre-flight gate* that
        // catches this before a compose ever uses the data  - that is the
        // whole point of running it at startup
        let out = tcx.sandbox.base().join("checkout-corrupt");
        let _ = store.checkout(&tree, &out);
        let safe_to_use = core.verify_store().is_ok();
        tcx.ok(!safe_to_use, "corrupt store is gated out before compose");
    }
}
