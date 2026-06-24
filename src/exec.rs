// each test gets a fresh sandbox, runs, reports, sandbox is torn down

use crate::log::{Level, Outcome, Reporter};
use crate::sandbox::Sandbox;
use crate::tests::{self, Category, TestContext, Test};

pub fn run_all(reporter: &mut dyn Reporter) -> Outcome {
    run(reporter, tests::all())
}

pub fn run(reporter: &mut dyn Reporter, tests: Vec<Box<dyn Test>>) -> Outcome {
    let mut total = Outcome::default();
    let names: Vec<&str> = tests.iter().map(|t| t.name()).collect();
    reporter.plan(&names);
    for test in tests {
        reporter.test_start(test.name());
        // init tests boot in the boot harness, they only need the path scheme
        let sandbox = if test.category() == Category::Init {
            Sandbox::path_only(test.name())
        } else {
            match Sandbox::create(test.name()) {
                Ok(s) => s,
                Err(e) => {
                    reporter.line(Level::Err, &format!("sandbox: {e}"));
                    total.failed += 1;
                    continue;
                }
            }
        };
        let outcome = {
            let mut tcx = TestContext::new(&sandbox, reporter);
            test.run(&mut tcx);
            tcx.outcome()
        };
        reporter.test_done(test.name(), &outcome);
        total.passed += outcome.passed;
        total.failed += outcome.failed;
    }
    total
}
