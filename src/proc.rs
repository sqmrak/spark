// thread count from /proc/self/stat field 20
fn thread_count() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = stat.rsplit_once(')')?.1;
    // field 20 is num_threads. after ')' the next token is field 3, so
    // the data starts at index 17 (0-based) of the remaining tokens
    rest.split_whitespace().nth(17)?.parse().ok()
}

// guard before every fork. multithreaded fork is unsafe: child inherits
// locked mutexes, first malloc/println/mount deadlocks. live in all profiles
// so a release run that becomes multithreaded does not silently hang
#[inline]
pub(crate) fn assert_fork_safe() {
    assert!(
        thread_count().is_none_or(|n| n <= 1),
        "fork() while multithreaded ({} threads): child may inherit a locked mutex and deadlock",
        thread_count().unwrap_or(0),
    );
}

/// wait for a known child, return raw status
pub(crate) fn wait(pid: i32) -> i32 {
    let mut status = 0;
    // safe: waiting on a child we forked
    unsafe { libc::waitpid(pid, &mut status, 0) };
    status
}

/// exit code from a wait status, -1 if signalled
pub(crate) fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 }
}

/// non-blocking reap of any child, returns pid or 0 (none ready) or -1 (none left)
pub(crate) fn try_reap() -> i32 {
    let mut status = 0;
    // safe: non-blocking wait on any child
    unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_one_thread_when_alone() {
        // the test harness may run tests in parallel, but each runs on its
        // own thread; the count is process-wide, so just assert it parses to
        // a sane positive number
        assert!(thread_count().unwrap() >= 1);
    }

    #[test]
    fn sees_a_spawned_thread() {
        // a started + parked thread means the process-wide count is at least
        // two (this thread plus that one). an absolute floor, not a delta off a
        // baseline the parallel harness can shift between samples
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let h = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().ok();
        });
        started_rx.recv().unwrap();
        assert!(thread_count().unwrap() >= 2);
        drop(release_tx);
        h.join().unwrap();
    }
}
