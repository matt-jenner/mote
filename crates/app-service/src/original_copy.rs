use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use photo_catalog::Catalog;
use photo_domain::{GalleryScope, NativePathKey};
use serde::{Deserialize, Serialize};

use crate::{AppService, AppServiceError};

/// An authorized, ordered snapshot. Source paths stay private to the service.
/// Consumed by copying; retry by preparing the failed asset IDs again.
pub struct OriginalCopyBatch {
    items: Vec<PreparedItem>,
    source_roots: Vec<PathBuf>,
}

struct PreparedItem {
    asset_id: String,
    source: Option<PreparedSource>,
}

struct PreparedSource {
    root: PathBuf,
    relative: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyItemStatus {
    Copied,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyItemResult {
    pub asset_id: String,
    pub status: CopyItemStatus,
    pub destination_name: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalCopyResult {
    pub items: Vec<CopyItemResult>,
    pub copied_count: u32,
    pub failed_count: u32,
    /// A preference failure must not discard the successful-copy results.
    pub warning_code: Option<String>,
}

#[derive(Clone, Default)]
pub struct CopyCancellation(Arc<AtomicBool>);

impl CopyCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub enum OriginalCopyOutcome {
    Complete(OriginalCopyResult),
    Cancelled,
}

impl AppService {
    /// Native-only preference. Callers must check that the directory still exists.
    pub fn last_copy_destination(&self) -> Result<Option<PathBuf>, AppServiceError> {
        self.state()?
            .libraries
            .catalog()
            .last_copy_destination()
            .map_err(|_| AppServiceError::CopyPreparationFailed)?
            .map(|path| {
                path.to_path_buf()
                    .map_err(|_| AppServiceError::CopyPreparationFailed)
            })
            .transpose()
    }

    pub fn prepare_original_copy(
        &self,
        asset_ids: &[String],
    ) -> Result<OriginalCopyBatch, AppServiceError> {
        let ids = crate::picks::parse_pick_asset_ids(asset_ids)?;
        if ids.is_empty() {
            return Err(AppServiceError::InvalidLimit);
        }
        let state = self.state()?;
        let catalog = state.libraries.catalog();
        let picks = catalog
            .list_photo_picks()
            .map_err(|_| AppServiceError::CopyPreparationFailed)?
            .into_iter()
            .map(|pick| (pick.asset_id, pick.folder_group_id))
            .collect::<HashMap<_, _>>();
        let source_roots = source_roots(catalog)?;
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let group_id = *picks.get(&id).ok_or(AppServiceError::ForeignAsset)?;
            // Membership is checked independently of the caller and current wall.
            let records = catalog
                .wall_records_for_assets_scoped(group_id, GalleryScope::IncludeSubfolders, &[id])
                .map_err(|_| AppServiceError::CopyPreparationFailed)?;
            let asset = catalog
                .find_asset(id)
                .map_err(|_| AppServiceError::CopyPreparationFailed)?;
            let group = catalog
                .folder_group(group_id)
                .map_err(|_| AppServiceError::CopyPreparationFailed)?;
            let source = match (asset, group) {
                (Some(asset), Some(group))
                    if !records.is_empty() && asset.library_id == group.library_id =>
                {
                    catalog
                        .find_library(asset.library_id)
                        .map_err(|_| AppServiceError::CopyPreparationFailed)?
                        .and_then(|library| {
                            Some(PreparedSource {
                                root: library.canonical_root_key.to_path_buf().ok()?,
                                relative: asset.relative_path.to_path_buf().ok()?,
                            })
                        })
                        .filter(|source| resolve_source(source).is_ok())
                }
                // A pick can become unavailable after it was added. Keep its ID
                // in the operation instead of aborting the other authorized items.
                _ => None,
            };
            items.push(PreparedItem {
                asset_id: id.as_uuid().to_string(),
                source,
            });
        }
        Ok(OriginalCopyBatch {
            items,
            source_roots,
        })
    }

    /// Blocking filesystem work; desktop callers should run this on a worker.
    pub fn copy_originals(
        &self,
        batch: OriginalCopyBatch,
        destination: &Path,
    ) -> Result<OriginalCopyResult, AppServiceError> {
        self.copy_originals_with_progress(batch, destination, |_| {})
    }

    /// Reports each completed item, in order, without holding the service lock.
    pub fn copy_originals_with_progress(
        &self,
        batch: OriginalCopyBatch,
        destination: &Path,
        on_item: impl FnMut(&CopyItemResult),
    ) -> Result<OriginalCopyResult, AppServiceError> {
        match self.copy_originals_with_control(
            batch,
            destination,
            &CopyCancellation::default(),
            on_item,
        )? {
            OriginalCopyOutcome::Complete(mut result) => {
                self.remember_original_copy_destination(destination, &mut result);
                Ok(result)
            }
            OriginalCopyOutcome::Cancelled => unreachable!("private cancellation handle"),
        }
    }

    /// Controlled batches never persist a destination; their owner does so only
    /// after every batch has completed, so a later cancellation leaves it intact.
    pub fn copy_originals_with_control(
        &self,
        batch: OriginalCopyBatch,
        destination: &Path,
        cancellation: &CopyCancellation,
        mut on_item: impl FnMut(&CopyItemResult),
    ) -> Result<OriginalCopyOutcome, AppServiceError> {
        if cancellation.is_cancelled() {
            return Ok(OriginalCopyOutcome::Cancelled);
        }
        let canonical = self.validate_copy_destination(destination, &batch.source_roots)?;
        let mut reserved = HashSet::new();
        scan_names(&canonical, &mut reserved).map_err(|_| {
            if self.check_copy_destination(&canonical, &batch.source_roots)
                == Err("destination_missing")
            {
                AppServiceError::CopyDestinationMissing
            } else {
                AppServiceError::CopyDestinationUnavailable
            }
        })?;
        let mut result = OriginalCopyResult {
            items: Vec::with_capacity(batch.items.len()),
            copied_count: 0,
            failed_count: 0,
            warning_code: None,
        };
        for item in batch.items {
            if cancellation.is_cancelled() {
                return Ok(OriginalCopyOutcome::Cancelled);
            }
            let outcome = self
                .check_copy_destination(&canonical, &batch.source_roots)
                .and_then(|_| {
                    item.source
                        .as_ref()
                        .ok_or("source_unavailable")
                        .and_then(|source| {
                            self.copy_original_item(
                                source,
                                &canonical,
                                &batch.source_roots,
                                &mut reserved,
                                cancellation,
                            )
                        })
                });
            let outcome = outcome.map_err(|code| {
                if code != "cancelled"
                    && self.check_copy_destination(&canonical, &batch.source_roots)
                        == Err("destination_missing")
                {
                    "destination_missing"
                } else {
                    code
                }
            });
            let item_result = match outcome {
                Err("cancelled") => return Ok(OriginalCopyOutcome::Cancelled),
                Err("destination_missing") => return Err(AppServiceError::CopyDestinationMissing),
                Ok(name) => {
                    result.copied_count += 1;
                    CopyItemResult {
                        asset_id: item.asset_id,
                        status: CopyItemStatus::Copied,
                        destination_name: Some(name.to_string_lossy().into_owned()),
                        error_code: None,
                    }
                }
                Err(code) => {
                    result.failed_count += 1;
                    CopyItemResult {
                        asset_id: item.asset_id,
                        status: CopyItemStatus::Failed,
                        destination_name: None,
                        error_code: Some(code.into()),
                    }
                }
            };
            on_item(&item_result);
            result.items.push(item_result);
        }
        if cancellation.is_cancelled() {
            return Ok(OriginalCopyOutcome::Cancelled);
        }
        Ok(OriginalCopyOutcome::Complete(result))
    }

    pub fn remember_original_copy_destination(
        &self,
        destination: &Path,
        result: &mut OriginalCopyResult,
    ) {
        if result.copied_count > 0 {
            let saved = self.state().and_then(|mut state| {
                let canonical = fs::canonicalize(destination)
                    .map_err(|_| AppServiceError::CopyDestinationUnavailable)?;
                state
                    .libraries
                    .catalog_mut()
                    .set_last_copy_destination(&NativePathKey::from_path(&canonical))
                    .map_err(|_| AppServiceError::CopyPreparationFailed)
            });
            if saved.is_err() {
                result.warning_code = Some("destination_not_remembered".into());
            }
        }
    }

    fn check_copy_destination(
        &self,
        destination: &Path,
        roots: &[PathBuf],
    ) -> Result<(), &'static str> {
        if fs::symlink_metadata(destination)
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            return Err("destination_missing");
        }
        match self.validate_copy_destination(destination, roots) {
            Ok(checked) if checked == destination => Ok(()),
            Err(AppServiceError::CopyDestinationIsSource) => Err("destination_is_source"),
            _ => Err("destination_unavailable"),
        }
    }

