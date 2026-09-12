use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

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

impl AppService {
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
        mut on_item: impl FnMut(&CopyItemResult),
    ) -> Result<OriginalCopyResult, AppServiceError> {
        let canonical = self.validate_copy_destination(destination, &batch.source_roots)?;
        let mut reserved = HashSet::new();
        scan_names(&canonical, &mut reserved)
            .map_err(|_| AppServiceError::CopyDestinationUnavailable)?;
        let mut result = OriginalCopyResult {
            items: Vec::with_capacity(batch.items.len()),
            copied_count: 0,
            failed_count: 0,
            warning_code: None,
        };
        for item in batch.items {
            let outcome = item
                .source
                .as_ref()
                .ok_or("source_unavailable")
                .and_then(|source| {
                    self.copy_original_item(source, &canonical, &batch.source_roots, &mut reserved)
                });
            let item_result = match outcome {
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
        if result.copied_count > 0 {
            let saved = self.state().and_then(|mut state| {
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
        Ok(result)
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
    ) -> Result<OsString, &'static str> {
        let source_path = resolve_source(source)?;
        let mut input = File::open(source_path).map_err(|_| "source_unavailable")?;
        if !input.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return Err("source_unavailable");
        }
        let original_name = source.relative.file_name().ok_or("source_unavailable")?;
        let mut suffix = 1_u64;
        loop {
            let name = candidate_name(original_name, suffix);
            suffix = suffix.checked_add(1).ok_or("destination_unavailable")?;
            let key = fold_name(&name);
            if reserved.contains(&key) {
                continue;
            }
            // Earlier items already reserved their suffixes. Refresh external
            // entries only once we have a candidate that might be available.
            scan_names(destination, reserved).map_err(|_| "destination_unavailable")?;
            if !reserved.insert(key) {
                continue;
            }
            // Revalidate every creation attempt, including retries after another
            // process created our candidate. Never recreate a lost directory.
            let checked = self
                .validate_copy_destination(destination, roots)
                .map_err(|error| match error {
                    AppServiceError::CopyDestinationIsSource => "destination_is_source",
                    _ => "destination_unavailable",
                })?;
            if checked != destination {
                return Err("destination_unavailable");
            }
            let target = checked.join(&name);
            let mut output = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(output) => output,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("destination_unavailable"),
            };
            // Neither copying nor cleanup ever opens a source for writing.
            if io::copy(&mut input, &mut output)
                .and_then(|_| output.sync_all())
                .is_err()
            {
                if self
                    .validate_copy_destination(destination, roots)
                    .is_ok_and(|checked| checked == destination)
                {
                    remove_created_file(&target, &output);
                }
                return Err("copy_failed");
            }
            return Ok(name);
        }
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

fn remove_created_file(target: &Path, output: &File) {
    let Ok(current) = fs::symlink_metadata(target) else {
        return;
    };
    if !current.is_file() {
        return;
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
    if is_output {
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

    thread_local! {
        pub(super) static DIRECTORY_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
