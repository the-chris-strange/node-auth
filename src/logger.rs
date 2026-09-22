//! Lightweight terminal logging implementation with color support.

use std::sync::atomic::{AtomicU8, Ordering};
use colored::Colorize;
use log::{Level, LevelFilter, Log, Metadata, Record};

/// Standard output stream destination for log messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogTarget {
    /// Emit logs to standard output (`stdout`).
    Stdout = 0,
    /// Emit logs to standard error (`stderr`).
    Stderr = 1,
}

static LOG_TARGET: AtomicU8 = AtomicU8::new(LogTarget::Stdout as u8);

/// Minimalist logger that formats records with ANSI color codes.
pub struct SimpleLogger;

static LOGGER: SimpleLogger = SimpleLogger;

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let formatted = format_record(record);
            match get_target() {
                LogTarget::Stdout => println!("{formatted}"),
                LogTarget::Stderr => eprintln!("{formatted}"),
            }
        }
    }

    fn flush(&self) {}
}

/// Formats a log record with colored level indicator.
pub fn format_record(record: &Record) -> String {
    let level_str = match record.level() {
        Level::Error => "[ERROR]".red().bold().to_string(),
        Level::Warn => "[WARN]".yellow().bold().to_string(),
        Level::Info => "[INFO]".green().bold().to_string(),
        Level::Debug => "[DEBUG]".cyan().bold().to_string(),
        Level::Trace => "[TRACE]".magenta().bold().to_string(),
    };
    format!("{level_str} {}", record.args())
}

/// Sets the current output target for log records (stdout vs stderr).
pub fn set_target(target: LogTarget) {
    LOG_TARGET.store(target as u8, Ordering::SeqCst);
}

/// Gets the current output target for log records.
pub fn get_target() -> LogTarget {
    match LOG_TARGET.load(Ordering::SeqCst) {
        0 => LogTarget::Stdout,
        _ => LogTarget::Stderr,
    }
}

/// Sets whether verbose (debug-level) logging is enabled.
pub fn set_verbose(verbose: bool) {
    if verbose {
        log::set_max_level(LevelFilter::Debug);
    } else {
        log::set_max_level(LevelFilter::Warn);
    }
}

/// Initializes the global logger.
///
/// - `verbose`: If true, enables debug logging (equivalent to `-v` or `--verbose`).
/// - `to_stdout`: If true, logs go to stdout; otherwise to stderr.
pub fn init(verbose: bool, to_stdout: bool) {
    set_target(if to_stdout {
        LogTarget::Stdout
    } else {
        LogTarget::Stderr
    });
    set_verbose(verbose);

    // set_logger will fail if already initialized (e.g. across multiple tests); ignore the error
    let _ = log::set_logger(&LOGGER);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_switching() {
        set_target(LogTarget::Stdout);
        assert_eq!(get_target(), LogTarget::Stdout);

        set_target(LogTarget::Stderr);
        assert_eq!(get_target(), LogTarget::Stderr);

        // Reset to default
        set_target(LogTarget::Stdout);
    }

    #[test]
    fn test_set_verbose_sets_max_level() {
        set_verbose(true);
        assert_eq!(log::max_level(), LevelFilter::Debug);

        set_verbose(false);
        assert_eq!(log::max_level(), LevelFilter::Warn);
    }

    #[test]
    fn test_format_record() {
        let record = Record::builder()
            .args(format_args!("test message"))
            .level(Level::Debug)
            .target("test")
            .build();

        let formatted = format_record(&record);
        assert!(formatted.contains("[DEBUG]"));
        assert!(formatted.contains("test message"));
    }
}