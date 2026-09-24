//! Path resolution and isolated, atomic configuration-file replacement.

use crate::AuthError;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{Builder, NamedTempFile};

#[cfg(windows)]
mod windows_security {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::iter;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    };
    #[cfg(test)]
    use windows_sys::Win32::Security::Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
        GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SetFileSecurityW, TOKEN_QUERY, TOKEN_USER,
        TokenUser, UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_TEMPORARY, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    #[derive(Clone)]
    pub(super) struct SecurityDescriptor {
        storage: Vec<usize>,
    }

    impl SecurityDescriptor {
        fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
            self.storage.as_ptr().cast_mut().cast()
        }
    }

    struct LocalAllocation(*mut c_void);

    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect()
    }

    fn current_user_sid() -> io::Result<String> {
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = OwnedHandle::from_raw_handle(token as RawHandle);

            let mut required = 0;
            GetTokenInformation(
                token.as_raw_handle() as _,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut required,
            );
            if required == 0 {
                return Err(io::Error::last_os_error());
            }
            let words = (required as usize).div_ceil(size_of::<usize>());
            let mut token_info = vec![0usize; words];
            if GetTokenInformation(
                token.as_raw_handle() as _,
                TokenUser,
                token_info.as_mut_ptr().cast(),
                required,
                &mut required,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let token_user = &*token_info.as_ptr().cast::<TOKEN_USER>();
            let mut sid_string = std::ptr::null_mut();
            if ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) == 0 {
                return Err(io::Error::last_os_error());
            }
            let allocation = LocalAllocation(sid_string.cast());
            let len = (0..).take_while(|&i| *sid_string.add(i) != 0).count();
            let result = String::from_utf16(std::slice::from_raw_parts(sid_string, len))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            drop(allocation);
            result
        }
    }

    fn private_descriptor() -> io::Result<LocalAllocation> {
        let sddl = format!("D:P(A;;FA;;;{})", current_user_sid()?);
        let encoded = sddl.encode_utf16().chain(iter::once(0)).collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                encoded.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(LocalAllocation(descriptor))
    }

    /// Create a file whose DACL grants access only to the current user.
    pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
        let descriptor = private_descriptor()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let path = wide(path);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | DELETE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_TEMPORARY,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            // The returned handle is uniquely owned and becomes owned by File.
            Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
        }
    }

    /// Capture the target DACL, including whether inheritance is protected.
    pub(super) fn read_dacl(path: &Path) -> io::Result<SecurityDescriptor> {
        let path = wide(path);
        let mut required = 0;
        unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut required,
            );
        }
        if required == 0 {
            return Err(io::Error::last_os_error());
        }
        let words = (required as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; words];
        if unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                storage.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(SecurityDescriptor { storage })
    }

    /// Apply a captured DACL without changing its inheritance protection state.
    pub(super) fn apply_dacl(path: &Path, descriptor: &SecurityDescriptor) -> io::Result<()> {
        let mut control = 0;
        let mut revision = 0;
        if unsafe { GetSecurityDescriptorControl(descriptor.as_ptr(), &mut control, &mut revision) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let protection = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let path = wide(path);
        if unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | protection,
                descriptor.as_ptr(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    unsafe fn descriptor_to_sddl(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<String> {
        let mut text = std::ptr::null_mut();
        if unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let allocation = LocalAllocation(text.cast());
        let len = (0..).take_while(|&i| unsafe { *text.add(i) != 0 }).count();
        let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) })
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        drop(allocation);
        result
    }

    #[cfg(test)]
    pub(super) fn dacl_sddl(path: &Path) -> io::Result<String> {
        let descriptor = read_dacl(path)?;
        unsafe { descriptor_to_sddl(descriptor.as_ptr()) }
    }

    #[cfg(test)]
    pub(super) fn private_dacl_sddl() -> io::Result<String> {
        let descriptor = private_descriptor()?;
        unsafe { descriptor_to_sddl(descriptor.0) }
    }
}

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
    #[cfg(windows)]
    dacl: Option<windows_security::SecurityDescriptor>,
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
                    #[cfg(windows)]
                    dacl: Some(windows_security::read_dacl(&path)?),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot {
                    contents: None,
                    permissions: None,
                    #[cfg(windows)]
                    dacl: None,
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
        #[cfg(windows)]
        let mut temp = Builder::new()
            .prefix(".node-auth-")
            .make_in(parent, windows_security::create_private_file)?;
        #[cfg(not(windows))]
        let mut temp = Builder::new().prefix(".node-auth-").tempfile_in(parent)?;
        #[cfg(windows)]
        if let Some(dacl) = &snapshot.dacl {
            windows_security::apply_dacl(temp.path(), dacl)?;
        }
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

    #[cfg(windows)]
    #[test]
    fn new_file_is_private_and_existing_dacl_is_preserved() {
        let dir = tempdir().unwrap();
        let new_path = dir.path().join("new.npmrc");
        let mut tx = FileTransaction::new(std::slice::from_ref(&new_path)).unwrap();
        tx.stage(&resolve(&new_path).unwrap(), "token").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            windows_security::dacl_sddl(&new_path).unwrap(),
            windows_security::private_dacl_sddl().unwrap()
        );

        let existing_path = dir.path().join("existing.npmrc");
        fs::write(&existing_path, "old").unwrap();
        let original_dacl = windows_security::dacl_sddl(&existing_path).unwrap();
        let mut tx = FileTransaction::new(std::slice::from_ref(&existing_path)).unwrap();
        tx.stage(&resolve(&existing_path).unwrap(), "next").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            windows_security::dacl_sddl(&existing_path).unwrap(),
            original_dacl
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
