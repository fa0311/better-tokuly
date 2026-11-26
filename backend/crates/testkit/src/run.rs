//! A test run: one test binary process, and the names of what it creates on
//! the shared infrastructure.
//!
//! Every name testkit gives a resource carries the run's tag, the Unix time
//! the run started and a random id: `tokuly_test_1795702429_ab12cd` (a
//! database), `T1795702429_ab12cd_EVENTS` (a stream),
//! `tokuly-test-1795702429-ab12cd-media` (a bucket). The tag is how testkit
//! knows a name is its own, and how old it is.
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Leftovers of runs that started longer ago than this belong to a run that
/// crashed (or was killed) before it could remove them. No test binary runs
/// this long, so a sweep never takes what a live run still uses.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

const ID_LEN: usize = 6;

pub(crate) fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_secs()
}

#[derive(Clone, Debug)]
pub(crate) struct Run {
    started: u64,
    id: String,
}

impl Run {
    pub(crate) fn new() -> Self {
        Self::started_at(unix_now())
    }

    /// A run that started at `started` (tests use it to fake old runs).
    pub(crate) fn started_at(started: u64) -> Self {
        Self { started, id: tokuly_core::ids::random_base62(ID_LEN).to_ascii_lowercase() }
    }

    /// `<head><started><sep><id>`, the name of one of the run's resources.
    pub(crate) fn name(&self, head: &str, sep: char) -> String {
        format!("{head}{}{sep}{}", self.started, self.id)
    }
}

/// When the run that owns `name` started, if `name` is one testkit gave out
/// with `head` and `sep` ([`Run::name`], optionally followed by
/// `<sep><more>`); `None` for every other name.
pub(crate) fn started_of(name: &str, head: &str, sep: char) -> Option<u64> {
    let (started, rest) = name.strip_prefix(head)?.split_once(sep)?;
    let id = rest.get(..ID_LEN)?;
    let tail = &rest[ID_LEN..];
    let is_tag = started.len() == 10
        && started.bytes().all(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && (tail.is_empty() || tail.starts_with(sep));
    if is_tag { started.parse().ok() } else { None }
}

/// Whether `name` is testkit's and older than [`STALE_AFTER`].
pub(crate) fn is_stale(name: &str, head: &str, sep: char, now: u64) -> bool {
    started_of(name, head, sep).is_some_and(|started| now.saturating_sub(started) > STALE_AFTER.as_secs())
}

static AT_EXIT: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Runs `hook` when the process exits. libtest exits through `exit(3)` both
/// when every test passed and when some panicked, so the hook runs either
/// way; a process that is killed never runs it (the next run's sweep takes
/// over). Only the first hook registered counts.
pub(crate) fn at_exit(hook: impl Fn() + Send + Sync + 'static) {
    if AT_EXIT.set(Box::new(hook)).is_ok() {
        register_at_exit();
    }
}

extern "C" fn run_at_exit_hook() {
    if let Some(hook) = AT_EXIT.get() {
        // Unwinding out of an `extern "C"` function aborts the process.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(hook));
    }
}

#[allow(unsafe_code)]
fn register_at_exit() {
    // SAFETY: `atexit` only stores the pointer to a plain `extern "C" fn()`,
    // which lives as long as the process; the function touches nothing but
    // a `'static` and catches its own panics.
    let status = unsafe { libc::atexit(run_at_exit_hook) };
    if status != 0 {
        eprintln!("testkit: cannot register the clean-up at exit; leftovers are swept by a later run");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_the_start_time() {
        let run = Run::started_at(1_795_702_429);
        let database = run.name("tokuly_test_", '_');
        assert_eq!(started_of(&database, "tokuly_test_", '_'), Some(1_795_702_429));
        assert_eq!(started_of(&format!("{}_EVENTS", run.name("T", '_')), "T", '_'), Some(1_795_702_429));
        assert_eq!(
            started_of(&format!("{}-media", run.name("tokuly-test-", '-')), "tokuly-test-", '-'),
            Some(1_795_702_429)
        );
    }

    #[test]
    fn names_that_are_not_testkit_s_are_never_claimed() {
        for name in [
            "tokuly",
            "tokuly_dev",
            "tokuly_auditadmin",
            "tokuly_test",
            "tokuly_test_1795702429",
            "tokuly_test_1795702429_ab12c",
            "tokuly_test_1795702429_AB12CD",
            "tokuly_test_1795702429_ab12cdx",
            "tokuly_test_179570242_ab12cd",
            "tokuly_test_17957024290_ab12cd",
            "EVENTS",
            "auditadmin_EVENTS",
            "Tfoo_ab12cd_EVENTS",
            "tokuly-media",
            "tokuly-private",
        ] {
            for (head, sep) in [("tokuly_test_", '_'), ("T", '_'), ("tokuly-test-", '-')] {
                assert_eq!(started_of(name, head, sep), None, "{name} claimed as {head}");
            }
        }
    }

    #[test]
    fn only_old_runs_are_stale() {
        let now = 1_795_702_429;
        let old = Run::started_at(now - STALE_AFTER.as_secs() - 1).name("tokuly_test_", '_');
        let recent = Run::started_at(now - 60).name("tokuly_test_", '_');
        assert!(is_stale(&old, "tokuly_test_", '_', now));
        assert!(!is_stale(&recent, "tokuly_test_", '_', now));
        assert!(!is_stale("tokuly_dev", "tokuly_test_", '_', now));
    }
}
