// the test registry. each test drives nexus against a live kernel inside a
// sandbox and records assertions. adding coverage means a new file here and
// one line in all()

mod cas;
mod cgroups;
mod ephemeral;
mod erofs;
mod errors;
mod evict;
mod gc;
mod glibc;
mod idmap;
mod init;
mod landlock;
mod musl;
mod rollback;
mod rootless;
mod seccomp;
mod seccomp_idmap;
mod select;
mod verify;

mod support;
pub mod userns;

use crate::log::{Level, Outcome, Reporter};
use crate::sandbox::Sandbox;
use std::path::Path;

// which libc a rootfs test exercises. tests that are not libc-specific
// report None and run under any selection
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Libc {
    Glibc,
    Musl,
}

impl Libc {
    pub fn label(self) -> &'static str {
        match self {
            Libc::Glibc => "glibc",
            Libc::Musl => "musl",
        }
    }
}

// the things spark exercises, used to group the menu and to gate on
// privilege. Store needs nothing; Namespace composes a layer and needs real
// root; Rootless drives nexus as an ordinary user through its own userns;
// Init boots nexus's pid-1 path inside a user+pid namespace
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Store,
    Init,
    Namespace,
    Rootless,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Store => "store",
            Category::Init => "init",
            Category::Namespace => "namespace",
            Category::Rootless => "rootless",
        }
    }

    // a one-line description for the menu
    pub fn description(self) -> &'static str {
        match self {
            Category::Store => "content store, gens, selection",
            Category::Init => "boot the pid-1 path: reap orphans, signal discipline",
            Category::Namespace => "compose and run a layer",
            Category::Rootless => "drive nexus as an ordinary user via userns",
        }
    }

    pub const ALL: [Category; 4] =
        [Category::Store, Category::Init, Category::Namespace, Category::Rootless];
}

// a test exercises one nexus behaviour. it asserts through TestContext, which emits
// lines and tallies the outcome; it never panics
pub trait Test {
    fn name(&self) -> &str;
    fn category(&self) -> Category;
    // libc-specific tests report their libc so the menu can filter them;
    // libc-agnostic tests return None and always run
    fn libc(&self) -> Option<Libc> {
        None
    }
    fn run(&self, tcx: &mut TestContext);
}

// the full registry, in run order: unprivileged mechanism tests first, then
// the privileged compose tests, then the rootless ones that fetch
pub fn all() -> Vec<Box<dyn Test>> {
    vec![
        Box::new(cas::StoreCas),
        Box::new(gc::GarbageCollect),
        Box::new(rollback::Rollback),
        Box::new(select::HealthySelection),
        Box::new(select::NoHealthyLayer),
        Box::new(errors::ErrorPaths),
        Box::new(verify::VerifyStore),
        Box::new(erofs::Erofs),
        Box::new(init::Isolation),
        Box::new(init::ReapOrphans),
        Box::new(init::ReaperBurst),
        Box::new(init::SignalDiscipline),
        Box::new(init::EarlyMounts),
        Box::new(init::SwitchRoot),
        Box::new(init::BootToHandoff),
        Box::new(glibc::ShadowGlibc),
        Box::new(musl::ShadowMusl),
        Box::new(ephemeral::Ephemeral),
        Box::new(seccomp::Seccomp),
        Box::new(idmap::Idmapped),
        Box::new(seccomp_idmap::SeccompIdmap),
        Box::new(landlock::Landlock),
        Box::new(cgroups::Cgroups),
        Box::new(evict::EvictRebuild),
        Box::new(rootless::RootlessGlibc),
        Box::new(rootless::RootlessMusl),
    ]
}

// the registry filtered to a chosen set of categories and libcs. a
// libc-specific test is kept only when its libc is in `libcs`; agnostic
// tests pass the libc filter. order is preserved
pub fn selected(cats: &[Category], libcs: &[Libc]) -> Vec<Box<dyn Test>> {
    all()
        .into_iter()
        .filter(|t| cats.contains(&t.category()))
        .filter(|t| t.libc().map(|l| libcs.contains(&l)).unwrap_or(true))
        .collect()
}

// passed to a test: the sandbox, a reporter to log through, and assert
// helpers that record into outcome
pub struct TestContext<'a> {
    pub sandbox: &'a Sandbox,
    reporter: &'a mut dyn Reporter,
    outcome: Outcome,
    // an optional tag prefixed to every line, so libc-parameterized tests
    // that share step labels stay distinguishable in the flat log
    scope: Option<String>,
}