    fn validate_copy_destination(
        &self,
        destination: &Path,
        prepared_roots: &[PathBuf],
    ) -> Result<PathBuf, AppServiceError> {
        let canonical = fs::canonicalize(destination)
            .map_err(|_| AppServiceError::CopyDestinationUnavailable)?;
        if !canonical.is_dir() {
            return Err(AppServiceError::CopyDestinationUnavailable);
        }
        let state = self.state()?;
        let current_roots = source_roots(state.libraries.catalog())?;
        for root in prepared_roots.iter().chain(&current_roots) {
            // Keep the stored identity protected even when a root is offline or
            // has been replaced. Also protect its current symlink target.
            let stored = photo_core::normalize_prevalidated_source_key(root)
                .map_err(|_| AppServiceError::CopyPreparationFailed)?;
            if path_is_within(&canonical, &stored)
                || fs::canonicalize(root).is_ok_and(|root| path_is_within(&canonical, &root))
            {
                return Err(AppServiceError::CopyDestinationIsSource);
            }
        }
        Ok(canonical)
    }

    fn copy_original_item(
        &self,
        source: &PreparedSource,
        destination: &Path,
        roots: &[PathBuf],
        reserved: &mut HashSet<String>,
        cancellation: &CopyCancellation,
    ) -> Result<OsString, &'static str> {
        let source_path = resolve_source(source)?;
        let mut input = File::open(source_path).map_err(|_| "source_unavailable")?;
        if !input.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return Err("source_unavailable");
        }
        let original_name = source.relative.file_name().ok_or("source_unavailable")?;
        self.check_copy_destination(destination, roots)?;
        let directory_identity =
            open_copy_directory(destination).map_err(|_| "destination_unavailable")?;
        let temporary = destination.join(format!(".mote-copy-{}", uuid::Uuid::new_v4()));
        #[cfg(all(test, unix))]
        tests::copy_failure("create").map_err(|_| "destination_unavailable")?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&temporary)
            .map_err(|_| "destination_unavailable")?;
        // Keep an identity handle for cleanup after closing the writing stream.
        let identity = match output.try_clone() {
            Ok(identity) => identity,
            Err(_) => {
                remove_created_file(&temporary, &output);
                return Err("copy_failed");
            }
        };
        let copied = (|| {
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                if cancellation.is_cancelled() {
                    return Err("cancelled");
                }
                let read = input.read(&mut buffer).map_err(|_| "copy_failed")?;
                if read == 0 {
                    break;
                }
                if cancellation.is_cancelled() {
                    return Err("cancelled");
                }
                #[cfg(all(test, unix))]
                tests::copy_failure("write").map_err(|_| "copy_failed")?;
                output
                    .write_all(&buffer[..read])
                    .map_err(|_| "copy_failed")?;
                #[cfg(all(test, unix))]
                tests::COPY_STEP.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook("chunk");
                    }
                });
            }
            #[cfg(all(test, unix))]
            tests::copy_failure("sync").map_err(|_| "copy_failed")?;
            output.sync_all().map_err(|_| "copy_failed")
        })();
        drop(output);
        let published = copied.and_then(|_| {
            let mut suffix = 1_u64;
            loop {
                if cancellation.is_cancelled() {
                    return Err("cancelled");
                }
                self.check_copy_destination(destination, roots)?;
                let name = candidate_name(original_name, suffix);
                suffix = suffix.checked_add(1).ok_or("destination_unavailable")?;
                let key = fold_name(&name);
                if reserved.contains(&key) {
                    continue;
                }
                // Earlier items already reserved their suffixes. Refresh external
                // entries only once we have a candidate that might be available.
                #[cfg(all(test, unix))]
                tests::copy_failure("scan").map_err(|_| "destination_unavailable")?;
                scan_names(destination, reserved).map_err(|_| "destination_unavailable")?;
                if !reserved.insert(key) {
                    continue;
                }
                // Revalidate every creation attempt, including retries after another
                // process created our candidate. Never recreate a lost directory.
                #[cfg(all(test, unix))]
                tests::COPY_STEP.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook("publish");
                    }
                });
                self.check_copy_destination(destination, roots)?;
                if cancellation.is_cancelled() {
                    return Err("cancelled");
                }
                if !is_created_file(&temporary, &identity) {
                    return Err("copy_failed");
                }
                match fs::hard_link(&temporary, destination.join(&name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        self.check_copy_destination(destination, roots)?;
                        return Err("destination_unavailable");
                    }
                }
                #[cfg(all(test, unix))]
                tests::COPY_STEP.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook("published");
                    }
                });
                return Ok(name);
            }
        });
        // Never unlink a substituted entry or a file beneath a newly selected source.
        if let Some(current_directory) = copy_directory_path(&directory_identity)
            && self
                .check_copy_destination(&current_directory, roots)
                .is_ok()
        {
            remove_created_file(
                &current_directory.join(temporary.file_name().expect("generated filename")),
                &identity,
            );
        }
        published
    }
}

