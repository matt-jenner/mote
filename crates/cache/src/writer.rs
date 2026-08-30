use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use photo_catalog::Catalog;

use crate::CacheError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheWrite {
    pub relative_path: PathBuf,
    pub size_bytes: u64,
    pub reused: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheReconcileReport {
    pub partial_files_removed: u64,
    pub missing_rows_removed: u64,
}

#[derive(Clone, Debug)]
pub struct CacheWriter {
    root: PathBuf,
}

impl CacheWriter {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub fn new(root: &Path) -> Result<Self, CacheError> {
        std::fs::create_dir_all(root)?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn write_atomic<F>(
        &self,
        relative_path: PathBuf,
        write: F,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        let final_path = self.resolve_checked(&relative_path)?;
        if is_regular_file(&final_path)? {
            return Ok(CacheWrite {
                relative_path,
                size_bytes: final_path.metadata()?.len(),
                reused: true,
            });
        }

        let parent = final_path.parent().ok_or(CacheError::PathEscape)?;
        self.create_safe_directories(parent)?;
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CacheError::PathEscape)?;
        let partial_path = parent.join(format!("{file_name}.partial-{}", uuid::Uuid::new_v4()));
        let mut partial = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial_path)?;

        if let Err(error) = write(&mut partial) {
            drop(partial);
            let _ = std::fs::remove_file(&partial_path);
            return Err(CacheError::Write(error));
        }
        if let Err(error) = partial.flush().and_then(|()| partial.sync_all()) {
            drop(partial);
            let _ = std::fs::remove_file(&partial_path);
            return Err(CacheError::Write(error));
        }
        let size_bytes = partial.metadata()?.len();
        drop(partial);

        match std::fs::hard_link(&partial_path, &final_path) {
            Ok(()) => {
                std::fs::remove_file(&partial_path)?;
                Ok(CacheWrite {
                    relative_path,
                    size_bytes,
                    reused: false,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&partial_path)?;
                self.resolve_checked(&relative_path)?;
                if !is_regular_file(&final_path)? {
                    return Err(CacheError::PathEscape);
                }
                Ok(CacheWrite {
                    relative_path,
                    size_bytes: final_path.metadata()?.len(),
                    reused: true,
                })
            }
            Err(error) => {
                let _ = std::fs::remove_file(&partial_path);
                Err(CacheError::Write(error))
            }
        }
    }

    pub fn reconcile_catalog(
        &self,
        catalog: &mut Catalog,
    ) -> Result<CacheReconcileReport, CacheError> {
        let mut partial_files_removed = 0_u64;
        // Named desktop profiles own independent cache roots below this namespace.
        let profiles_namespace = self.root.join("profiles");
        for entry in walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.path() != profiles_namespace.as_path())
        {
            let entry = entry.map_err(|error| {
                CacheError::Io(
                    error
                        .into_io_error()
                        .unwrap_or_else(|| std::io::Error::other("could not walk cache directory")),
                )
            })?;
            if entry.file_type().is_file()
                && entry.file_name().to_string_lossy().contains(".partial-")
            {
                std::fs::remove_file(entry.path())?;
                partial_files_removed += 1;
            }
        }

        let missing = catalog
            .all_derivatives()?
            .into_iter()
            .filter_map(|record| {
                self.resolve_checked(&record.relative_cache_path)
                    .and_then(|path| is_regular_file(&path))
                    .map(|present| (!present).then_some(record.id))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let missing_rows_removed = catalog.delete_derivatives(&missing)?;
        Ok(CacheReconcileReport {
            partial_files_removed,
            missing_rows_removed: u64::try_from(missing_rows_removed)
                .map_err(|_| CacheError::SizeOutOfRange)?,
        })
    }

    pub fn read_checked(&self, relative_path: &Path) -> Result<Vec<u8>, CacheError> {
        let mut file = self.open_checked(relative_path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)?;
        Ok(bytes)
    }

    /// Opens a regular file beneath the managed cache root after checking every
    /// path component for traversal or symlink escapes.
    pub fn open_checked(&self, relative_path: &Path) -> Result<File, CacheError> {
        let path = self.resolve_checked(relative_path)?;
        if !is_regular_file(&path)? {
            return Err(CacheError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cache file does not exist",
            )));
        }
        let file = OpenOptions::new().read(true).open(path)?;
        // The lexical/symlink checks above can race a rename. Validate the
        // descriptor after opening as well, so a swap cannot redirect the
        // returned handle outside the managed root.
        if !file.metadata()?.is_file() || !descriptor_is_contained(&file, &self.root) {
            return Err(CacheError::PathEscape);
        }
        Ok(file)
    }

    /// Removes a managed cache file after applying the same containment checks
    /// used for reads. Missing files are treated as already removed.
    pub fn remove_checked(&self, relative_path: &Path) -> Result<(), CacheError> {
        let path = self.resolve_checked(relative_path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn resolve_checked(&self, relative_path: &Path) -> Result<PathBuf, CacheError> {
        validate_relative(relative_path)?;
        let final_path = self.root.join(relative_path);
        let mut current = self.root.clone();
        let component_count = relative_path.components().count();
        for (index, component) in relative_path.components().enumerate() {
            current.push(component.as_os_str());
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(CacheError::PathEscape);
                }
                Ok(metadata) if index + 1 < component_count && !metadata.is_dir() => {
                    return Err(CacheError::PathEscape);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(final_path);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(final_path)
    }

    pub(crate) fn remove_relative_file(&self, relative_path: &Path) -> Result<(), CacheError> {
        let path = self.resolve_checked(relative_path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn create_safe_directories(&self, parent: &Path) -> Result<(), CacheError> {
        let relative = parent
            .strip_prefix(&self.root)
            .map_err(|_| CacheError::PathEscape)?;
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(CacheError::PathEscape);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            let metadata = std::fs::symlink_metadata(&current)?;
                            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                                return Err(CacheError::PathEscape);
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn descriptor_is_contained(file: &File, root: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        let mut buffer = [0_i8; libc::PATH_MAX as usize];
        let result = unsafe { libc::fcntl(fd, libc::F_GETPATH, buffer.as_mut_ptr()) };
        if result == -1 {
            return false;
        }
        let path = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
        return path
            .to_str()
            .ok()
            .is_some_and(|path| Path::new(path).starts_with(root));
    }
    #[cfg(not(target_os = "macos"))]
    {
        [format!("/proc/self/fd/{fd}"), format!("/dev/fd/{fd}")]
            .into_iter()
            .filter_map(|path| std::fs::canonicalize(path).ok())
            .any(|path| path.starts_with(root))
    }
}

#[cfg(windows)]
fn descriptor_is_contained(file: &File, root: &Path) -> bool {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            hFile: *mut std::ffi::c_void,
            lpszFilePath: *mut u16,
            cchFilePath: u32,
            dwFlags: u32,
        ) -> u32;
    }

    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as *mut std::ffi::c_void,
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).unwrap_or(u32::MAX),
            0,
        )
    };
    if length == 0
        || usize::try_from(length)
            .ok()
            .is_none_or(|length| length >= buffer.len())
    {
        return false;
    }
    let path = std::ffi::OsString::from_wide(&buffer[..length as usize]);
    Path::new(&path).starts_with(root)
}

fn is_regular_file(path: &Path) -> Result<bool, CacheError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn validate_relative(path: &Path) -> Result<(), CacheError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CacheError::PathEscape);
    }
    Ok(())
}
