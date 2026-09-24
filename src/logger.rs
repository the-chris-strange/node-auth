//! CLI-only logging and presentation. Library operations never initialize this logger.

use crate::vcs::GitStatus;
use crate::{AuthError, RunOutcome};
use colored::Colorize;
use log::{Level, LevelFilter, Log, Metadata, Record};

struct CliLogger;
static LOGGER: CliLogger = CliLogger;

impl Log for CliLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}", level_label(record.level()), record.args());
        }
    }

    fn flush(&self) {}
}

fn level_label(level: Level) -> String {
    match level {
        Level::Error => "[ERROR]".red().bold().to_string(),
        Level::Warn => "[WARN]".yellow().bold().to_string(),
        Level::Info => "[INFO]".green().bold().to_string(),
        Level::Debug => "[DEBUG]".cyan().bold().to_string(),
        Level::Trace => "[TRACE]".magenta().bold().to_string(),
    }
}

/// Initialize logging for a CLI invocation. All diagnostic output goes to stderr.
pub fn init(verbose: bool) {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    });
}

/// Present a successful library result to a CLI user.
pub fn present_outcome(outcome: RunOutcome) {
    match outcome {
        RunOutcome::Token(token) => println!("{token}"),
        RunOutcome::Updated { paths, git_status } => {
            if let Some(status) = git_status {
                match status {
                    GitStatus::GitRepoNotIgnored => eprintln!(
                        "{} Local .npmrc is tracked or not ignored by Git; it may expose credentials if committed.",
                        "Warning:".yellow().bold()
                    ),
                    GitStatus::NotGitRepo => eprintln!(
                        "{} Local .npmrc is outside a Git repository; keep credentials out of version control.",
                        "Warning:".yellow().bold()
                    ),
                    GitStatus::Unavailable => eprintln!(
                        "{} Could not verify Git ignore status for local .npmrc.",
                        "Warning:".yellow().bold()
                    ),
                    GitStatus::GitRepoIgnored => {}
                }
            }
            if !paths.is_empty() {
                println!(
                    "{} Updated {} configuration file(s).",
                    "Success!".green().bold(),
                    paths.len()
                );
            }
        }
    }
}

/// Present one CLI error on stderr.
pub fn present_error(error: &AuthError) {
    eprintln!("{} {error}", "Error:".red().bold());
}
