// log format: "[tag] text", colored by level. engine emits through a Reporter,
// frontend renders the same events however it likes

use crossterm::style::{Color, Stylize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Err,
    Msg,
    Wrn,
    Tip,
    Ok,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Err => "err",
            Level::Msg => "msg",
            Level::Wrn => "wrn",
            Level::Tip => "tip",
            Level::Ok => "ok",
        }
    }

    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Level::Err => (220, 70, 60),
            Level::Msg => (60, 190, 205),
            Level::Wrn => (210, 180, 50),
            Level::Tip => (70, 170, 210),
            Level::Ok => (90, 190, 100),
        }
    }

    fn tag_bracketed(self) -> &'static str {
        match self {
            Level::Err => "[err]",
            Level::Msg => "[msg]",
            Level::Wrn => "[wrn]",
            Level::Tip => "[tip]",
            Level::Ok => "[ok]",
        }
    }
}

pub struct Line<'a> {
    pub level: Level,
    pub text: &'a str,
}

impl fmt::Display for Line<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (r, g, b) = self.level.rgb();
        let tag = self.level.tag_bracketed().with(Color::Rgb { r, g, b });
        write!(f, "{tag} {}", self.text)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub passed: u32,
    pub failed: u32,
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        self.failed == 0
    }
}

// seam between engine and ui. text reporter prints; tui reporter draws
pub trait Reporter {
    fn plan(&mut self, _names: &[&str]) {}
    fn line(&mut self, level: Level, text: &str);
    fn test_start(&mut self, name: &str);
    fn test_done(&mut self, name: &str, outcome: &Outcome);
}

pub struct TextReporter;

impl Reporter for TextReporter {
    fn line(&mut self, level: Level, text: &str) {
        println!("{}", Line { level, text });
    }

    fn test_start(&mut self, name: &str) {
        self.line(Level::Msg, &format!("test {name}"));
    }

    fn test_done(&mut self, name: &str, outcome: &Outcome) {
        let level = if outcome.is_ok() { Level::Ok } else { Level::Err };
        self.line(level, &format!("{name}: {} passed, {} failed", outcome.passed, outcome.failed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stable() {
        assert_eq!(Level::Err.tag(), "err");
        assert_eq!(Level::Ok.tag(), "ok");
        assert_eq!(Level::Tip.tag(), "tip");
    }

    #[test]
    fn line_starts_with_bracketed_tag() {
        let s = Line { level: Level::Msg, text: "hello" }.to_string();
        assert!(s.contains("[msg]"));
        assert!(s.ends_with("hello"));
    }

    #[test]
    fn outcome_ok_iff_no_failures() {
        assert!(Outcome { passed: 3, failed: 0 }.is_ok());
        assert!(!Outcome { passed: 1, failed: 1 }.is_ok());
    }
}
