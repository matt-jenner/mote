use photo_domain::{
    AssetId, Availability, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey,
};
use rusqlite::params;

use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError, parse_i128};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAsset {
    pub id: AssetId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
    pub media_kind: MediaKind,
    pub signature: FileSignature,
    pub folder_group_id: Option<FolderGroupId>,
}

impl NewAsset {
    pub fn minimal(
        library: LibraryId,
        path: RelativePathKey,
        display_path: impl Into<String>,
        kind: MediaKind,
        size_bytes: u64,
    ) -> Self {
        Self {
            id: AssetId::for_path(library, &path),
            library_id: library,
            relative_path: path,
            display_path: display_path.into(),
            media_kind: kind,
            signature: FileSignature {
                size_bytes,
                modified_unix_ns: 0,
                sidecar_modified_unix_ns: None,
            },
            folder_group_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetRecord {
    pub id: AssetId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
    pub media_kind: MediaKind,
    pub signature: FileSignature,
    pub availability: Availability,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    pub representative_rgb: Option<u32>,
    pub captured_at_utc: Option<String>,
    pub rating: Option<u8>,
    pub folder_group_id: Option<FolderGroupId>,
    pub provisional_order: u64,
    pub shape_status: crate::ShapeStatus,
}

impl Catalog {
    pub fn upsert_asset(&mut self, value: &NewAsset) -> Result<(), CatalogError> {
        upsert_asset_on(&self.connection, value)
    }

    pub fn find_asset(&self, id: AssetId) -> Result<Option<AssetRecord>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT id, library_id, relative_path_key, display_path, media_kind, size_bytes, \
                    modified_unix_ns, sidecar_modified_unix_ns, availability, width, height, \
                    orientation, representative_rgb, captured_at_utc, rating, folder_group_id, provisional_order, shape_status \
             FROM assets WHERE id = ?1",
        )?;
        let mut rows = statement.query([id.as_uuid().as_bytes()])?;
        rows.next()?
            .map(decode_asset)
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_assets_page(
        &self,
        library: LibraryId,
        after: Option<(String, AssetId)>,
        limit: u32,
    ) -> Result<Vec<AssetRecord>, CatalogError> {
        const COLUMNS: &str = "id, library_id, relative_path_key, display_path, media_kind, size_bytes, modified_unix_ns, sidecar_modified_unix_ns, availability, width, height, orientation, representative_rgb, captured_at_utc, rating, folder_group_id, provisional_order, shape_status";
        let records = if let Some((display_path, id)) = after {
            let sql = format!(
                "SELECT {COLUMNS} FROM assets \
                 WHERE library_id = ?1 AND (display_path > ?2 OR (display_path = ?2 AND id > ?3)) \
                 ORDER BY display_path, id LIMIT ?4"
            );
            let mut statement = self.connection.prepare(&sql)?;
            statement
                .query_map(
                    params![
                        library.as_uuid().as_bytes(),
                        display_path,
                        id.as_uuid().as_bytes(),
                        i64::from(limit)
                    ],
                    decode_asset,
                )?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let sql = format!(
                "SELECT {COLUMNS} FROM assets WHERE library_id = ?1 \
                 ORDER BY display_path, id LIMIT ?2"
            );
            let mut statement = self.connection.prepare(&sql)?;
            statement
                .query_map(
                    params![library.as_uuid().as_bytes(), i64::from(limit)],
                    decode_asset,
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(records)
    }
}

pub(crate) fn upsert_asset_on(
    connection: &rusqlite::Connection,
    value: &NewAsset,
) -> Result<(), CatalogError> {
    let size_bytes =
        i64::try_from(value.signature.size_bytes).map_err(|_| CatalogError::ValueOutOfRange)?;
    let relative_path = value
        .relative_path
        .to_path_buf()
        .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
    let relative_parent = RelativePathKey::from_relative_path(
        relative_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("")),
    )
    .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
    connection.execute(
        "INSERT INTO assets (\
                id, library_id, relative_path_key, display_path, media_kind, size_bytes, \
                modified_unix_ns, sidecar_modified_unix_ns, availability, folder_group_id, provisional_order, relative_parent_key \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'available', ?9, \
                COALESCE((SELECT MAX(provisional_order) + 1 FROM assets WHERE library_id = ?2), 1), ?10) \
             ON CONFLICT(id) DO UPDATE SET \
                display_path = excluded.display_path, \
                media_kind = excluded.media_kind, \
                size_bytes = excluded.size_bytes, \
                modified_unix_ns = excluded.modified_unix_ns, \
                sidecar_modified_unix_ns = excluded.sidecar_modified_unix_ns, \
                folder_group_id = COALESCE(excluded.folder_group_id, assets.folder_group_id), \
                relative_parent_key = excluded.relative_parent_key, \
                availability = excluded.availability",
        params![
            value.id.as_uuid().as_bytes(),
            value.library_id.as_uuid().as_bytes(),
            value.relative_path.as_bytes(),
            value.display_path,
            encode_media_kind(value.media_kind),
            size_bytes,
            value.signature.modified_unix_ns.to_string(),
            value
                .signature
                .sidecar_modified_unix_ns
                .map(|timestamp| timestamp.to_string()),
            value.folder_group_id.map(|id| id.as_uuid().as_bytes().to_vec()),
            relative_parent.as_bytes(),
        ],
    )?;
    Ok(())
}

fn decode_asset(row: &rusqlite::Row<'_>) -> Result<AssetRecord, rusqlite::Error> {
    let id: Vec<u8> = row.get(0)?;
    let library_id: Vec<u8> = row.get(1)?;
    let relative_path: Vec<u8> = row.get(2)?;
    let media_kind: String = row.get(4)?;
    let size_bytes: i64 = row.get(5)?;
    let modified: String = row.get(6)?;
    let sidecar_modified: Option<String> = row.get(7)?;
    let availability: String = row.get(8)?;
    let group: Option<Vec<u8>> = row.get(15)?;

    Ok(AssetRecord {
        id: AssetId::from_uuid(decode_uuid(id, 0)?),
        library_id: LibraryId::from_uuid(decode_uuid(library_id, 1)?),
        relative_path: RelativePathKey::from_bytes(relative_path)
            .map_err(|error| conversion_error(2, rusqlite::types::Type::Blob, error))?,
        display_path: row.get(3)?,
        media_kind: decode_media_kind(&media_kind, 4)?,
        signature: FileSignature {
            size_bytes: u64::try_from(size_bytes)
                .map_err(|error| conversion_error(5, rusqlite::types::Type::Integer, error))?,
            modified_unix_ns: parse_i128(modified)?,
            sidecar_modified_unix_ns: sidecar_modified.map(parse_i128).transpose()?,
        },
        availability: decode_availability(&availability, 8)?,
        width: optional_u32(row, 9)?,
        height: optional_u32(row, 10)?,
        orientation: optional_u16(row, 11)?,
        representative_rgb: optional_u32(row, 12)?,
        captured_at_utc: row.get(13)?,
        rating: optional_u8(row, 14)?,
        folder_group_id: group
            .map(|id| decode_uuid(id, 15).map(FolderGroupId::from_uuid))
            .transpose()?,
        provisional_order: u64::try_from(row.get::<_, i64>(16)?)
            .map_err(|error| conversion_error(16, rusqlite::types::Type::Integer, error))?,
        shape_status: crate::ShapeStatus::decode(row.get::<_, String>(17)?.as_str(), 17)?,
    })
}

fn optional_u32(row: &rusqlite::Row<'_>, column: usize) -> Result<Option<u32>, rusqlite::Error> {
    row.get::<_, Option<i64>>(column)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|error| conversion_error(column, rusqlite::types::Type::Integer, error))
        })
        .transpose()
}