fn source_roots(catalog: &Catalog) -> Result<Vec<PathBuf>, AppServiceError> {
    catalog
        .list_libraries()
        .map_err(|_| AppServiceError::CopyPreparationFailed)?
        .into_iter()
        .map(|library| {
            library
                .canonical_root_key
                .to_path_buf()
                .map_err(|_| AppServiceError::CopyPreparationFailed)
        })
        .collect()
}

fn resolve_source(source: &PreparedSource) -> Result<PathBuf, &'static str> {
    if source
        .relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("source_unavailable");
    }
    let root = fs::canonicalize(&source.root).map_err(|_| "source_unavailable")?;
    let path = fs::canonicalize(root.join(&source.relative)).map_err(|_| "source_unavailable")?;
    if !path_is_within(&path, &root) || !path.is_file() {
        return Err("source_unavailable");
    }
    Ok(path)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
    #[cfg(windows)]
    {
        let mut components = path.components();
        root.components().all(|root_component| {
            components.next().is_some_and(|component| {
                fold_name(component.as_os_str()) == fold_name(root_component.as_os_str())
            })
        })
    }
}

fn fold_name(name: &OsStr) -> String {
    // Lower first so capital-only aliases such as Kelvin and capital sharp S
    // normalize before uppercase expands multi-character variants.
    name.to_string_lossy().to_lowercase().to_uppercase()
}

