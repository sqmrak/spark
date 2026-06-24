// drive nexus layer selection: the default selector picks the healthy
// layer of lowest priority, skipping one whose loader is missing. health
// resolves the loader *inside the layer tree* (store/<id>/<loader>), so
// spark plants the loader there, not on the host. the libc name is
// irrelevant to health, so the test uses a neutral one

use super::{Category, TestContext, Test};
use nexus::{DefaultHealthCheck, DefaultSelector, Error, HealthCheck, LayerSelector};
use std::fs;

pub struct HealthySelection;

impl Test for HealthySelection {
    fn name(&self) -> &str {
        "store: select healthy layer"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let store = layout.store();
        let loader = "/lib/ld.so";
        let rel = loader.trim_start_matches('/');

        // the "good" layer's tree carries the loader; the "broken" one does
        // not, so it resolves to nothing inside its own tree
        let good_loader = store.join("good").join(rel);
        let Some(good_dir) = good_loader.parent() else {
            tcx.ok(false, "loader path has a parent");
            return;
        };
        let planted = fs::create_dir_all(good_dir)
            .and_then(|_| fs::write(&good_loader, "loader"))
            .and_then(|_| fs::create_dir_all(store.join("broken")));
        if tcx.try_ok(planted, "planted layer trees").is_none() {
            return;
        }

        // lowest priority is broken (loader absent in its tree); the next is
        // healthy. the selector must skip the broken one despite its rank
        let toml = format!(
            "[layer.broken]\n\
             type = \"native\"\n\
             priority = 1\n\
             [layer.broken.libc]\n\
             name = \"testlibc\"\n\
             loader = \"{loader}\"\n\
             \n\
             [layer.good]\n\
             type = \"native\"\n\
             priority = 2\n\
             [layer.good.libc]\n\
             name = \"testlibc\"\n\
             loader = \"{loader}\"\n"
        );
        let dir = layout.state();
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("layers.toml");
        if fs::write(&path, toml).is_err() {
            tcx.ok(false, "write layers.toml");
            return;
        }

        let Some(layers) = tcx.try_ok(nexus::load_layers(&path), "layers parsed") else {
            return;
        };
        tcx.ok(layers.len() == 2, "both layers parsed");

        match DefaultSelector.select(&layers, &DefaultHealthCheck, &store) {
            Ok(id) => {
                tcx.ok(id.as_str() == "good", "skips unhealthy, picks healthy");
            }
            Err(e) => {
                tcx.ok(false, &format!("select: {e}"));
            }
        }

        // the broken layer is independently unhealthy: its loader is absent
        // from its own tree
        let Some(broken) = layers.iter().find(|l| l.id.as_str() == "broken") else {
            tcx.ok(false, "broken layer present in parsed toml");
            return;
        };
        tcx.ok(
            !DefaultHealthCheck.is_healthy(broken, &store.join("broken")),
            "broken layer reports unhealthy",
        );
    }
}

// when every candidate is unhealthy (loader absent from every layer tree),
// the selector returns Error::NoHealthyLayer rather than panicking or
// picking garbage
pub struct NoHealthyLayer;

impl Test for NoHealthyLayer {
    fn name(&self) -> &str {
        "store: no healthy layer"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let store = layout.store();
        let loader = "/lib/ld.so";

        // plant empty dirs for both layers  - neither has the loader
        let planted =
            fs::create_dir_all(store.join("a")).and_then(|_| fs::create_dir_all(store.join("b")));
        if tcx.try_ok(planted, "planted empty layer trees").is_none() {
            return;
        }

        let toml = format!(
            "[layer.a]\n\
             type = \"native\"\n\
             priority = 1\n\
             [layer.a.libc]\n\
             name = \"testlibc\"\n\
             loader = \"{loader}\"\n\
             \n\
             [layer.b]\n\
             type = \"native\"\n\
             priority = 2\n\
             [layer.b.libc]\n\
             name = \"testlibc\"\n\
             loader = \"{loader}\"\n"
        );
        let dir = layout.state();
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("layers.toml");
        if fs::write(&path, toml).is_err() {
            tcx.ok(false, "write layers.toml");
            return;
        }
        let Some(layers) = tcx.try_ok(nexus::load_layers(&path), "layers parsed") else {
            return;
        };
        tcx.ok(layers.len() == 2, "both layers parsed");

        match DefaultSelector.select(&layers, &DefaultHealthCheck, &store) {
            Err(Error::NoHealthyLayer) => {
                tcx.ok(true, "all unhealthy >  NoHealthyLayer");
            }
            other => {
                tcx.ok(false, &format!("expected NoHealthyLayer, got {other:?}"));
            }
        };
    }
}
