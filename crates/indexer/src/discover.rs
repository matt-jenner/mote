use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use photo_catalog::NewAsset;
use photo_domain::{FileSignature, LibraryId, MediaKind, RelativePathKey};

use crate::IndexError;

pub struct DiscoveredAsset {
    pub asset: NewAsset,
    pub source_path: PathBuf,
    pub sidecar_path: Option<PathBuf>,
}

pub fn discover_asset(
    root: &Path,
    path: &Path,
    library_id: LibraryId,
) -> Result<Option<DiscoveredAsset>, IndexError> {
    let Some(media_kind) = MediaKind::from_path(path) else {
        return Ok(None);
    };
    let relative_path = path
        .strip_prefix(root)
        .map_err(|_| IndexError::PathOutsideRoot(path.to_path_buf()))?;
    let relative = RelativePathKey::from_relative_path(relative_path)
        .map_err(|error| IndexError::InvalidRelativePath(error.to_string()))?;
    let metadata = path.metadata()?;
    let sidecar_path = find_sidecar(path)?;
    let mut asset = NewAsset::minimal(
        library_id,
        relative,
        relative_path.to_string_lossy(),
        media_kind,
        metadata.len(),
    );
    asset.signature = FileSignature {
        size_bytes: metadata.len(),
        modified_unix_ns: system_time_ns(metadata.modified()?),
        sidecar_modified_unix_ns: sidecar_path
            .as_ref()
            .and_then(|sidecar| sidecar.metadata().ok())
            .and_then(|metadata| metadata.modified().ok())
            .map(system_time_ns),
    };
    Ok(Some(DiscoveredAsset {
        asset,
        source_path: path.to_path_buf(),
        sidecar_path,
    }))
}

pub fn find_sidecar(media_path: &Path) -> Result<Option<PathBuf>, IndexError> {
    let Some(parent) = media_path.parent() else {
        return Ok(None);
    };
    let Some(file_name) = media_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let stem = media_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(file_name);
    let preferred = format!("{file_name}.xmp").to_lowercase();
    let fallback = format!("{stem}.xmp").to_lowercase();
    let mut entries = std::fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for wanted in [preferred, fallback] {
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn system_time_ns(value: SystemTime) -> i128 {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX),
        Err(error) => -i128::try_from(error.duration().as_nanos()).unwrap_or(i128::MAX),
    }
}