fn scan_names(directory: &Path, reserved: &mut HashSet<String>) -> io::Result<()> {
    #[cfg(all(test, unix))]
    tests::DIRECTORY_SCANS.with(|count| count.set(count.get() + 1));
    for entry in fs::read_dir(directory)? {
        reserved.insert(fold_name(&entry?.file_name()));
    }
    Ok(())
}

fn candidate_name(original: &OsStr, suffix: u64) -> OsString {
    if suffix == 1 {
        return original.to_owned();
    }
    let path = Path::new(original);
    let mut name = path.file_stem().unwrap_or(original).to_owned();
    name.push(format!(" ({suffix})"));
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    name
}

fn open_copy_directory(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS permits opening a directory handle.
        options.custom_flags(0x0200_0000);
    }
    options.open(path)
}

/// Resolve the retained directory handle, including an external rename. This
/// avoids searching directories and cannot mistake a replacement at the old path
/// for the directory where the temporary file was created.
fn copy_directory_path(directory: &File) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn fcntl(fd: i32, command: i32, ...) -> i32;
        }
        let mut bytes = [0_u8; 1024];
        // SAFETY: F_GETPATH writes at most MAXPATHLEN bytes to a valid buffer;
        // the retained descriptor remains open throughout the call.
        if unsafe { fcntl(directory.as_raw_fd(), 50, bytes.as_mut_ptr()) } == -1 {
            return None;
        }
        let length = bytes.iter().position(|byte| *byte == 0)?;
        Some(PathBuf::from(OsStr::from_bytes(&bytes[..length])))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::fd::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{}", directory.as_raw_fd())).ok()
    }
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFinalPathNameByHandleW(
                handle: *mut std::ffi::c_void,
                path: *mut u16,
                length: u32,
                flags: u32,
            ) -> u32;
        }
        let mut buffer = vec![0_u16; 32768];
        // SAFETY: the handle is retained and the buffer size matches its capacity.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                directory.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if length == 0 || length >= buffer.len() {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buffer[..length])))
    }
}

