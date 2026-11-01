//! Shared classification of journal-open failures for the CLI's JSON error categories.
//!
//! The journal store wraps every redb open error as `redb open failed: …`. redb holds an
//! exclusive file lock on `journal.redb` while it is open (the TUI, a running agent, or a second
//! concurrent `akmon --task`), and a lock conflict surfaces as `Database already open. Cannot
//! acquire lock.`. That is not a missing journal, so it gets its own `journal_locked` category.

/// Stable JSON error category for a journal held open by another process.
pub(crate) const JOURNAL_LOCKED: &str = "journal_locked";

/// Stable JSON error category for a journal that does not exist or cannot be opened.
pub(crate) const JOURNAL_NOT_FOUND: &str = "journal_not_found";

/// Whether `msg` describes redb's single-writer lock being held by someone else.
pub(crate) fn is_journal_locked(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("database already open") || lower.contains("cannot acquire lock")
}

/// Category for a journal-open failure, or `None` when `msg` is not a journal-open failure.
///
/// The lock check runs first because a locked journal's message also contains
/// `redb open failed`.
pub(crate) fn journal_open_category(msg: &str) -> Option<&'static str> {
    if is_journal_locked(msg) {
        return Some(JOURNAL_LOCKED);
    }
    let lower = msg.to_ascii_lowercase();
    if lower.contains("redb open failed") || lower.contains("no such file or directory") {
        Some(JOURNAL_NOT_FOUND)
    } else {
        None
    }
}

/// Category for a failure to open or create a writable journal: `journal_locked` for a lock
/// conflict, otherwise `journal_access`.
pub(crate) fn journal_access_category(msg: &str) -> &'static str {
    if is_journal_locked(msg) {
        JOURNAL_LOCKED
    } else {
        "journal_access"
    }
}

/// Appends an operator hint to `error` when it is a journal lock conflict.
pub(crate) fn with_journal_lock_hint(error: String) -> String {
    if is_journal_locked(&error) {
        format!(
            "{error}\nThe journal is single-writer and another Akmon process (the TUI or a \
running agent) has it open; wait for it to finish, then retry."
        )
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCKED: &str = "cannot open journal /j for session x: session failed: verification \
failed: redb open failed: Database already open. Cannot acquire lock.";

    #[test]
    fn locked_journal_is_journal_locked_not_not_found() {
        assert_eq!(journal_open_category(LOCKED), Some(JOURNAL_LOCKED));
    }

    #[test]
    fn missing_journal_is_journal_not_found() {
        assert_eq!(
            journal_open_category("redb open failed: No such file or directory (os error 2)"),
            Some(JOURNAL_NOT_FOUND)
        );
        assert_eq!(journal_open_category("session not found"), None);
    }

    #[test]
    fn access_category_distinguishes_lock_conflicts() {
        assert_eq!(journal_access_category(LOCKED), JOURNAL_LOCKED);
        assert_eq!(
            journal_access_category("cannot open journal store: permission denied"),
            "journal_access"
        );
    }

    #[test]
    fn lock_hint_only_added_for_lock_conflicts() {
        assert!(with_journal_lock_hint(format!("journal: {LOCKED}")).contains("single-writer"));
        assert_eq!(
            with_journal_lock_hint("journal: other".into()),
            "journal: other"
        );
    }
}
