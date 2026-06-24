// the data the renderer reads. the reporter mutates it on every event and
// redraws; the renderer is a pure function of this

use crate::log::{Level, Outcome};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Pass,
    Fail,
}

pub struct Row {
    pub name: String,
    pub status: Status,
    // assertions tallied once the test is done; zero while pending/running
    pub passed: u32,
    pub failed: u32,
}

impl Row {
    pub fn new(name: String) -> Self {
        Row { name, status: Status::Pending, passed: 0, failed: 0 }
    }

    // the category prefix (before the first ": ") and the short label after
    // it. names are "category: description"; un-prefixed names are their own
    // label with an empty category
    pub fn split(&self) -> (&str, &str) {
        match self.name.split_once(": ") {
            Some((cat, label)) => (cat, label),
            None => ("", self.name.as_str()),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub tests: Vec<Row>,
    pub logs: Vec<(Level, String)>,
    pub total: Outcome,
    // advanced each draw, drives the spinner
    pub frame: u64,
    // rows scrolled up from the tail (0 = auto-follow tail)
    pub log_scroll: usize,
}

impl State {
    pub fn set_status(&mut self, name: &str, status: Status) {
        if let Some(r) = self.tests.iter_mut().find(|r| r.name == name) {
            r.status = status;
        }
    }

    pub fn set_outcome(&mut self, name: &str, o: &Outcome) {
        if let Some(r) = self.tests.iter_mut().find(|r| r.name == name) {
            r.passed = o.passed;
            r.failed = o.failed;
        }
    }
}
