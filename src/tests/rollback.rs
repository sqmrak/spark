// drive nexus gens through Core: commit two, activate, roll back. also
// covers a missing gen (activate > error) and crash atomicity (a leftover
// .current.next must not corrupt the live current symlink). no root

use super::{Category, TestContext, Test};
use nexus::{Core, Generations, ObjectHash, System};

pub struct Rollback;

impl Test for Rollback {
    fn name(&self) -> &str {
        "store: commit, roll back & crash safety"
    }

    fn category(&self) -> Category {
        Category::Store
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        // gens do not need layers; an empty core is enough. keep the cgroup
        // subtree inside the sandbox so open touches nothing on the host
        let system = System { cgroup_root: layout.root().join(".cgroup"), ..System::default() };
        let core = Core::open(layout.clone(), Vec::new(), system);

        let h = |s: &str| ObjectHash::new(s);
        let Some(g1) = tcx.try_ok(core.commit(&[h("aaaa")]), "committed gen 1") else {
            return;
        };
        let Some(g2) = tcx.try_ok(core.commit(&[h("bbbb")]), "committed gen 2") else {
            return;
        };
        tcx.ok(g2 > g1, "gen numbers increase");

        tcx.try_ok(core.activate_gen(g2), "activated gen 2");
        tcx.ok(core.current_gen().ok() == Some(g2), "current is gen 2");

        // the rollback: activate the older gen
        tcx.try_ok(core.activate_gen(g1), "rolled back to gen 1");
        tcx.ok(core.current_gen().ok() == Some(g1), "rollback took, current is gen 1");

        // activating a gen that was never committed must fail, not panic,
        // and must leave current untouched
        let nonexistent_gen = nexus::Gen::new(999);
        tcx.ok(core.activate_gen(nonexistent_gen).is_err(), "activate missing gen >  error");
        tcx.ok(core.current_gen().ok() == Some(g1), "current unchanged after failed activate");

        // crash atomicity: a leftover .current.next from a crash mid-activate
        // must not corrupt the live symlink. plant a stray temp, then a
        // normal activate must still land cleanly on its target
        let gens = Generations::new(layout.gens());
        let stray = layout.gens().join(".current.next");
        let _ = std::os::unix::fs::symlink("999", &stray);
        tcx.try_ok(gens.activate(g2), "activate over a stray .current.next");
        tcx.ok(core.current_gen().ok() == Some(g2), "current is the activated gen, not the stray");
        // current always resolves to a real gen directory
        tcx.ok(
            core.current_gen().map(|g| layout.gens().join(g.to_string()).is_dir()).unwrap_or(false),
            "current points at a real gen dir",
        );
    }
}
