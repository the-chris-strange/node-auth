//! Git-backed inspection of local credential file safety.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Whether Git reports the local `.npmrc` as safely ignored.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GitStatus {
    /// The directory does not belong to a Git repository.
    NotGitRepo,
    /// Git confirms `.npmrc` is ignored and untracked.
    GitRepoIgnored,
    /// Git reports `.npmrc` as tracked or not ignored.
    GitRepoNotIgnored,
    /// Git was unavailable or could not determine the answer.
    Unavailable,
}

/// Ask Git for the repository root containing `start_dir`.
pub fn find_git_repo_root(start_dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start_dir)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(PathBuf::from(String::from_utf8(output.stdout).ok()?.trim()))
}

/// Ask Git whether the local `.npmrc` is ignored and not tracked.
pub fn check_git_status(dir: &Path) -> GitStatus {
    let repo = match Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) if status.code() == Some(128) => false,
        _ => return GitStatus::Unavailable,
    };
    if !repo {
        return GitStatus::NotGitRepo;
    }

    let tracked = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--", ".npmrc"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match tracked {
        Ok(status) if status.success() => return GitStatus::GitRepoNotIgnored,
        Ok(status) if status.code() == Some(1) => {}
        _ => return GitStatus::Unavailable,
    }

    match Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "-q", "--", ".npmrc"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => GitStatus::GitRepoIgnored,
        Ok(status) if status.code() == Some(1) => GitStatus::GitRepoNotIgnored,
        _ => GitStatus::Unavailable,
    }
}

/// Return the Git safety status without emitting terminal output.
pub fn check_local_credential_safety(dir: &Path) -> GitStatus {
    check_git_status(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn git_determines_ignore_status() {
        let dir = tempdir().unwrap();
        assert_eq!(check_git_status(dir.path()), GitStatus::NotGitRepo);
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(check_git_status(dir.path()), GitStatus::GitRepoNotIgnored);
        fs::write(dir.path().join(".gitignore"), ".npmrc\n").unwrap();
        assert_eq!(check_git_status(dir.path()), GitStatus::GitRepoIgnored);
        assert_eq!(
            find_git_repo_root(dir.path()).unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }
}
