// draw the whole screen from the reporter state: header with the wordmark,
// a test list on the left, a colored log pane on the right, a footer with
// the running tally

use super::logo;
use super::menu::{ItemKind, MenuState};
use super::state::{Row, State, Status};
use crate::log::Level;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const HEADER_H: u16 = logo::WORDMARK.len() as u16 + 1;
const WORDMARK_FG: Color = Color::Yellow;

pub fn draw(f: &mut Frame, st: &State) {
    let rows =
        Layout::vertical([Constraint::Length(HEADER_H), Constraint::Min(0), Constraint::Length(1)])
            .split(f.area());

    header(f, rows[0]);
    body(f, rows[1], st);
    footer(f, rows[2], st);
}

// the wordmark needs a wide header; below that, fall back to the plain name
const WORDMARK_MIN: u16 = 28;

fn header(f: &mut Frame, area: Rect) {
    if area.width >= WORDMARK_MIN {
        let mut lines: Vec<Line> = vec![Line::from("")];
        for l in logo::WORDMARK {
            lines.push(Line::from(Span::raw(format!("  {l}")).fg(WORDMARK_FG)));
        }
        f.render_widget(Paragraph::new(lines), area);
    } else {
        let title = vec![Line::from(Span::raw(logo::NAME).fg(Color::Yellow).bold())];
        let h = title.len() as u16;
        f.render_widget(Paragraph::new(title), center_v(area, h));
    }
}

// the selection screen: the header, then the category and libc toggles, then
// a key hint
pub fn menu(f: &mut Frame, st: &MenuState) {
    let rows =
        Layout::vertical([Constraint::Length(HEADER_H), Constraint::Min(0), Constraint::Length(1)])
            .split(f.area());

    header(f, rows[0]);
    menu_body(f, rows[1], st);
    menu_hint(f, rows[2]);
}

fn menu_body(f: &mut Frame, area: Rect, st: &MenuState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" select tests ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let mut last_was_cat = false;
    for (i, it) in st.items.iter().enumerate() {
        let is_cat = matches!(it.class, ItemKind::Cat(_));
        // a blank gap between the category block and the libc block
        if !is_cat && last_was_cat {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::raw("  rootfs libc").fg(Color::DarkGray)));
        }
        if i == 0 {
            lines.push(Line::from(Span::raw("  categories").fg(Color::DarkGray)));
        }
        last_was_cat = is_cat;

        let cursor = if i == st.cursor { "›" } else { " " };
        let boxed = if !it.enabled {
            "[-]"
        } else if it.on {
            "[x]"
        } else {
            "[ ]"
        };
        let box_fg = if !it.enabled {
            Color::DarkGray
        } else if it.on {
            Color::Green
        } else {
            Color::Gray
        };
        let label_fg = if it.enabled { Color::White } else { Color::DarkGray };
        lines.push(Line::from(vec![
            Span::raw(format!(" {cursor} ")).fg(Color::Yellow),
            Span::raw(format!("{boxed} ")).fg(box_fg),
            Span::raw(it.label.clone()).fg(label_fg).bold(),
            Span::raw(format!("  {}", it.description)).fg(Color::DarkGray),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn menu_hint(f: &mut Frame, area: Rect) {
    let hint = Line::from(vec![
        Span::raw(" ↑↓ move").fg(Color::Gray),
        Span::raw(" · ").fg(Color::DarkGray),
        Span::raw("space toggle").fg(Color::Gray),
        Span::raw(" · ").fg(Color::DarkGray),
        Span::raw("a all").fg(Color::Gray),
        Span::raw(" · ").fg(Color::DarkGray),
        Span::raw("enter run").fg(Color::Green),
        Span::raw(" · ").fg(Color::DarkGray),
        Span::raw("q quit").fg(Color::Gray),
    ]);
    f.render_widget(Paragraph::new(hint).style(Style::default().add_modifier(Modifier::DIM)), area);
}

fn body(f: &mut Frame, area: Rect, st: &State) {
    // the test list takes about 40%, wide enough for the grouped labels but
    // bounded so the log pane keeps usable width on any terminal
    let list_w = (area.width * 40 / 100).clamp(22, 40).min(area.width);
    let cols = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)]).split(area);
    test_list(f, cols[0], st);
    log_pane(f, cols[1], st);
}

