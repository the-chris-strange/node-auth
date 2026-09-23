//! Path resolution and isolated, atomic configuration-file replacement.

use crate::AuthError;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{Builder, NamedTempFile};

/// Resolve an existing path through symlinks, or resolve its nearest existing ancestor.
pub fn resolve(path: &Path) -> Result<PathBuf, AuthError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    match fs::symlink_metadata(&absolute) {
        Ok(_) => Ok(absolute.canonicalize()?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = absolute.parent().ok_or_else(|| {
                AuthError::Config(format!("Path has no parent: {}", absolute.display()))
            })?;
            let name = absolute.file_name().ok_or_else(|| {
                AuthError::Config(format!("Path has no file name: {}", absolute.display()))
            })?;
            Ok(resolve(parent)?.join(name))
        }
        Err(e) => Err(e.into()),
    }
}

#[derive(Clone)]
struct Snapshot {
    contents: Option<String>,
    permissions: Option<Permissions>,
}

/// Holds cooperative locks from the initial reads through all replacements.
///
/// Every output is staged before the first rename. Each rename is atomic, although
/// replacing several different files cannot be a single filesystem transaction.
pub struct FileTransaction {
    snapshots: BTreeMap<PathBuf, Snapshot>,
    staged: BTreeMap<PathBuf, NamedTempFile>,
    _locks: Vec<File>,
}

