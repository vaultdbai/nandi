//! For tests: every log line of the test process, kept so a test can show
//! a secret never reached one. The logger is global and installed once;
//! every test's lines pass through it.

use std::sync::{Mutex, Once};

/// Lines kept, newest last; the oldest go first past this many
const MAX_LINES: usize = 50_000;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Keeper;

impl log::Log for Keeper {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let mut lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() >= MAX_LINES {
            lines.remove(0);
        }
        lines.push(format!("{} {}", record.target(), record.args()));
    }

    fn flush(&self) {}
}

static KEEPER: Keeper = Keeper;

/// Keeps every log line from now on, at every level. `false` when another
/// logger was installed first (the lines can't be watched then).
pub fn watch() -> bool {
    INSTALL.call_once(|| {
        if log::set_logger(&KEEPER).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
            INSTALLED.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    INSTALLED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Whether any line kept so far contains `needle`
pub fn contains(needle: &str) -> bool {
    LINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|line| line.contains(needle))
}

mod tests {
    #[test]
    fn keeps_what_is_logged() {
        if super::watch() {
            log::warn!("test_log marker 5521");
            assert!(super::contains("test_log marker 5521"));
        }
    }
}