fn test_list(f: &mut Frame, area: Rect, st: &State) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" tests ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // group rows under their category heading. names are "category: label";
    // the heading is shown once, each row indented under it
    let mut lines: Vec<Line> = Vec::new();
    let mut last_cat: Option<&str> = None;
    for r in &st.tests {
        let (cat, label) = r.split();
        if last_cat != Some(cat) {
            if last_cat.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(Span::raw(cat.to_string()).fg(Color::DarkGray)));
            last_cat = Some(cat);
        }
        lines.push(test_line(r, label, st.frame));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn test_line(r: &Row, label: &str, frame: u64) -> Line<'static> {
    let (icon, color) = match r.status {
        Status::Pending => ("·".to_string(), Color::DarkGray),
        Status::Running => (SPINNER[(frame as usize) % SPINNER.len()].to_string(), Color::Yellow),
        Status::Pass => ("✓".to_string(), Color::Green),
        Status::Fail => ("✗".to_string(), Color::Red),
    };
    let label_fg = match r.status {
        Status::Pending => Color::DarkGray,
        _ => Color::Gray,
    };
    let mut spans =
        vec![Span::raw(format!(" {icon} ")).fg(color), Span::raw(label.to_string()).fg(label_fg)];
    // a per-test count once it has run, so the list carries the tally too
    if matches!(r.status, Status::Pass | Status::Fail) {
        let tally = if r.failed > 0 {
            Span::raw(format!("  {}/{}", r.passed, r.passed + r.failed)).fg(Color::Red)
        } else {
            Span::raw(format!("  {}", r.passed)).fg(Color::DarkGray)
        };
        spans.push(tally);
    }
    Line::from(spans)
}

fn log_pane(f: &mut Frame, area: Rect, st: &State) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" log ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    let cap = inner.height as usize;

    // the visible window: scroll rows above the tail, clamped to the log
    let total = st.logs.len();
    let scroll = st.log_scroll.min(total.saturating_sub(cap));
    let start = total.saturating_sub(cap + scroll);
    let end = total.saturating_sub(scroll);
    let lines: Vec<Line> = st.logs[start..end].iter().map(|(lvl, t)| log_line(*lvl, t)).collect();

    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn log_line(level: Level, text: &str) -> Line<'static> {
    // a blank entry is a spacer: no tag, an empty line
    if text.is_empty() {
        return Line::raw("");
    }
    // tag and color come from Level, the single source shared with the text
    // frontend; here they are only converted to a ratatui color
    let (r, g, b) = level.rgb();
    Line::from(vec![
        Span::raw(format!("[{}]", level.tag())).fg(Color::Rgb(r, g, b)),
        Span::raw(format!(" {text}")).fg(Color::Gray),
    ])
}

fn footer(f: &mut Frame, area: Rect, st: &State) {
    let pass = Span::raw(format!(" {} passed", st.total.passed)).fg(Color::Green);
    let sep = Span::raw(" · ").fg(Color::DarkGray);
    let fail = Span::raw(format!("{} failed", st.total.failed)).fg(if st.total.failed > 0 {
        Color::Red
    } else {
        Color::DarkGray
    });
    let hint = Span::raw("  q to quit").fg(Color::DarkGray);
    let line = Line::from(vec![pass, sep, fail, hint]);
    f.render_widget(Paragraph::new(line).style(Style::default().add_modifier(Modifier::DIM)), area);
}

// a one-row-tall area centered vertically inside area, height rows tall
fn center_v(area: Rect, height: u16) -> Rect {
    let pad = area.height.saturating_sub(height) / 2;
    Rect { y: area.y + pad, height: height.min(area.height), ..area }
}
