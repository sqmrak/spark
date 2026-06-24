// the pre-run selection screen: pick which categories to run and which
// libcs the rootfs images use. arrow keys move, space toggles, enter
// starts, q quits

use crate::tests::{Category, Libc};
use crossterm::event::KeyCode;

// what the current user can do, which gates the menu
pub struct Privileges {
    pub root: bool,
    pub userns: bool,
}

impl Privileges {
    pub fn detect() -> Self {
        let root = unsafe { libc::geteuid() } == 0;
        Privileges { root, userns: root || probe_userns() }
    }
}

// fork a child that tries to create a user namespace; clean signal of
// whether rootless paths can run at all
fn probe_userns() -> bool {
    // safe: the tui runs single-threaded (blocking event::read, no render
    // thread), so this fork has no sibling thread holding a lock
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        let r = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
        unsafe { libc::_exit(if r == 0 { 0 } else { 1 }) };
    }
    let status = crate::proc::wait(pid);
    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
}

// the user's choice, handed to tests::selected
pub struct Selection {
    pub cats: Vec<Category>,
    pub libcs: Vec<Libc>,
}

pub enum ItemKind {
    Cat(Category),
    Libc(Libc),
}

pub struct Item {
    pub class: ItemKind,
    pub label: String,
    pub description: String,
    pub on: bool,
    pub enabled: bool,
}

pub struct MenuState {
    pub items: Vec<Item>,
    pub cursor: usize,
}

impl MenuState {
    pub fn new(caps: Privileges) -> Self {
        let mut items = Vec::new();
        for c in Category::ALL {
            let (enabled, missing) = match c {
                Category::Store => (true, ""),
                Category::Init => (caps.userns, " (needs user namespaces)"),
                Category::Namespace => (caps.root, " (needs root)"),
                Category::Rootless => (caps.userns, " (needs user namespaces)"),
            };
            items.push(Item {
                class: ItemKind::Cat(c),
                label: c.label().to_string(),
                description: if enabled {
                    c.description().to_string()
                } else {
                    format!("{}{missing}", c.description())
                },
                on: enabled,
                enabled,
            });
        }
        for l in [Libc::Glibc, Libc::Musl] {
            items.push(Item {
                class: ItemKind::Libc(l),
                label: l.label().to_string(),
                description: format!("use the {} rootfs", l.label()),
                on: true,
                enabled: true,
            });
        }
        MenuState { items, cursor: 0 }
    }

    // the chosen categories and libcs, or None when nothing runnable is on
    pub fn selection(&self) -> Option<Selection> {
        let cats: Vec<Category> = self
            .items
            .iter()
            .filter_map(|i| match i.class {
                ItemKind::Cat(c) if i.on => Some(c),
                _ => None,
            })
            .collect();
        let libcs: Vec<Libc> = self
            .items
            .iter()
            .filter_map(|i| match i.class {
                ItemKind::Libc(l) if i.on => Some(l),
                _ => None,
            })
            .collect();
        if cats.is_empty() {
            return None;
        }
        Some(Selection { cats, libcs })
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        self.cursor = (((self.cursor as isize + delta) % n + n) % n) as usize;
    }

    fn toggle(&mut self) {
        let it = &mut self.items[self.cursor];
        if it.enabled {
            it.on = !it.on;
        }
    }

    fn all_on(&mut self) {
        for it in &mut self.items {
            if it.enabled {
                it.on = true;
            }
        }
    }
}

// what a key did to the menu
pub enum Action {
    Redraw,
    Start,
    Quit,
    Ignore,
}

pub fn on_key(st: &mut MenuState, key: KeyCode) -> Action {
    match key {
        KeyCode::Up | KeyCode::Char('k') => {
            st.move_by(-1);
            Action::Redraw
        }
        KeyCode::Down | KeyCode::Char('j') => {
            st.move_by(1);
            Action::Redraw
        }
        KeyCode::Char(' ') => {
            st.toggle();
            Action::Redraw
        }
        KeyCode::Char('a') => {
            st.all_on();
            Action::Redraw
        }
        KeyCode::Enter => {
            if st.selection().is_some() {
                Action::Start
            } else {
                Action::Ignore
            }
        }
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        _ => Action::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(root: bool, userns: bool) -> Privileges {
        Privileges { root, userns }
    }

    #[test]
    fn root_gates_namespace_category() {
        let nonroot = MenuState::new(caps(false, true));
        let ns = nonroot
            .items
            .iter()
            .find(|i| matches!(i.class, ItemKind::Cat(Category::Namespace)))
            .unwrap();
        assert!(!ns.enabled, "namespace needs root");
        assert!(!ns.on);

        let root = MenuState::new(caps(true, true));
        let ns = root
            .items
            .iter()
            .find(|i| matches!(i.class, ItemKind::Cat(Category::Namespace)))
            .unwrap();
        assert!(ns.enabled && ns.on);
    }

    #[test]
    fn space_toggles_only_enabled() {
        let mut st = MenuState::new(caps(false, true));
        // cursor starts on the store row (enabled, on by default)
        on_key(&mut st, KeyCode::Char(' '));
        assert!(!st.items[0].on, "store toggled off");
        // namespace is disabled without root: space over it is a no-op
        let ns = st
            .items
            .iter()
            .position(|i| matches!(i.class, ItemKind::Cat(Category::Namespace)))
            .unwrap();
        assert!(!st.items[ns].enabled);
        st.cursor = ns;
        on_key(&mut st, KeyCode::Char(' '));
        assert!(!st.items[ns].on);
    }

    #[test]
    fn enter_needs_a_category() {
        let mut st = MenuState::new(caps(false, true));
        // turn every category off
        for it in &mut st.items {
            if matches!(it.class, ItemKind::Cat(_)) {
                it.on = false;
            }
        }
        assert!(matches!(on_key(&mut st, KeyCode::Enter), Action::Ignore));
        assert!(st.selection().is_none());
        // turn one back on > enter starts
        st.items[0].on = true;
        assert!(matches!(on_key(&mut st, KeyCode::Enter), Action::Start));
    }
}