fn optional_u16(row: &rusqlite::Row<'_>, column: usize) -> Result<Option<u16>, rusqlite::Error> {
    row.get::<_, Option<i64>>(column)?
        .map(|value| {
            u16::try_from(value)
                .map_err(|error| conversion_error(column, rusqlite::types::Type::Integer, error))
        })
        .transpose()
}

fn optional_u8(row: &rusqlite::Row<'_>, column: usize) -> Result<Option<u8>, rusqlite::Error> {
    row.get::<_, Option<i64>>(column)?
        .map(|value| {
            u8::try_from(value)
                .map_err(|error| conversion_error(column, rusqlite::types::Type::Integer, error))
        })
        .transpose()
}

fn conversion_error(
    column: usize,
    data_type: rusqlite::types::Type,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, data_type, Box::new(error))
}

fn encode_media_kind(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Jpeg => "jpeg",
        MediaKind::Png => "png",
        MediaKind::Tiff => "tiff",
        MediaKind::Heif => "heif",
        MediaKind::Webp => "webp",
        MediaKind::Avif => "avif",
        MediaKind::Raw => "raw",
        MediaKind::Video => "video",
        MediaKind::Unknown => "unknown",
    }
}

pub(crate) fn decode_media_kind(value: &str, column: usize) -> Result<MediaKind, rusqlite::Error> {
    match value {
        "jpeg" => Ok(MediaKind::Jpeg),
        "png" => Ok(MediaKind::Png),
        "tiff" => Ok(MediaKind::Tiff),
        "heif" => Ok(MediaKind::Heif),
        "webp" => Ok(MediaKind::Webp),
        "avif" => Ok(MediaKind::Avif),
        "raw" => Ok(MediaKind::Raw),
        "video" => Ok(MediaKind::Video),
        "unknown" => Ok(MediaKind::Unknown),
        other => Err(conversion_error(
            column,
            rusqlite::types::Type::Text,
            CatalogError::InvalidData(format!("unknown media kind {other}")),
        )),
    }
}

pub(crate) fn decode_availability(
    value: &str,
    column: usize,
) -> Result<Availability, rusqlite::Error> {
    match value {
        "available" => Ok(Availability::Available),
        "root_offline" => Ok(Availability::RootOffline),
        "missing" => Ok(Availability::Missing),
        "unreadable" => Ok(Availability::Unreadable),
        other => Err(conversion_error(
            column,
            rusqlite::types::Type::Text,
            CatalogError::InvalidData(format!("unknown availability {other}")),
        )),
    }
}