fn is_created_file(target: &Path, output: &File) -> bool {
    let Ok(current) = fs::symlink_metadata(target) else {
        return false;
    };
    if !current.is_file() {
        return false;
    }
    #[cfg(unix)]
    let is_output = {
        use std::os::unix::fs::MetadataExt;
        output
            .metadata()
            .is_ok_and(|output| output.dev() == current.dev() && output.ino() == current.ino())
    };
    #[cfg(windows)]
    let is_output = same_file(output, target);
    is_output
}

fn remove_created_file(target: &Path, output: &File) {
    if is_created_file(target, output) {
        // Revalidation protects normal destination changes. An adversarial
        // rename between identity checking and unlink needs descriptor APIs.
        let _ = fs::remove_file(target);
    }
}

#[cfg(windows)]
fn same_file(left: &File, target: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    // Identity lookup must not require READ_DATA on a write-only output, and
    // must not follow a reparse point substituted after symlink_metadata.
    let Ok(right) = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(target)
    else {
        return false;
    };

    // Stable Rust does not expose the Windows file identity in MetadataExt.
    #[repr(C)]
    struct FileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            information: *mut FileInformation,
        ) -> i32;
    }
    fn identity(file: &File) -> Option<(u32, u32, u32)> {
        let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
        // SAFETY: the handle stays open and the output has the documented C
        // layout. A successful call initializes every field used below.
        unsafe {
            if GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) == 0 {
                return None;
            }
            let information = information.assume_init();
            Some((
                information.volume,
                information.index_high,
                information.index_low,
            ))
        }
    }
    identity(left).is_some_and(|left| identity(&right) == Some(left))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    type CopyStepHook = Box<dyn FnMut(&str)>;
    type CopyFailureHook = Box<dyn FnMut(&str) -> io::Result<()>>;

    pub(super) fn copy_failure(step: &str) -> io::Result<()> {
        COPY_FAILURE.with(|hook| match hook.borrow_mut().as_mut() {
            Some(hook) => hook(step),
            None => Ok(()),
        })
    }

    thread_local! {
        pub(super) static DIRECTORY_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(super) static COPY_STEP: std::cell::RefCell<Option<CopyStepHook>> = const { std::cell::RefCell::new(None) };
        pub(super) static COPY_FAILURE: std::cell::RefCell<Option<CopyFailureHook>> = const { std::cell::RefCell::new(None) };
    }

    #[test]
    fn replaced_temporary_entry_is_never_published_as_an_original() {
        let (_temp, service, mut batch, destination) = controlled_fixture();
        batch.items.truncate(1);
        let folder = destination.clone();
        COPY_STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |step| {
                if step == "publish" {
                    let temporary = fs::read_dir(&folder)
                        .unwrap()
                        .map(Result::unwrap)
                        .find(|entry| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(".mote-copy-")
                        })
                        .unwrap()
                        .path();
                    fs::remove_file(&temporary).unwrap();
                    fs::write(temporary, b"unrelated replacement").unwrap();
                }
            }))
        });
        let outcome = service
            .copy_originals_with_control(batch, &destination, &CopyCancellation::default(), |_| {})
            .unwrap();
        COPY_STEP.with(|hook| *hook.borrow_mut() = None);
        let OriginalCopyOutcome::Complete(result) = outcome else {
            panic!("expected item failure");
        };
        assert_eq!((result.copied_count, result.failed_count), (0, 1));
        assert!(!destination.join("first.jpg").exists());
        let replacement = fs::read_dir(&destination)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read(replacement).unwrap(), b"unrelated replacement");
    }

    #[test]
    fn renamed_destination_cleans_owned_temporary_file_on_cancel_or_loss() {
        for cancel_after_rename in [false, true] {
            let (temp, service, batch, destination) = controlled_fixture();
            let moved = temp.path().join("renamed-exports");
            let folder = destination.clone();
            let new_folder = moved.clone();
            let cancellation = CopyCancellation::default();
            let trigger = cancellation.clone();
            let mut renamed = false;
            COPY_STEP.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |step| {
                    if step == "chunk" && !renamed {
                        fs::rename(&folder, &new_folder).unwrap();
                        renamed = true;
                        if cancel_after_rename {
                            trigger.cancel();
                        }
                    }
                }))
            });
            let result =
                service.copy_originals_with_control(batch, &destination, &cancellation, |_| {
                    panic!("remaining files must not start")
                });
            COPY_STEP.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(
                result,
                Err(AppServiceError::CopyDestinationMissing) | Ok(OriginalCopyOutcome::Cancelled)
            ));
            assert_eq!(fs::read_dir(moved).unwrap().count(), 0);
        }
    }

    #[test]
    fn destination_io_failures_on_last_item_are_promoted_to_missing() {
        for boundary in ["create", "write", "sync", "scan"] {
            let (_temp, service, mut batch, destination) = controlled_fixture();
            batch.items.truncate(1);
            let folder = destination.clone();
            COPY_FAILURE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |step| {
                    if step == boundary {
                        fs::remove_dir_all(&folder).unwrap();
                        Err(io::Error::other("destination disconnected"))
                    } else {
                        Ok(())
                    }
                }))
            });
            let mut reports = 0;
            let outcome = service.copy_originals_with_control(
                batch,
                &destination,
                &CopyCancellation::default(),
                |_| reports += 1,
            );
            COPY_FAILURE.with(|hook| *hook.borrow_mut() = None);
            assert!(
                matches!(outcome, Err(AppServiceError::CopyDestinationMissing)),
                "{boundary}: {outcome:?}"
            );
            assert_eq!(reports, 0);
        }
    }

    #[test]
    fn independent_reader_never_observes_incomplete_final_payload() {
        let (_temp, service, mut batch, destination) = controlled_fixture();
        batch.items.truncate(1);
        let final_path = destination.join("first.jpg");
        let done = Arc::new(AtomicBool::new(false));
        let reader_done = done.clone();
        let started = Arc::new(std::sync::Barrier::new(2));
        let reader_started = started.clone();
        let reader = std::thread::spawn(move || {
            assert!(!final_path.exists());
            reader_started.wait();
            loop {
                if let Ok(bytes) = fs::read(&final_path) {
                    assert_eq!(bytes, vec![42; 4 * 1024 * 1024]);
                }
                if reader_done.load(Ordering::Acquire) {
                    break;
                }
                std::thread::yield_now();
            }
        });
        started.wait();
        service
            .copy_originals_with_control(batch, &destination, &CopyCancellation::default(), |_| {})
            .unwrap();
        done.store(true, Ordering::Release);
        reader.join().unwrap();
        assert_eq!(
            fs::read(destination.join("first.jpg")).unwrap(),
            vec![42; 4 * 1024 * 1024]
        );
    }

    fn controlled_fixture() -> (tempfile::TempDir, AppService, OriginalCopyBatch, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let destination = temp.path().join("exports");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&destination).unwrap();
        let service = AppService::open(crate::AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let items = ["first.jpg", "second.jpg", "third.jpg"]
            .into_iter()
            .map(|name| {
                fs::write(root.join(name), vec![42; 4 * 1024 * 1024]).unwrap();
                PreparedItem {
                    asset_id: name.into(),
                    source: Some(PreparedSource {
                        root: root.clone(),
                        relative: name.into(),
                    }),
                }
            })
            .collect();
        (
            temp,
            service,
            OriginalCopyBatch {
                items,
                source_roots: vec![root],
            },
            destination,
        )
    }

    #[test]
    fn cancelling_first_or_later_file_cleans_only_current_temp_and_keeps_complete_finals() {
        for completed_before_cancel in [0, 1] {
            let (_temp, service, batch, destination) = controlled_fixture();
            let cancel = CopyCancellation::default();
            let trigger = cancel.clone();
            let folder = destination.clone();
            let mut published = 0;
            COPY_STEP.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |step| {
                    if step == "chunk" && published == completed_before_cancel {
                        assert!(
                            !folder
                                .join(if published == 0 {
                                    "first.jpg"
                                } else {
                                    "second.jpg"
                                })
                                .exists()
                        );
                        trigger.cancel();
                    }
                    if step == "published" {
                        published += 1;
                    }
                }))
            });
            let outcome = service
                .copy_originals_with_control(batch, &destination, &cancel, |_| {})
                .unwrap();
            COPY_STEP.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(outcome, OriginalCopyOutcome::Cancelled));
            assert_eq!(
                fs::read_dir(&destination).unwrap().count(),
                completed_before_cancel
            );
            if completed_before_cancel > 0 {
                assert_eq!(
                    fs::read(destination.join("first.jpg")).unwrap(),
                    vec![42; 4 * 1024 * 1024]
                );
            }
            assert!(!destination.join("third.jpg").exists());
            assert_eq!(service.last_copy_destination().unwrap(), None);
        }
    }

    #[test]
    fn final_names_are_complete_and_competing_publication_is_never_overwritten() {
        let (_temp, service, batch, destination) = controlled_fixture();
        let folder = destination.clone();
        let mut raced = false;
        COPY_STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |step| {
                if step == "chunk" && !raced {
                    assert!(!folder.join("first.jpg").exists());
                }
                if step == "publish" && !raced {
                    fs::write(folder.join("first.jpg"), b"competitor").unwrap();
                    raced = true;
                }
                if step == "published" {
                    assert_eq!(
                        fs::read(folder.join("first (2).jpg")).unwrap(),
                        vec![42; 4 * 1024 * 1024]
                    );
                }
            }))
        });
        let result = service
            .copy_originals_with_control(batch, &destination, &CopyCancellation::default(), |_| {})
            .unwrap();
        COPY_STEP.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, OriginalCopyOutcome::Complete(_)));
        assert_eq!(
            fs::read(destination.join("first.jpg")).unwrap(),
            b"competitor"
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 4);
    }

    #[test]
    fn destination_removed_during_copy_stops_batch_with_missing_error() {
        let (_temp, service, batch, destination) = controlled_fixture();
        let folder = destination.clone();
        let mut removed = false;
        COPY_STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |step| {
                if step == "chunk" && !removed {
                    fs::remove_dir_all(&folder).unwrap();
                    removed = true;
                }
            }))
        });
        let result = service.copy_originals_with_control(
            batch,
            &destination,
            &CopyCancellation::default(),
            |_| panic!("missing destination must stop remaining items"),
        );
        COPY_STEP.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(
            result,
            Err(AppServiceError::CopyDestinationMissing)
        ));
        assert!(!destination.exists());
        assert_eq!(service.last_copy_destination().unwrap(), None);
    }

    #[test]
    fn temporary_payload_is_private_until_publication() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, service, batch, destination) = controlled_fixture();
        let folder = destination.clone();
        COPY_STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |step| {
                if step == "chunk" {
                    let temporary = fs::read_dir(&folder)
                        .unwrap()
                        .map(Result::unwrap)
                        .find(|entry| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(".mote-copy-")
                        })
                        .unwrap();
                    assert_eq!(
                        temporary.metadata().unwrap().permissions().mode() & 0o077,
                        0
                    );
                }
            }))
        });
        service
            .copy_originals_with_control(batch, &destination, &CopyCancellation::default(), |_| {})
            .unwrap();
        COPY_STEP.with(|hook| *hook.borrow_mut() = None);
    }

    // Catches rescanning for every suffix already reserved by earlier items.
    #[test]
    fn same_basename_batch_scans_at_most_once_per_item() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let destination = temp.path().join("exports");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&destination).unwrap();
        let service = AppService::open(crate::AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let mut items = Vec::new();
        for index in 0..250 {
            let relative = PathBuf::from(format!("{index}/IMG_2048.jpg"));
            fs::create_dir(root.join(index.to_string())).unwrap();
            fs::write(root.join(&relative), format!("original {index}")).unwrap();
            items.push(PreparedItem {
                asset_id: uuid::Uuid::new_v4().to_string(),
                source: Some(PreparedSource {
                    root: root.clone(),
                    relative,
                }),
            });
        }
        let batch = OriginalCopyBatch {
            items,
            source_roots: vec![root],
        };
        DIRECTORY_SCANS.with(|count| count.set(0));
        let result = service.copy_originals(batch, &destination).unwrap();
        assert_eq!((result.copied_count, result.failed_count), (250, 0));
        assert_eq!(
            result.items[249].destination_name.as_deref(),
            Some("IMG_2048 (250).jpg")
        );
        assert_eq!(
            fs::read(destination.join("IMG_2048 (250).jpg")).unwrap(),
            b"original 249"
        );
        let scans = DIRECTORY_SCANS.with(|count| count.get());
        assert!(
            scans <= 251,
            "250 copies needed {scans} directory scans, expected at most 251"
        );
    }

    // Catches Unicode case aliases receiving separate collision reservations.
    #[test]
    fn unicode_case_aliases_share_a_destination_reservation() {
        for (existing, requested) in [
            ("kelvin.jpg", "KELVIN.JPG"),
            ("straße.jpg", "STRAẞE.JPG"),
            ("strasse.jpg", "straße.JPG"),
        ] {
            let reserved = HashSet::from([fold_name(OsStr::new(existing))]);
            assert!(
                reserved.contains(&fold_name(OsStr::new(requested))),
                "{requested} must collide with {existing}"
            );
        }
    }

    // Catches cleanup deleting a replacement at the incomplete output's old path.
    #[test]
    fn failed_copy_cleanup_preserves_replacement_file() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("export.jpg");
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o200)).unwrap();
        assert!(
            File::open(&target).is_err(),
            "test requires a non-root runner"
        );
        fs::rename(&target, temp.path().join("moved-incomplete.jpg")).unwrap();
        fs::write(&target, b"replacement must survive").unwrap();
        remove_created_file(&target, &output);
        assert_eq!(fs::read(&target).unwrap(), b"replacement must survive");
    }

    // Catches cleanup leaving its own partial file behind after a write failure.
    #[test]
    fn failed_copy_cleanup_removes_only_its_own_incomplete_file() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("export.jpg");
        let other = temp.path().join("other.jpg");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .unwrap();
        std::io::Write::write_all(&mut output, b"incomplete").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o200)).unwrap();
        assert!(
            File::open(&target).is_err(),
            "test requires a non-root runner"
        );
        fs::write(&other, b"previous export").unwrap();
        remove_created_file(&target, &output);
        assert!(!target.exists());
        assert_eq!(fs::read(&other).unwrap(), b"previous export");
    }
}
