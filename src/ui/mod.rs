// the tui frontend: a Reporter that draws with ratatui instead of
// printing. the engine is unchanged; main swaps this in when stdout is a
// terminal. tests block (mounts, fork), so the screen redraws on each
// reporter event rather than from a render loop

mod logo;
mod menu;
mod render;
mod state;

pub use menu::{Privileges, Selection};

use crate::log::{Level, Outcome, Reporter};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use ratatui::DefaultTerminal;
use state::{Row, State, Status};
use std::time::Instant;

pub struct Tui {
    term: DefaultTerminal,
    st: State,
    start: Instant,
}

impl Tui {
    // enter the alternate screen and raw mode. paired with restore on drop
    // not Default: construction has the side effect of taking over the
    // terminal
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
        Tui { term: ratatui::init(), st: State::default(), start: Instant::now() }
    }

    // run the interactive selection screen. returns the chosen tests, or None
    // if the user quit without starting
    pub fn select(&mut self, caps: Privileges) -> Option<Selection> {
        let mut st = menu::MenuState::new(caps);
        loop {
            let _ = self.term.draw(|f| render::menu(f, &st));
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    match menu::on_key(&mut st, k.code) {
                        menu::Action::Start => return st.selection(),
                        menu::Action::Quit => return None,
                        _ => {}
                    }
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    fn draw(&mut self) {
        self.st.frame = (self.start.elapsed().as_millis() / 100) as u64;
        let st = &self.st;
        let _ = self.term.draw(|f| render::draw(f, st));
    }

    // hold the final screen until the user presses q, esc or enter, then
    // after the run, write a few summary lines into the log: the test and
    // assertion tally, and how to leave. shown before finish() blocks
    pub fn summary(&mut self) {
        let total = self.st.tests.len();
        let failed = self.st.tests.iter().filter(|r| r.status == Status::Fail).count();
        let passed = total - failed;
        let a = self.st.total;

        // a blank line sets the summary apart from the run's last line
        self.st.logs.push((Level::Msg, String::new()));
        if failed == 0 {
            self.st.logs.push((
                Level::Ok,
                format!("{passed}/{total} tests passed · {} assertions green", a.passed),
            ));
        } else {
            self.st.logs.push((
                Level::Err,
                format!("{passed}/{total} tests passed · {} assertions failed", a.failed),
            ));
        }
        self.st.logs.push((Level::Tip, "press q, esc or enter to quit".into()));
        self.draw();
    }

    // hold the final screen until the user quits (q/esc/enter); arrow keys and
    // the mouse wheel scroll the log meanwhile. mouse capture is disabled here
    // so the terminal's native text selection works for copying output
    pub fn finish(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        const SCROLL_PAGE: usize = 10;
        const SCROLL_WHEEL: usize = 3;

        self.st.log_scroll = 0;
        self.draw();
        loop {
            match event::read() {
                Ok(Event::Key(k))
                    if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter) =>
                {
                    break;
                }
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Up => self.st.log_scroll = self.st.log_scroll.saturating_add(1),
                        KeyCode::Down => self.st.log_scroll = self.st.log_scroll.saturating_sub(1),
                        KeyCode::PageUp => {
                            self.st.log_scroll = self.st.log_scroll.saturating_add(SCROLL_PAGE)
                        }
                        KeyCode::PageDown => {
                            self.st.log_scroll = self.st.log_scroll.saturating_sub(SCROLL_PAGE)
                        }
                        _ => {}
                    }
                    self.draw();
                }
                Ok(Event::Mouse(m)) if m.kind == MouseEventKind::ScrollUp => {
                    self.st.log_scroll = self.st.log_scroll.saturating_add(SCROLL_WHEEL);
                    self.draw();
                }
                Ok(Event::Mouse(m)) if m.kind == MouseEventKind::ScrollDown => {
                    self.st.log_scroll = self.st.log_scroll.saturating_sub(SCROLL_WHEEL);
                    self.draw();
                }
                Ok(Event::Resize(_, _)) => self.draw(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        ratatui::restore();
    }
}

impl Reporter for Tui {
    fn plan(&mut self, names: &[&str]) {
        self.st.tests = names.iter().map(|n| Row::new(n.to_string())).collect();
        self.draw();
    }

    fn line(&mut self, level: Level, text: &str) {
        self.st.logs.push((level, text.to_string()));
        self.draw();
    }

    fn test_start(&mut self, name: &str) {
        self.st.set_status(name, Status::Running);
        self.draw();
    }

    fn test_done(&mut self, name: &str, outcome: &Outcome) {
        let status = if outcome.is_ok() { Status::Pass } else { Status::Fail };
        self.st.set_status(name, status);
        self.st.set_outcome(name, outcome);
        self.st.total.passed += outcome.passed;
        self.st.total.failed += outcome.failed;
        self.draw();
    }
}

#[cfg(test)]
mod tests {
    use super::state::{Row, State, Status};
    use crate::log::Level;
    use ratatui::{backend::TestBackend, Terminal};

    // render to an in-memory buffer: guards the layout against panics and
    // checks the headline content lands. no real terminal needed
    #[test]
    fn renders_without_panic() {
        let st = State {
            tests: vec![
                Row {
                    name: "namespace: glibc".into(),
                    status: Status::Pass,
                    passed: 8,
                    failed: 0,
                },
                Row {
                    name: "namespace: musl".into(),
                    status: Status::Running,
                    passed: 0,
                    failed: 0,
                },
            ],
            logs: vec![
                (Level::Msg, "fetch glibc rootfs".into()),
                (Level::Ok, "rootfs imported".into()),
            ],
            ..State::default()
        };

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| super::render::draw(f, &st)).unwrap();

        let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        // wide header shows the wordmark art; check a stable fragment of it
        assert!(text.contains("/___/"));
        assert!(text.contains("tests"));
        assert!(text.contains("rootfs imported"));
    }

    // on a narrow terminal the bolt is dropped, but the title must survive
    #[test]
    fn narrow_keeps_title() {
        let st = State::default();
        let mut term = Terminal::new(TestBackend::new(20, 16)).unwrap();
        term.draw(|f| super::render::draw(f, &st)).unwrap();
        let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("spark"), "title must survive a narrow header");
    }

    // the menu renders the header art and all category/libc rows
    #[test]
    fn menu_renders() {
        let caps = super::menu::Privileges { root: false, userns: true };
        let st = super::menu::MenuState::new(caps);
        let mut term = Terminal::new(TestBackend::new(74, 18)).unwrap();
        term.draw(|f| super::render::menu(f, &st)).unwrap();
        let lines: Vec<String> = {
            let buf = term.backend().buffer().clone();
            (0..buf.area().height)
                .map(|y| {
                    (0..buf.area().width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect()
        };
        let text = lines.join("\n");
        assert!(text.contains("/___/"));
        assert!(text.contains("store"));
        assert!(text.contains("namespace"));
        assert!(text.contains("rootless"));
        assert!(text.contains("glibc"));
        assert!(text.contains("musl"));
        if std::env::var("PEEK").is_ok() {
            eprintln!("\n{text}\n");
        }
    }

    // peek the full grouped results screen with a finished run
    #[test]
    fn results_screen_peek() {
        let row =
            |n: &str, p: u32| Row { name: n.into(), status: Status::Pass, passed: p, failed: 0 };
        let st = State {
            tests: vec![
                row("store: import, dedup, checkout", 6),
                row("store: commit & roll back gens", 7),
                row("store: select healthy layer", 4),
                row("store: clean error paths", 3),
                row("namespace: glibc", 8),
                row("namespace: musl", 8),
                row("rootless: glibc", 8),
                row("rootless: musl", 8),
            ],
            logs: vec![
                (Level::Ok, "rootless/musl: ran /bin/true rootless".into()),
                (Level::Ok, "rootless/musl: process rooted in layer (marker at /)".into()),
                (Level::Msg, String::new()),
                (Level::Ok, "8/8 tests passed · 52 assertions green".into()),
                (Level::Tip, "press q, esc or enter to quit".into()),
            ],
            total: crate::log::Outcome { passed: 52, failed: 0 },
            frame: 0,
            log_scroll: 0,
        };
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| super::render::draw(f, &st)).unwrap();
        if std::env::var("PEEK").is_ok() {
            let buf = term.backend().buffer().clone();
            for y in 0..buf.area().height {
                let row: String =
                    (0..buf.area().width).map(|x| buf[(x, y)].symbol().to_string()).collect();
                eprintln!("{row}");
            }
        }
    }
}
