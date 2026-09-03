use std::fs::File;
#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
#[cfg(any(test, debug_assertions))]
use std::sync::atomic::{AtomicBool, Ordering};

use photo_catalog::Catalog;

use crate::CacheError;

#[cfg(any(test, debug_assertions))]
type TestHook = std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>>;

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

#[derive(Clone)]
pub struct CacheWriter {
    root: PathBuf,
    #[cfg(all(any(test, debug_assertions), unix))]
    path_race_test_hook: TestHook,
    #[cfg(any(test, debug_assertions))]
    replace_failure_test_hook: std::sync::Arc<AtomicBool>,
    #[cfg(any(test, debug_assertions))]
    cleanup_failure_test_hook: std::sync::Arc<AtomicBool>,
    #[cfg(any(test, debug_assertions))]
    replacement_test_hook: TestHook,
}

impl std::fmt::Debug for CacheWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CacheWriter")
            .field("root", &self.root)
            .finish()
    }
}

impl CacheWriter {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub fn new(root: &Path) -> Result<Self, CacheError> {
        std::fs::create_dir_all(root)?;
        Ok(Self {
            root: root.canonicalize()?,
            #[cfg(all(any(test, debug_assertions), unix))]
            path_race_test_hook: std::sync::Arc::new(std::sync::Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            replace_failure_test_hook: std::sync::Arc::new(AtomicBool::new(false)),
            #[cfg(any(test, debug_assertions))]
            cleanup_failure_test_hook: std::sync::Arc::new(AtomicBool::new(false)),
            #[cfg(any(test, debug_assertions))]
            replacement_test_hook: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// Installs a one-shot callback immediately before the final path
    /// component is opened or unlinked.  This narrow seam is only available
    /// in test/debug builds so Unix/macOS race behavior can be exercised
    /// deterministically without weakening normal path handling.
    #[cfg(all(any(test, debug_assertions), unix))]
    #[doc(hidden)]
    pub fn install_path_race_test_hook(&self, hook: std::sync::Arc<dyn Fn() + Send + Sync>) {
        *self
            .path_race_test_hook
            .lock()
            .expect("cache path race hook poisoned") = Some(hook);
    }

    #[cfg(all(any(test, debug_assertions), unix))]
    fn run_path_race_test_hook(&self) {
        let hook = self
            .path_race_test_hook
            .lock()
            .expect("cache path race hook poisoned")
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }

    #[cfg(unix)]
    fn write_atomic_unix<F>(
        &self,
        relative_path: PathBuf,
        write: F,
        replace_existing: bool,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::io::FromRawFd;

        validate_relative(&relative_path)?;
        let file_name = relative_path
            .file_name()
            .ok_or(CacheError::PathEscape)
            .and_then(|name| CString::new(name.as_bytes()).map_err(|_| CacheError::PathEscape))?;
        let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
        let directory_fd = self.open_parent_fd_unix(parent, true)?;

        // The opened parent descriptor pins every ancestor. An attacker may
        // rename the pathname and install a symlink while the hook is held,
        // but all operations below still target this directory inode.
        #[cfg(any(test, debug_assertions))]
        self.run_path_race_test_hook();

        let mut existing_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let existing_result = unsafe {
            libc::fstatat(
                directory_fd,
                file_name.as_ptr(),
                existing_stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if existing_result == 0 {
            let stat = unsafe { existing_stat.assume_init() };
            if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
                unsafe { libc::close(directory_fd) };
                return Err(CacheError::PathEscape);
            }
            if !replace_existing {
                let size_bytes =
                    u64::try_from(stat.st_size).map_err(|_| CacheError::SizeOutOfRange)?;
                unsafe { libc::close(directory_fd) };
                return Ok(CacheWrite {
                    relative_path,
                    size_bytes,
                    reused: true,
                });
            }
        } else {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::NotFound {
                unsafe { libc::close(directory_fd) };
                return Err(error.into());
            }
        }

        let partial_name = CString::new(format!(
            "{}.partial-{}",
            file_name.to_string_lossy(),
            uuid::Uuid::new_v4()
        ))
        .map_err(|_| CacheError::PathEscape)?;
        let partial_fd = unsafe {
            libc::openat(
                directory_fd,
                partial_name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC,
                0o600,
            )
        };
        if partial_fd < 0 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(directory_fd) };
            return Err(error.into());
        }
        let mut partial = unsafe { File::from_raw_fd(partial_fd) };
        if let Err(error) = write(&mut partial) {
            drop(partial);
            unsafe {
                libc::unlinkat(directory_fd, partial_name.as_ptr(), 0);
                libc::close(directory_fd);
            }
            return Err(CacheError::Write(error));
        }
        if let Err(error) = partial.flush().and_then(|()| partial.sync_all()) {
            drop(partial);
            unsafe {
                libc::unlinkat(directory_fd, partial_name.as_ptr(), 0);
                libc::close(directory_fd);
            }
            return Err(CacheError::Write(error));
        }
        let size_bytes = partial.metadata()?.len();
        drop(partial);

        #[cfg(any(test, debug_assertions))]
        if replace_existing && self.should_fail_replace_for_test() {
            unsafe {
                libc::unlinkat(directory_fd, partial_name.as_ptr(), 0);
                libc::close(directory_fd);
            }
            return Err(CacheError::Write(std::io::Error::other(
                "injected atomic replacement failure",
            )));
        }

        let publish_result = if replace_existing {
            unsafe {
                libc::renameat(
                    directory_fd,
                    partial_name.as_ptr(),
                    directory_fd,
                    file_name.as_ptr(),
                )
            }
        } else {
            unsafe {
                libc::linkat(
                    directory_fd,
                    partial_name.as_ptr(),
                    directory_fd,
                    file_name.as_ptr(),
                    0,
                )
            }
        };
        if publish_result == 0 {
            if !replace_existing {
                let unlink_result =
                    unsafe { libc::unlinkat(directory_fd, partial_name.as_ptr(), 0) };
                if unlink_result < 0 {
                    let error = std::io::Error::last_os_error();
                    unsafe { libc::close(directory_fd) };
                    return Err(error.into());
                }
            }
            unsafe { libc::close(directory_fd) };
            return Ok(CacheWrite {
                relative_path,
                size_bytes,
                reused: false,
            });
        }

        let publish_error = std::io::Error::last_os_error();
        unsafe {
            libc::unlinkat(directory_fd, partial_name.as_ptr(), 0);
        }
        if !replace_existing && publish_error.kind() == std::io::ErrorKind::AlreadyExists {
            let mut final_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            let final_result = unsafe {
                libc::fstatat(
                    directory_fd,
                    file_name.as_ptr(),
                    final_stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if final_result == 0 {
                let final_stat = unsafe { final_stat.assume_init() };
                unsafe { libc::close(directory_fd) };
                if (final_stat.st_mode & libc::S_IFMT) == libc::S_IFREG {
                    return Ok(CacheWrite {
                        relative_path,
                        size_bytes: u64::try_from(final_stat.st_size)
                            .map_err(|_| CacheError::SizeOutOfRange)?,
                        reused: true,
                    });
                }
                return Err(CacheError::PathEscape);
            }
        }
        unsafe { libc::close(directory_fd) };
        Err(CacheError::Write(publish_error))
    }

    #[cfg(unix)]
    fn open_parent_fd_unix(&self, relative: &Path, create: bool) -> Result<i32, CacheError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let root =
            CString::new(self.root.as_os_str().as_bytes()).map_err(|_| CacheError::PathEscape)?;
        let mut directory_fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for component in relative.components() {
            let name = CString::new(component.as_os_str().as_bytes())
                .map_err(|_| CacheError::PathEscape)?;
            let mut next_fd = unsafe {
                libc::openat(
                    directory_fd,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if next_fd < 0
                && create
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
            {
                let created = unsafe { libc::mkdirat(directory_fd, name.as_ptr(), 0o700) };
                if created < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        unsafe { libc::close(directory_fd) };
                        return Err(error.into());
                    }
                }
                next_fd = unsafe {
                    libc::openat(
                        directory_fd,
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
            }
            if next_fd < 0 {
                let error = std::io::Error::last_os_error();
                unsafe { libc::close(directory_fd) };
                return Err(error.into());
            }
            unsafe { libc::close(directory_fd) };
            directory_fd = next_fd;
        }
        Ok(directory_fd)
    }

    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn fail_next_replace_for_test(&self) {
        self.replace_failure_test_hook
            .store(true, Ordering::Release);
    }

    #[cfg(any(test, debug_assertions))]
    fn should_fail_replace_for_test(&self) -> bool {
        self.replace_failure_test_hook.swap(false, Ordering::AcqRel)
    }

    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn fail_next_cleanup_for_test(&self) {
        self.cleanup_failure_test_hook
            .store(true, Ordering::Release);
    }

    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn install_replacement_test_hook(&self, hook: std::sync::Arc<dyn Fn() + Send + Sync>) {
        *self
            .replacement_test_hook
            .lock()
            .expect("cache replacement hook poisoned") = Some(hook);
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    fn run_replacement_test_hook(&self) {
        let hook = self
            .replacement_test_hook
            .lock()
            .expect("cache replacement hook poisoned")
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }

    pub fn write_atomic<F>(
        &self,
        relative_path: PathBuf,
        write: F,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        self.write_atomic_inner(relative_path, write, false)
    }

    /// Atomically replaces the bytes at an immutable key. This is reserved
    /// for repair: a key can still have a regular but corrupt file, and
    /// treating mere existence as reuse would preserve that corruption.
    pub fn replace_atomic<F>(
        &self,
        relative_path: PathBuf,
        write: F,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        self.write_atomic_inner(relative_path, write, true)
    }

    fn write_atomic_inner<F>(
        &self,
        relative_path: PathBuf,
        write: F,
        replace_existing: bool,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        #[cfg(unix)]
        {
            self.write_atomic_unix(relative_path, write, replace_existing)
        }

        #[cfg(not(unix))]
        {
            let final_path = self.resolve_checked(&relative_path)?;
            if !replace_existing && is_regular_file(&final_path)? {
                return Ok(CacheWrite {
                    relative_path,
                    size_bytes: final_path.metadata()?.len(),
                    reused: true,
                });
            }

            let parent = final_path.parent().ok_or(CacheError::PathEscape)?;
            #[cfg(windows)]
            let _parent_guards = self.open_windows_parent_guards(
                relative_path.parent().unwrap_or_else(|| Path::new("")),
            )?;
            #[cfg(not(windows))]
            self.create_safe_directories(parent)?;
            let file_name = final_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(CacheError::PathEscape)?;
            let partial_path = parent.join(format!("{file_name}.partial-{}", uuid::Uuid::new_v4()));
            let mut partial_options = OpenOptions::new();
            partial_options.create_new(true).write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                const DELETE: u32 = 0x0001_0000;
                const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
                const GENERIC_WRITE: u32 = 0x4000_0000;
                const FILE_SHARE_READ: u32 = 0x1;
                const FILE_SHARE_WRITE: u32 = 0x2;
                const FILE_SHARE_DELETE: u32 = 0x4;
                partial_options
                    .access_mode(GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
            }
            let mut partial = partial_options.open(&partial_path)?;

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
            #[cfg(not(windows))]
            drop(partial);

            #[cfg(all(any(test, debug_assertions), not(windows)))]
            if replace_existing && self.should_fail_replace_for_test() {
                let _ = std::fs::remove_file(&partial_path);
                return Err(CacheError::Write(std::io::Error::other(
                    "injected atomic replacement failure",
                )));
            }
            #[cfg(any(test, debug_assertions))]
            if replace_existing {
                self.run_replacement_test_hook();
            }
            #[cfg(windows)]
            let publish = rename_file_by_handle_windows(
                &partial,
                _parent_guards.last().ok_or(CacheError::PathEscape)?,
                std::ffi::OsStr::new(file_name),
                replace_existing,
                {
                    #[cfg(any(test, debug_assertions))]
                    {
                        replace_existing && self.should_fail_replace_for_test()
                    }
                    #[cfg(not(any(test, debug_assertions)))]
                    {
                        false
                    }
                },
            );
            #[cfg(not(windows))]
            let publish = if replace_existing {
                std::fs::rename(&partial_path, &final_path)
            } else {
                std::fs::hard_link(&partial_path, &final_path)
            };
            match publish {
                Ok(()) => {
                    #[cfg(windows)]
                    drop(partial);
                    if !replace_existing {
                        #[cfg(not(windows))]
                        std::fs::remove_file(&partial_path)?;
                    }
                    Ok(CacheWrite {
                        relative_path,
                        size_bytes,
                        reused: false,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if replace_existing {
                        let _ = std::fs::remove_file(&partial_path);
                        return Err(CacheError::Write(error));
                    }
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

    pub fn read_checked_limited(
        &self,
        relative_path: &Path,
        max_bytes: u64,
    ) -> Result<Vec<u8>, CacheError> {
        let mut file = self.open_checked(relative_path)?;
        let length = file.metadata()?.len();
        if length > max_bytes {
            return Err(CacheError::SizeOutOfRange);
        }
        let capacity = usize::try_from(length).map_err(|_| CacheError::SizeOutOfRange)?;
        let mut bytes = Vec::with_capacity(capacity);
        std::io::Read::take(&mut file, max_bytes.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(CacheError::SizeOutOfRange);
        }
        Ok(bytes)
    }

    /// Opens a regular file beneath the managed cache root after checking every
    /// path component for traversal or symlink escapes.
    pub fn open_checked(&self, relative_path: &Path) -> Result<File, CacheError> {
        validate_relative(relative_path)?;
        #[cfg(unix)]
        {
            self.open_checked_unix(relative_path)
        }
        #[cfg(not(unix))]
        {
            let path = self.resolve_checked(relative_path)?;
            if !is_regular_file(&path)? {
                return Err(CacheError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "cache file does not exist",
                )));
            }
            #[cfg(windows)]
            use std::os::windows::fs::OpenOptionsExt;
            #[cfg(windows)]
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            let file = options.open(path)?;
            if !file.metadata()?.is_file() || !descriptor_is_contained(&file, &self.root) {
                return Err(CacheError::PathEscape);
            }
            Ok(file)
        }
    }

    /// Removes a managed cache file after applying the same containment checks
    /// used for reads. Missing files are treated as already removed.
    pub fn remove_checked(&self, relative_path: &Path) -> Result<(), CacheError> {
        validate_relative(relative_path)?;
        #[cfg(any(test, debug_assertions))]
        if self.cleanup_failure_test_hook.swap(false, Ordering::AcqRel) {
            return Err(CacheError::Io(std::io::Error::other(
                "injected cache cleanup failure",
            )));
        }
        #[cfg(unix)]
        {
            self.remove_checked_unix(relative_path)
        }
        #[cfg(windows)]
        {
            self.remove_checked_windows(relative_path)
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            let path = self.resolve_checked(relative_path)?;
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
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
        self.remove_checked(relative_path)
    }

    #[cfg(unix)]
    fn open_checked_unix(&self, relative_path: &Path) -> Result<File, CacheError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::io::FromRawFd;

        let mut components = relative_path.components();
        let file_name = components.next_back().ok_or(CacheError::PathEscape)?;
        let file_name =
            CString::new(file_name.as_os_str().as_bytes()).map_err(|_| CacheError::PathEscape)?;
        let root =
            CString::new(self.root.as_os_str().as_bytes()).map_err(|_| CacheError::PathEscape)?;
        let mut directory_fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for component in components {
            let component = match CString::new(component.as_os_str().as_bytes()) {
                Ok(component) => component,
                Err(_) => {
                    unsafe { libc::close(directory_fd) };
                    return Err(CacheError::PathEscape);
                }
            };
            let next_fd = unsafe {
                libc::openat(
                    directory_fd,
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if next_fd < 0 {
                let error = std::io::Error::last_os_error();
                unsafe { libc::close(directory_fd) };
                return Err(error.into());
            }
            unsafe { libc::close(directory_fd) };
            directory_fd = next_fd;
        }
        #[cfg(any(test, debug_assertions))]
        self.run_path_race_test_hook();
        let file_fd = unsafe {
            libc::openat(
                directory_fd,
                file_name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        unsafe { libc::close(directory_fd) };
        if file_fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(file_fd) };
        if !file.metadata()?.is_file() || !descriptor_is_contained(&file, &self.root) {
            return Err(CacheError::PathEscape);
        }
        Ok(file)
    }

    #[cfg(unix)]
    fn remove_checked_unix(&self, relative_path: &Path) -> Result<(), CacheError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let mut components = relative_path.components();
        let file_name = components.next_back().ok_or(CacheError::PathEscape)?;
        let file_name =
            CString::new(file_name.as_os_str().as_bytes()).map_err(|_| CacheError::PathEscape)?;
        let root =
            CString::new(self.root.as_os_str().as_bytes()).map_err(|_| CacheError::PathEscape)?;
        let mut directory_fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for component in components {
            let component = match CString::new(component.as_os_str().as_bytes()) {
                Ok(component) => component,
                Err(_) => {
                    unsafe { libc::close(directory_fd) };
                    return Err(CacheError::PathEscape);
                }
            };
            let next_fd = unsafe {
                libc::openat(
                    directory_fd,
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if next_fd < 0 {
                let error = std::io::Error::last_os_error();
                unsafe { libc::close(directory_fd) };
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Ok(());
                }
                return Err(error.into());
            }
            unsafe { libc::close(directory_fd) };
            directory_fd = next_fd;
        }
        #[cfg(any(test, debug_assertions))]
        self.run_path_race_test_hook();
        let mut file_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let stat_result = unsafe {
            libc::fstatat(
                directory_fd,
                file_name.as_ptr(),
                file_stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if stat_result < 0 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(directory_fd) };
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        let file_stat = unsafe { file_stat.assume_init() };
        if (file_stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            unsafe { libc::close(directory_fd) };
            return Err(CacheError::PathEscape);
        }
        let result = unsafe { libc::unlinkat(directory_fd, file_name.as_ptr(), 0) };
        let error = if result < 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        unsafe { libc::close(directory_fd) };
        match error {
            None => Ok(()),
            Some(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Some(error) => Err(error.into()),
        }
    }

    #[cfg(all(not(unix), not(windows)))]
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

    /// Opens every Windows cache ancestor with reparse-point handling and
    /// without sharing delete access. Keeping these directory handles alive
    /// through staging and publication prevents an ancestor rename/reparse
    /// swap from redirecting a pathname operation outside the managed root.
    #[cfg(windows)]
    fn open_windows_parent_guards(&self, relative: &Path) -> Result<Vec<File>, CacheError> {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;

        let mut guards = Vec::new();
        let mut current = self.root.clone();
        let open_directory = |path: &Path| -> Result<File, CacheError> {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::create_dir(path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    std::fs::symlink_metadata(path)?
                }
                Err(error) => return Err(error.into()),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(CacheError::PathEscape);
            }
            let mut options = OpenOptions::new();
            options
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
            let file = options.open(path)?;
            if !file.metadata()?.is_dir() || !descriptor_is_contained(&file, &self.root) {
                return Err(CacheError::PathEscape);
            }
            Ok(file)
        };
        guards.push(open_directory(&current)?);
        for component in relative.components() {
            current.push(component.as_os_str());
            guards.push(open_directory(&current)?);
        }
        Ok(guards)
    }

    #[cfg(windows)]
    fn remove_checked_windows(&self, relative_path: &Path) -> Result<(), CacheError> {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};

        #[repr(C)]
        struct FileDispositionInfo {
            delete_file: i32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateFileW(
                name: *const u16,
                access: u32,
                share_mode: u32,
                security_attributes: *const std::ffi::c_void,
                creation_disposition: u32,
                flags_and_attributes: u32,
                template_file: RawHandle,
            ) -> RawHandle;
            fn SetFileInformationByHandle(
                file: RawHandle,
                info_class: u32,
                info: *const std::ffi::c_void,
                info_size: u32,
            ) -> i32;
            fn GetLastError() -> u32;
        }
        const INVALID_HANDLE_VALUE: RawHandle = -1_isize as RawHandle;
        const DELETE: u32 = 0x0001_0000;
        const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        const FILE_SHARE_DELETE: u32 = 0x4;
        const OPEN_EXISTING: u32 = 3;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_INFO_DISPOSITION: u32 = 4;
        let path = self.root.join(relative_path);
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                DELETE | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32);
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        let file = unsafe { std::fs::File::from_raw_handle(handle) };
        if !file.metadata()?.is_file() || !descriptor_is_contained(&file, &self.root) {
            return Err(CacheError::PathEscape);
        }
        let disposition = FileDispositionInfo { delete_file: 1 };
        let deleted = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FILE_INFO_DISPOSITION,
                (&disposition as *const FileDispositionInfo).cast(),
                u32::try_from(std::mem::size_of::<FileDispositionInfo>())
                    .map_err(|_| CacheError::SizeOutOfRange)?,
            )
        };
        if deleted == 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32).into());
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
        path.to_str()
            .ok()
            .is_some_and(|path| Path::new(path).starts_with(root))
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
            file.as_raw_handle(),
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
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let root = root.to_string_lossy();
    let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
    path.eq_ignore_ascii_case(root)
        || (path.len() > root.len()
            && path[..root.len()].eq_ignore_ascii_case(root)
            && path.as_bytes()[root.len()] == b'\\')
}

fn is_regular_file(path: &Path) -> Result<bool, CacheError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn rename_file_by_handle_windows(
    partial: &File,
    parent: &File,
    file_name: &std::ffi::OsStr,
    replace_existing: bool,
    inject_native_failure: bool,
) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, RawHandle};

    #[repr(C)]
    struct FileRenameInfo {
        replace_if_exists: u8,
        root_directory: RawHandle,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            file: RawHandle,
            info_class: u32,
            info: *const std::ffi::c_void,
            info_size: u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }
    const FILE_RENAME_INFO_CLASS: u32 = 3;
    let wide = file_name.encode_wide().collect::<Vec<_>>();
    let name_bytes = wide
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| std::io::Error::other("replacement name is too long"))?;
    let info_size = std::mem::offset_of!(FileRenameInfo, file_name)
        .checked_add(name_bytes)
        .ok_or_else(|| std::io::Error::other("replacement metadata is too large"))?;
    let words = info_size.div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0_usize; words];
    let info = storage.as_mut_ptr().cast::<FileRenameInfo>();
    unsafe {
        (*info).replace_if_exists = u8::from(replace_existing);
        (*info).root_directory = parent.as_raw_handle();
        (*info).file_name_length = u32::try_from(name_bytes)
            .map_err(|_| std::io::Error::other("replacement name is too long"))?;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            std::ptr::addr_of_mut!((*info).file_name).cast::<u16>(),
            wide.len(),
        );
    }
    if inject_native_failure {
        return Err(std::io::Error::other(
            "injected native atomic replacement failure",
        ));
    }
    let renamed = unsafe {
        SetFileInformationByHandle(
            partial.as_raw_handle(),
            FILE_RENAME_INFO_CLASS,
            info.cast(),
            u32::try_from(info_size)
                .map_err(|_| std::io::Error::other("replacement metadata is too large"))?,
        )
    };
    if renamed != 0 {
        return Ok(());
    }
    Err(std::io::Error::from_raw_os_error(
        unsafe { GetLastError() } as i32
    ))
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
