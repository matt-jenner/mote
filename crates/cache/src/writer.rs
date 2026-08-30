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

#[derive(Clone)]
pub struct CacheWriter {
    root: PathBuf,
    #[cfg(any(test, debug_assertions))]
    path_race_test_hook:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>>,
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
            #[cfg(any(test, debug_assertions))]
            path_race_test_hook: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// Installs a one-shot callback immediately before the final path
    /// component is opened or unlinked.  This narrow seam is only available
    /// in test/debug builds so Unix/macOS race behavior can be exercised
    /// deterministically without weakening normal path handling.
    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn install_path_race_test_hook(&self, hook: std::sync::Arc<dyn Fn() + Send + Sync>) {
        *self
            .path_race_test_hook
            .lock()
            .expect("cache path race hook poisoned") = Some(hook);
    }

    #[cfg(any(test, debug_assertions))]
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
        let final_path = self.resolve_checked(&relative_path)?;
        if !replace_existing && is_regular_file(&final_path)? {
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

        let publish = if replace_existing {
            match std::fs::rename(&partial_path, &final_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Windows does not replace an existing destination with
                    // rename. The destination was validated above and this
                    // path is used only for an in-process immutable-key
                    // repair, so remove and retry the atomic rename.
                    std::fs::remove_file(&final_path)?;
                    std::fs::rename(&partial_path, &final_path)
                }
                Err(error) => Err(error),
            }
        } else {
            std::fs::hard_link(&partial_path, &final_path)
        };
        match publish {
            Ok(()) => {
                if !replace_existing {
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
        validate_relative(relative_path)?;
        #[cfg(unix)]
        {
            return self.open_checked_unix(relative_path);
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
            return Ok(file);
        }
    }

    /// Removes a managed cache file after applying the same containment checks
    /// used for reads. Missing files are treated as already removed.
    pub fn remove_checked(&self, relative_path: &Path) -> Result<(), CacheError> {
        validate_relative(relative_path)?;
        #[cfg(unix)]
        {
            return self.remove_checked_unix(relative_path);
        }
        #[cfg(not(unix))]
        {
            #[cfg(windows)]
            {
                return self.remove_checked_windows(relative_path);
            }
            let path = self.resolve_checked(relative_path)?;
            return match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            };
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
