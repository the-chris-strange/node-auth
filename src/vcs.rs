//! Version control system inspection and Git safety checks.
//!
//! Provides utilities to verify whether local `.npmrc` files containing authentication
//! credentials are safe from being inadvertently committed to Git repositories.

use std::path::{Path, PathBuf};
use std::process::Command;
use colored::Colorize;

/// Git repository status indicating whether `.npmrc` is tracked or ignored.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GitStatus {
    /// Directory is not part of a Git repository.
    NotGitRepo,
    /// Directory is in a Git repository and `.npmrc` is ignored by `.gitignore`.
    GitRepoIgnored,
    /// Directory is in a Git repository but `.npmrc` is NOT ignored by `.gitignore`.
    GitRepoNotIgnored,
}

/// Walk up from `start_dir` to find the root directory of a git repository (where `.git` exists).
pub fn find_git_repo_root(start_dir: &Path) -> Option<PathBuf> {
    let canonical = start_dir.canonicalize().unwrap_or_else(|_| start_dir.to_path_buf());
    let mut curr = canonical.as_path();
    loop {
        let git_path = curr.join(".git");
        if git_path.exists() {
            return Some(curr.to_path_buf());
        }
        match curr.parent() {
            Some(parent) => curr = parent,
            None => break,
        }
    }
    None
}

/// Simple fallback parser to check if `.npmrc` is matched in a `.gitignore` content.
pub fn is_npmrc_in_gitignore_content(content: &str) -> bool {
    let mut ignored = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('!') {
            let pattern = line.trim_start_matches('!').trim();
            if pattern == ".npmrc" || pattern == "/.npmrc" || pattern == "*.npmrc" {
                ignored = false;
            }
        } else {
            let pattern = line.trim_start_matches('/');
            if pattern == ".npmrc" || pattern == "*.npmrc" || pattern == ".*" {
                ignored = true;
            }
        }
    }
    ignored
}

/// Checks whether `.npmrc` is ignored in the git repository at or above `dir`.
pub fn check_git_status(dir: &Path) -> GitStatus {
    let repo_root = match find_git_repo_root(dir) {
        Some(root) => root,
        None => {
            log::debug!("Directory {} is not a git repository", dir.display());
            return GitStatus::NotGitRepo;
        }
    };

    // Attempt 1: Try running `git check-ignore -q .npmrc`
    if let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("check-ignore")
        .arg("-q")
        .arg(".npmrc")
        .stderr(std::process::Stdio::null())
        .status()
    {
        if output.success() {
            log::debug!(".npmrc is ignored according to git check-ignore in {}", dir.display());
            return GitStatus::GitRepoIgnored;
        } else if output.code() == Some(1) {
            log::debug!(".npmrc is NOT ignored according to git check-ignore in {}", dir.display());
            return GitStatus::GitRepoNotIgnored;
        }
    }

    // Attempt 2: Fallback to reading .gitignore files if git command is unavailable or failed
    let mut curr: Option<&Path> = Some(dir);
    while let Some(d) = curr {
        let gitignore = d.join(".gitignore");
        if gitignore.exists() {
            if let Ok(content) = std::fs::read_to_string(&gitignore) {
                if is_npmrc_in_gitignore_content(&content) {
                    log::debug!(".npmrc is matched in {}", gitignore.display());
                    return GitStatus::GitRepoIgnored;
                }
            }
        }
        if d == repo_root {
            break;
        }
        curr = d.parent();
    }

    log::debug!(".npmrc is not ignored in git repository {}", repo_root.display());
    GitStatus::GitRepoNotIgnored
}

/// Checks safety when writing credentials locally, printing appropriate warnings.
pub fn check_local_credential_safety(dir: &Path) -> GitStatus {
    let status = check_git_status(dir);
    match status {
        GitStatus::GitRepoNotIgnored => {
            log::warn!("Writing credentials to local .npmrc, but '.npmrc' is not ignored in .gitignore!");
            eprintln!(
                "{}",
                "⚠️  WARNING: Writing credentials to local .npmrc, but '.npmrc' is not ignored in .gitignore!"
                    .yellow()
                    .bold()
            );
            eprintln!(
                "{}",
                "   Make sure to add '.npmrc' to your .gitignore to avoid committing secret tokens to version control.\n"
                    .yellow()
            );
        }
        GitStatus::NotGitRepo => {
            log::warn!("Writing credentials to local .npmrc in a directory that is not a Git repository.");
            eprintln!(
                "{}",
                "⚠️  WARNING: Writing credentials to local .npmrc in a directory that is not a Git repository."
                    .yellow()
                    .bold()
            );
            eprintln!(
                "{}",
                "   Ensure this file is never committed or pushed to version control.\n"
                    .yellow()
            );
        }
        GitStatus::GitRepoIgnored => {
            // Safely ignored, no warning needed
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_is_npmrc_in_gitignore() {
        assert!(is_npmrc_in_gitignore_content(".npmrc\n"));
        assert!(is_npmrc_in_gitignore_content("node_modules\n.npmrc\ndist/\n"));
        assert!(is_npmrc_in_gitignore_content("*.npmrc\n"));
        assert!(is_npmrc_in_gitignore_content("/.npmrc\n"));
        assert!(is_npmrc_in_gitignore_content(".*\n"));

        // Negation
        assert!(!is_npmrc_in_gitignore_content(".npmrc\n!.npmrc\n"));

        // Not matching
        assert!(!is_npmrc_in_gitignore_content("node_modules\npackage-lock.json\n"));
        assert!(!is_npmrc_in_gitignore_content("# .npmrc is commented out\n"));
    }

    #[test]
    fn test_find_git_repo_root() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let git_dir = root.join(".git");
        fs::create_dir(&git_dir).unwrap();

        let nested = root.join("packages").join("app");
        fs::create_dir_all(&nested).unwrap();

        let found = find_git_repo_root(&nested).unwrap();
        assert_eq!(found.canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn test_check_git_status_not_git_repo() {
        let dir = tempdir().unwrap();
        assert_eq!(check_git_status(dir.path()), GitStatus::NotGitRepo);
    }

    #[test]
    fn test_check_git_status_ignored_and_not_ignored() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let git_dir = root.join(".git");
        fs::create_dir(&git_dir).unwrap();

        // No .gitignore -> Not ignored
        assert_eq!(check_git_status(root), GitStatus::GitRepoNotIgnored);

        // Add .gitignore with .npmrc
        let gitignore = root.join(".gitignore");
        fs::write(&gitignore, "node_modules/\n.npmrc\n").unwrap();

        assert_eq!(check_git_status(root), GitStatus::GitRepoIgnored);
    }
}