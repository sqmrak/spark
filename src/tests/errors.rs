// every error path returns the right Error variant, never panics
// concurrent imports converge under the object lock. unprivileged

use super::support::{open_core, write_state, LayerSpec};
use super::{Category, TestContext, Test};
use nexus::{Error, LayerId, ObjectHash, Store};

pub struct ErrorPaths;

impl Test for ErrorPaths {
    fn name(&self) -> &str {
        "store: clean error variants"
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
        if tcx.try_ok(state, "wrote layer state").is_none() {
            return;
        }
        let Some(mut core) = tcx.try_ok(open_core(&layout), "opened core") else {
            return;
        };

        // unknown layer > UnknownLayer, carrying the name
        match core.build("nonexistent_gen") {
            Err(Error::UnknownLayer(n)) => {
                tcx.ok(n == "nonexistent_gen", "build unknown >  UnknownLayer(nonexistent_gen)")
            }
            other => tcx.ok(false, &format!("build unknown: expected UnknownLayer, got {other:?}")),
        };

        // run on an unknown layer > UnknownLayer too (errors before exec)
        let missing = LayerId::new("nonexistent_gen").expect("\"nonexistent_gen\" is a valid layer id");
        match core.run(&missing, &["/bin/true".into()], &[]) {
            Err(Error::UnknownLayer(_)) => tcx.ok(true, "run unknown >  UnknownLayer"),
            other => tcx.ok(false, &format!("run unknown: expected UnknownLayer, got {other:?}")),
        };

        // checkout of an absent tree > Corrupt, carrying the hash
        let store = Store::new(layout.store());
        let invalid = ObjectHash::new("f".repeat(64));
        match store.checkout(&invalid, &layout.store().join("out")) {
            Err(Error::Corrupt { hash }) => {
                tcx.ok(hash.contains("ffff"), "checkout missing tree >  Corrupt(hash)")
            }
            other => tcx.ok(false, &format!("checkout missing: expected Corrupt, got {other:?}")),
        };

        // bad layers.toml > Config (parse error), not a panic
        let bad = layout.state().join("layers.toml");
        if tcx
            .try_ok(std::fs::write(&bad, "this is not = valid [toml"), "wrote bad-toml fixture")
            .is_none()
        {
            return;
        }
        match nexus::load_layers(&bad) {
            Err(Error::Config(_)) => tcx.ok(true, "bad layers.toml >  Config error"),
            other => tcx.ok(false, &format!("bad toml: expected Config, got {other:?}")),
        };
        // unknown layer type > Config
        let unknown_type =
            "[layer.x]\ntype = \"invalid\"\npriority = 1\n[layer.x.libc]\nname = \"glibc\"\n";
        if tcx.try_ok(std::fs::write(&bad, unknown_type), "wrote unknown-type fixture").is_none() {
            return;
        }
        match nexus::load_layers(&bad) {
            Err(Error::Config(m)) => {
                tcx.ok(m.contains("invalid"), "unknown layer type >  Config(invalid)")
            }
            other => tcx.ok(false, &format!("bad type: expected Config, got {other:?}")),
        };

        // an invalid layer id (path separator) is rejected at load, not
        // silently used as a directory name later
        let bad_id = "[layer.\"a/b\"]\ntype = \"native\"\npriority = 1\n[layer.\"a/b\".libc]\nname = \"glibc\"\n";
        if tcx.try_ok(std::fs::write(&bad, bad_id), "wrote bad-id fixture").is_none() {
            return;
        }
        match nexus::load_layers(&bad) {
            Err(Error::Config(_)) => tcx.ok(true, "invalid layer id rejected at load"),
            other => tcx.ok(false, &format!("bad id: expected Config, got {other:?}")),
        };

        // concurrent imports of identical content converge through the pool
        // lock: same tree hash from every thread, one object per content
        concurrent_import(tcx, &layout);
    }
}

fn concurrent_import(tcx: &mut TestContext, layout: &nexus::Layout) {
    let src = tcx.sandbox.base().join("conc-src");
    let built = std::fs::create_dir_all(src.join("usr/lib"))
        .and_then(|_| std::fs::write(src.join("usr/lib/libc.so"), b"SHARED"))
        .and_then(|_| std::fs::write(src.join("etc-os"), b"void"));
    if tcx.try_ok(built, "wrote concurrent-import fixture").is_none() {
        return;
    }

    let store_root = layout.store().join("conc");
    let mut handles = Vec::new();
    for _ in 0..8 {
        let root = store_root.clone();
        let s = src.clone();
        handles.push(std::thread::spawn(move || Store::new(root).import(&s)));
    }
    let hashes: Vec<_> =
        handles.into_iter().filter_map(|h| h.join().ok()).filter_map(|r| r.ok()).collect();

    tcx.ok(hashes.len() == 8, "all 8 concurrent imports returned a hash");
    let converged = hashes.windows(2).all(|w| w[0] == w[1]);
    tcx.ok(converged, "concurrent imports converge to one tree hash");

    let objs = std::fs::read_dir(store_root.join("objects"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0);
    tcx.ok(objs == 2, &format!("pool holds 2 objects, not torn duplicates ({objs})"));
}
