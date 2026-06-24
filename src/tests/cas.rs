// drive nexus's content-addressed store with a synthetic rootfs: import,
// dedup, checkout. no network, no root; exercises the real cas path

use super::support::first_object;
use super::{Category, TestContext, Test};
use nexus::Store;
use std::fs;

pub struct StoreCas;

impl Test for StoreCas {
    fn name(&self) -> &str {
        "store: import, dedup, checkout"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let src = tcx.sandbox.base().join("src");

        // two files share content, so the store should hold one object for
        // them, not two
        write(&src.join("usr/bin/sh"), "ELF-SH");
        write(&src.join("usr/lib/libc.so"), "SHARED-LIB");
        write(&src.join("opt/copy/libc.so"), "SHARED-LIB");

        let store = Store::new(layout.store());
        let Some(tree) = tcx.try_ok(store.import(&src), "rootfs imported") else {
            return;
        };
        tcx.ok(store.has(&tree), "tree present after import");

        // three files, two distinct contents > two objects
        let objects = count_dir(&layout.store().join("objects"));
        tcx.ok(objects == 2, &format!("shared content deduplicated ({objects} objects)"));

        // import again: same content, same tree hash, idempotent
        let again = store.import(&src);
        tcx.ok(again.map(|h| h == tree).unwrap_or(false), "re-import is idempotent");

        // checkout reconstructs the tree
        let out = tcx.sandbox.base().join("out");
        if tcx.try_ok(store.checkout(&tree, &out), "tree checked out").is_some() {
            let got = fs::read_to_string(out.join("usr/lib/libc.so")).unwrap_or_default();
            tcx.ok(got == "SHARED-LIB", "content round-trips");
        }

        // the integrity check passes on an intact store...
        match store.verify() {
            Ok(n) => tcx.ok(n == 2, &format!("store verifies ({n} objects)")),
            Err(e) => tcx.ok(false, &format!("store verify: {e}")),
        };

        // ...and catches a tampered object
        if let Some(obj) = first_object(&layout.store().join("objects")) {
            let _ = fs::write(&obj, "CORRUPT");
            tcx.ok(store.verify().is_err(), "verify catches tampering");
        }
    }
}

fn write(path: &std::path::Path, content: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, content);
}

// count real objects, skipping bookkeeping dotfiles (the pool lock, staging
// temp files)
fn count_dir(p: &std::path::Path) -> usize {
    fs::read_dir(p)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        })
        .unwrap_or(0)
}