impl<'a> TestContext<'a> {
    pub fn new(sandbox: &'a Sandbox, reporter: &'a mut dyn Reporter) -> Self {
        TestContext { sandbox, reporter, outcome: Outcome::default(), scope: None }
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    // tag every subsequent line with `scope: ...`
    pub fn scope(&mut self, scope: &str) {
        self.scope = Some(scope.to_string());
    }

    fn tagged(&self, text: &str) -> String {
        match &self.scope {
            Some(s) => format!("{s}: {text}"),
            None => text.to_string(),
        }
    }

    // log a plain progress line
    pub fn msg(&mut self, text: &str) {
        let line = self.tagged(text);
        self.reporter.line(Level::Msg, &line);
    }

    // log a non-fatal warning. does not touch the tally; used for skips
    pub fn wrn(&mut self, text: &str) {
        let line = self.tagged(text);
        self.reporter.line(Level::Wrn, &line);
    }

    // assert a condition. records pass or fail and logs which
    pub fn ok(&mut self, cond: bool, what: &str) -> bool {
        let line = self.tagged(what);
        if cond {
            self.outcome.passed += 1;
            self.reporter.line(Level::Ok, &line);
        } else {
            self.outcome.failed += 1;
            self.reporter.line(Level::Err, &line);
        }
        cond
    }

    // assert a path exists in the sandbox tree
    pub fn exists(&mut self, path: &Path, what: &str) -> bool {
        let cond = path.exists();
        self.ok(cond, &format!("{what}: {}", path.display()))
    }

    // unwrap a Result into an assertion: ok on Ok, fail (with the error) on
    // Err. returns the value so a test can keep going only when it makes
    // sense
    pub fn try_ok<T, E: std::fmt::Display>(&mut self, r: Result<T, E>, what: &str) -> Option<T> {
        match r {
            Ok(v) => {
                self.ok(true, what);
                Some(v)
            }
            Err(e) => {
                self.ok(false, &format!("{what}: {e}"));
                None
            }
        }
    }
}

// run argv inside a built layer and return its exit status. forks: the
// child enters the layer and execs (Core::run replaces it), the parent
// reaps. lets a test assert a process really ran inside the layer
pub fn run_in_layer(core: &mut nexus::Core, layer: &str, argv: &[&str]) -> Result<i32, String> {
    let id = nexus::LayerId::new(layer).map_err(|e| format!("layer id: {e}"))?;
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();

    // safe: spark forks single-threaded; the child only calls Core::run,
    // which execs or _exits
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("fork failed".into());
    }
    if pid == 0 {
        // run execs on success and only returns (Err) on failure to exec;
        // the Ok variant is uninhabited, so this match is exhaustive
        match core.run(&id, &owned, &[]) {
            Err(e) => eprintln!("run in layer: {e}"),
        }
        // safe: terminating the child after a failed exec
        unsafe { libc::_exit(127) };
    }

    Ok(crate::proc::exit_code(crate::proc::wait(pid)))
}

// the shared shadowed-layer flow, parameterized by libc. both headline
// tests are this with different data: same harness, two libc
//   fetch rootfs > import into the store > checkout as the layer tree >
//   write state > open a core > warm the layer > assert
pub struct ShadowedHarness {
    pub source: crate::fetch::Source,
    pub libc_name: &'static str,
    // loader path that must resolve inside the composed layer
    pub loader: &'static str,
}

impl ShadowedHarness {
    pub fn run(&self, tcx: &mut TestContext) {
        tcx.scope(&format!("namespace/{}", self.libc_name));
        let layout = tcx.sandbox.layout().clone();
        let layer = "void";

        tcx.msg(&format!("fetch {}", self.source.name));
        let Some(rootfs) = tcx.try_ok(crate::fetch::fetch(&self.source), "rootfs fetched") else {
            return;
        };

        let store = nexus::Store::new(layout.store());
        let Some(tree) = tcx.try_ok(store.import(&rootfs), "rootfs imported") else {
            return;
        };

        // activate the tree where the overlay backend reads the layer lower
        let lower = layout.store().join(layer);
        if tcx.try_ok(store.checkout(&tree, &lower), "tree checked out").is_none() {
            return;
        }

        // a marker that exists only in this layer's tree. seeing it at / from
        // inside proves the process is rooted in the composed layer, not the
        // host. planted in the lower so the overlay surfaces it read-only
        let _ = std::fs::write(lower.join(ROOT_MARKER), b"spark");

        if tcx
            .try_ok(
                support::write_shadow_state(&layout, layer, self.libc_name, self.loader),
                "state written",
            )
            .is_none()
        {
            return;
        }

        let layers = match nexus::load_layers(&layout.state().join("layers.toml")) {
            Ok(l) => l,
            Err(e) => {
                tcx.ok(false, &format!("load layers: {e}"));
                return;
            }
        };
        let system = match nexus::load_system(&layout.state().join("nexus.toml")) {
            Ok(s) => s,
            Err(e) => {
                tcx.ok(false, &format!("load system: {e}"));
                return;
            }
        };

        let mut core = nexus::Core::open(layout.clone(), layers, system);
        tcx.try_ok(core.build(layer), "namespace built and pinned");

        tcx.exists(&layout.ns_file(layer), "ns file pinned");

        // a process actually runs inside the layer
        match run_in_layer(&mut core, layer, &["/bin/true"]) {
            Ok(0) => {
                tcx.ok(true, "ran /bin/true in layer");
            }
            Ok(code) => {
                tcx.ok(false, &format!("/bin/true exited {code}"));
            }
            Err(e) => {
                tcx.ok(false, &format!("run in layer: {e}"));
            }
        }

        // the running process is rooted in the composed layer: the marker is
        // reachable at / (it exists only in this layer's tree). `test -e`
        // exits 0 iff the path is present from inside the namespace
        let marker_at_root = format!("/{ROOT_MARKER}");
        match run_in_layer(&mut core, layer, &["/usr/bin/test", "-e", &marker_at_root]) {
            Ok(0) => {
                tcx.ok(true, "process is rooted in the layer (marker visible at /)");
            }
            Ok(code) => {
                tcx.ok(false, &format!("layer-root marker not visible at /: test exited {code}"));
            }
            Err(e) => {
                tcx.ok(false, &format!("root-identity check: {e}"));
            }
        }
    }
}

// a filename planted only in a layer's tree, used to prove a process really
// runs rooted in the composed layer rather than the host
const ROOT_MARKER: &str = ".spark-layer-root";

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn registry_non_empty_unique() {
        let all = all();
        assert!(!all.is_empty());
        let mut names: Vec<&str> = all.iter().map(|t| t.name()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate test names");
    }
}