impl FileTransaction {
    /// Resolve and lock every path in sorted order, then read their contents.
    pub fn new(paths: &[PathBuf]) -> Result<Self, AuthError> {
        let mut resolved = paths
            .iter()
            .map(|p| resolve(p))
            .collect::<Result<Vec<_>, _>>()?;
        resolved.sort();
        resolved.dedup();

        let mut locks = Vec::with_capacity(resolved.len());
        for path in &resolved {
            let parent = path.parent().ok_or_else(|| {
                AuthError::Config(format!("Path has no parent: {}", path.display()))
            })?;
            fs::create_dir_all(parent)?;
            let file_name = path.file_name().ok_or_else(|| {
                AuthError::Config(format!("Path has no file name: {}", path.display()))
            })?;
            let mut lock_name = std::ffi::OsString::new();
            if !file_name.to_string_lossy().starts_with('.') {
                lock_name.push(".");
            }
            lock_name.push(file_name);
            lock_name.push(".node-auth.lock");
            let lock_path = parent.join(lock_name);
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let lock = options.open(lock_path)?;
            lock.lock()?;
            locks.push(lock);
        }

        let mut snapshots = BTreeMap::new();
        for path in resolved {
            let snapshot = match fs::read_to_string(&path) {
                Ok(contents) => Snapshot {
                    contents: Some(contents),
                    permissions: Some(fs::metadata(&path)?.permissions()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot {
                    contents: None,
                    permissions: None,
                },
                Err(e) => return Err(e.into()),
            };
            snapshots.insert(path, snapshot);
        }
        Ok(Self {
            snapshots,
            staged: BTreeMap::new(),
            _locks: locks,
        })
    }

    /// Return the content read while the transaction held all requested locks.
    pub fn contents(&self, path: &Path) -> Result<&str, AuthError> {
        self.snapshots
            .get(path)
            .map(|s| s.contents.as_deref().unwrap_or(""))
            .ok_or_else(|| {
                AuthError::Config(format!(
                    "Path was not included in the transaction: {}",
                    path.display()
                ))
            })
    }

    /// Whether the path existed when the transaction read it.
    pub fn existed(&self, path: &Path) -> Result<bool, AuthError> {
        self.snapshots
            .get(path)
            .map(|s| s.contents.is_some())
            .ok_or_else(|| {
                AuthError::Config(format!(
                    "Path was not included in the transaction: {}",
                    path.display()
                ))
            })
    }

    /// Whether an existing credential file was readable by group or other users at read time.
    pub fn is_broadly_readable(&self, path: &Path) -> Result<bool, AuthError> {
        let snapshot = self.snapshots.get(path).ok_or_else(|| {
            AuthError::Config(format!(
                "Path was not included in the transaction: {}",
                path.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(snapshot
                .permissions
                .as_ref()
                .is_some_and(|p| p.mode() & 0o044 != 0))
        }
        #[cfg(not(unix))]
        {
            let _ = snapshot;
            Ok(false)
        }
    }

    /// Stage an output beside its destination without replacing that destination yet.
    pub fn stage(&mut self, path: &Path, content: &str) -> Result<(), AuthError> {
        let snapshot = self.snapshots.get(path).ok_or_else(|| {
            AuthError::Config(format!(
                "Path was not included in the transaction: {}",
                path.display()
            ))
        })?;
        if self.staged.contains_key(path) {
            return Err(AuthError::Config(format!(
                "File staged twice: {}",
                path.display()
            )));
        }
        let parent = path
            .parent()
            .ok_or_else(|| AuthError::Config(format!("Path has no parent: {}", path.display())))?;
        let mut temp = Builder::new().prefix(".node-auth-").tempfile_in(parent)?;
        temp.write_all(content.as_bytes())?;
        if let Some(permissions) = &snapshot.permissions {
            temp.as_file().set_permissions(permissions.clone())?;
        }
        temp.as_file().sync_all()?;
        self.staged.insert(path.to_path_buf(), temp);
        Ok(())
    }

    /// Atomically replace each staged file, rejecting changes made since the initial read.
    pub fn commit(mut self) -> Result<Vec<PathBuf>, AuthError> {
        for (path, snapshot) in &self.snapshots {
            let current = match fs::read_to_string(path) {
                Ok(contents) => Some(contents),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if current != snapshot.contents {
                return Err(AuthError::Config(format!(
                    "Configuration changed during update: {}",
                    path.display()
                )));
            }
        }

        let mut written = Vec::new();
        for (path, temp) in std::mem::take(&mut self.staged) {
            temp.persist(&path).map_err(|e| AuthError::Io(e.error))?;
            #[cfg(unix)]
            File::open(path.parent().unwrap())?.sync_all()?;
            written.push(path);
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_aliases_and_replaces_atomically() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".npmrc");
        fs::write(&path, "old").unwrap();
        let alias = dir.path().join("./.npmrc");
        assert_eq!(resolve(&path).unwrap(), resolve(&alias).unwrap());
        let mut tx = FileTransaction::new(&[path.clone(), alias]).unwrap();
        let resolved = resolve(&path).unwrap();
        assert_eq!(tx.contents(&resolved).unwrap(), "old");
        tx.stage(&resolved, "new").unwrap();
        tx.commit().unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
    }

    #[cfg(unix)]
    #[test]
    fn new_file_is_private_and_existing_mode_is_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let new_path = dir.path().join("new.npmrc");
        let mut tx = FileTransaction::new(std::slice::from_ref(&new_path)).unwrap();
        tx.stage(&resolve(&new_path).unwrap(), "token").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            fs::metadata(&new_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(&new_path, Permissions::from_mode(0o640)).unwrap();
        let mut tx = FileTransaction::new(std::slice::from_ref(&new_path)).unwrap();
        tx.stage(&resolve(&new_path).unwrap(), "next").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            fs::metadata(&new_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn transactions_serialize_reads_and_writes() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = tempdir().unwrap();
        let path = dir.path().join("shared.npmrc");
        fs::write(&path, "old").unwrap();
        let mut first = FileTransaction::new(std::slice::from_ref(&path)).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (content_tx, content_rx) = mpsc::channel();
        let second_path = path.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let second = FileTransaction::new(std::slice::from_ref(&second_path)).unwrap();
            content_tx
                .send(
                    second
                        .contents(&resolve(&second_path).unwrap())
                        .unwrap()
                        .to_string(),
                )
                .unwrap();
        });
        started_rx.recv().unwrap();
        assert!(content_rx.recv_timeout(Duration::from_millis(50)).is_err());
        first.stage(&resolve(&path).unwrap(), "new").unwrap();
        first.commit().unwrap();
        assert_eq!(
            content_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "new"
        );
        worker.join().unwrap();
    }
}
