// spark entry point. `spark` opens the selection menu and runs the chosen
// tests; `spark -v` prints the version. on a terminal it uses the tui;
// piped, it runs every category the user can and prints [tag] lines

use spark::log::TextReporter;
use spark::ui::{Privileges, Tui};
use std::io::IsTerminal;

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(a) = args.next() {
        if a == "-v" || a == "--version" {
            println!("spark {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        eprintln!("[err] unknown argument: {a}");
        eprintln!("[tip] run: spark   (or spark -v)");
        std::process::exit(2);
    }

    if !std::process::Command::new("which").arg("mkfs.erofs").output().map(|o| o.status.success()).unwrap_or(false) {
        eprintln!("[wrn] erofs-utils not found, mkfs.erofs tests will skip");
    }

    let privs = Privileges::detect();

    if std::io::stdout().is_terminal() {
        let mut tui = Tui::new();
        if let Some(sel) = tui.select(privs) {
            let tests = spark::tests::selected(&sel.cats, &sel.libcs);
            let outcome = spark::run(&mut tui, tests);
            tui.summary();
            tui.finish();
            if !outcome.is_ok() {
                std::process::exit(1);
            }
        }
    } else {
        let mut reporter = TextReporter;
        if !privs.root {
            eprintln!("[wrn] not running as root, privileged tests may be skipped");
        }
        let outcome = spark::run_all(&mut reporter);
        let exit = if outcome.is_ok() { 0 } else { 1 };
        std::process::exit(exit);
    }
}
