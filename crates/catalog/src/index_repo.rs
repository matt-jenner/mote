use photo_domain::{AssetId, LibraryId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::ShapeStatus;
use crate::asset_repo::{encode_media_kind, upsert_asset_on};
use crate::{Catalog, CatalogError, NewAsset};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetShapeUpdate {
    pub asset_id: AssetId,
    pub width: u32,
    pub height: u32,
    pub orientation: Option<u16>,
    pub representative_rgb: Option<u32>,
    pub shape_status: ShapeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetColourUpdate {
    pub asset_id: AssetId,
    pub representative_rgb: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogKeyword {
    pub normalized: String,
    pub display_value: String,
    pub hierarchy: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogProvenance {
    pub field_name: String,
    pub source_kind: String,
    pub raw_value: String,
    pub chosen: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMetadataUpdate {
    pub asset_id: AssetId,
    pub captured_at_utc: Option<String>,
    pub rating: Option<u8>,
    pub keywords: Vec<CatalogKeyword>,
    pub provenance: Vec<CatalogProvenance>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogWarningRecord {
    pub library_id: LibraryId,
    pub asset_id: Option<AssetId>,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogWarningSummary {
    pub asset_id: Option<AssetId>,
    pub code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogIndexRecord {
    Discovered(NewAsset),
    Shaped(AssetShapeUpdate),
    Coloured(AssetColourUpdate),
    Metadata(AssetMetadataUpdate),
    Warning(CatalogWarningRecord),
}

impl Catalog {
    pub fn source_warning_summaries(
        &self,
        library_id: LibraryId,
    ) -> Result<Vec<CatalogWarningSummary>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT asset_id, code FROM warnings WHERE library_id = ?1 AND asset_id IS NULL ORDER BY id",
        )?;
        let rows = statement.query_map(params![library_id.as_uuid().as_bytes()], |row| {
            let raw_asset_id: Option<Vec<u8>> = row.get(0)?;
            let asset_id = raw_asset_id
                .map(|value| crate::library_repo::decode_uuid(value, 0))
                .transpose()?
                .map(AssetId::from_uuid);
            Ok(CatalogWarningSummary {
                asset_id,
                code: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CatalogError::from)
    }

    pub fn record_warning_once(
        &mut self,
        warning: &CatalogWarningRecord,
    ) -> Result<bool, CatalogError> {
        let inserted = self.connection.execute(
            "INSERT INTO warnings (library_id, asset_id, code, message, occurred_at) \
             SELECT ?1, ?2, ?3, ?4, unixepoch() \
             WHERE NOT EXISTS ( \
               SELECT 1 FROM warnings \
               WHERE library_id = ?1 AND asset_id IS ?2 AND code = ?3 \
             )",
            params![
                warning.library_id.as_uuid().as_bytes(),
                warning.asset_id.map(|id| id.as_uuid().as_bytes().to_vec()),
                warning.code,
                warning.message,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn clear_warning(
        &mut self,
        library_id: LibraryId,
        asset_id: Option<AssetId>,
        code: &str,
    ) -> Result<u64, CatalogError> {
        let removed = self.connection.execute(
            "DELETE FROM warnings \
             WHERE library_id = ?1 AND asset_id IS ?2 AND code = ?3",
            params![
                library_id.as_uuid().as_bytes(),
                asset_id.map(|id| id.as_uuid().as_bytes().to_vec()),
                code,
            ],
        )?;
        u64::try_from(removed).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn apply_index_batch(
        &mut self,
        records: &[CatalogIndexRecord],
    ) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        for record in records {
            apply_record(&transaction, record, None)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn apply_index_batch_for_generation(
        &mut self,
        library_id: LibraryId,
        generation: u64,
        records: &[CatalogIndexRecord],
    ) -> Result<(), CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        for record in records {
            apply_record(&transaction, record, Some((library_id, generation)))?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn asset_keywords(&self, asset: AssetId) -> Result<Vec<String>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT display_value FROM asset_keywords WHERE asset_id = ?1 \
             ORDER BY normalized, hierarchy",
        )?;
        statement
            .query_map([asset.as_uuid().as_bytes()], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CatalogError::from)
    }
}

fn apply_record(
    connection: &Connection,
    record: &CatalogIndexRecord,
    generation: Option<(LibraryId, i64)>,
) -> Result<(), CatalogError> {
    match record {
        CatalogIndexRecord::Discovered(asset) => {
            let size_bytes = i64::try_from(asset.signature.size_bytes)
                .map_err(|_| CatalogError::ValueOutOfRange)?;
            let source_changed: bool = connection.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM assets
                   WHERE id = ?1
                     AND (media_kind <> ?2
                       OR size_bytes <> ?3
                       OR modified_unix_ns <> ?4
                       OR sidecar_modified_unix_ns IS NOT ?5
                       OR availability <> 'available')
                 )",
                params![
                    asset.id.as_uuid().as_bytes(),
                    encode_media_kind(asset.media_kind),
                    size_bytes,
                    asset.signature.modified_unix_ns.to_string(),
                    asset
                        .signature
                        .sidecar_modified_unix_ns
                        .map(|timestamp| timestamp.to_string()),
                ],
                |row| row.get(0),
            )?;
            if source_changed {
                clear_terminal_derivative_state(connection, asset.id)?;
            }
            connection.execute(
                "DELETE FROM warnings WHERE asset_id = ?1 AND code IN (
                   'source_missing', 'source_unreadable', 'source_check_failed',
                   'image_open_failed', 'exif_open_failed', 'exif_io_failed', 'xmp_open_failed',
                   'xmp_read_failed')",
                [asset.id.as_uuid().as_bytes()],
            )?;
            if cfg!(feature = "heic") && asset.media_kind == photo_domain::MediaKind::Heif {
                connection.execute(
                    "DELETE FROM warnings WHERE asset_id = ?1 AND code IN (
                       'shape_read_failed', 'image_open_failed', 'image_decode_failed', 'empty_image',
                       'exif_open_failed', 'exif_io_failed', 'exif_read_failed', 'invalid_exif_date',
                       'xmp_open_failed', 'xmp_read_failed', 'oversized_xmp', 'oversized_xmp_value',
                       'malformed_xmp', 'invalid_rating', 'invalid_xmp_date', 'empty_keyword',
                       'source_missing', 'source_unreadable', 'source_check_failed')",
                    [asset.id.as_uuid().as_bytes()],
                )?;
            }
            if let Some((library_id, generation)) = generation {
                if asset.library_id != library_id {
                    return Err(CatalogError::InvalidData(
                        "index batch contains an asset from another library".into(),
                    ));
                }
                upsert_asset_on(connection, asset)?;
                connection.execute(
                    "UPDATE assets SET last_seen_generation = ?2, availability = 'available' WHERE id = ?1",
                    params![asset.id.as_uuid().as_bytes(), generation],
                )?;
                if let Some(group) = asset.folder_group_id {
                    crate::selection_repo::add_asset_membership_on(
                        connection, group, asset.id, generation,
                    )?;
                }
                Ok(())
            } else {
                upsert_asset_on(connection, asset)?;
                if let Some(group) = asset.folder_group_id {
                    crate::selection_repo::add_asset_membership_on(connection, group, asset.id, 0)?;
                }
                Ok(())
            }
        }
        CatalogIndexRecord::Shaped(shape) => {
            ensure_asset_library(connection, shape.asset_id, generation)?;
            invalidate_disabled_heif_metadata(connection, shape.asset_id)?;
            let orientation_changed = if let Some(orientation) = shape.orientation {
                connection.query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM assets
                       WHERE id = ?1 AND orientation IS NOT ?2
                     )",
                    params![shape.asset_id.as_uuid().as_bytes(), i64::from(orientation)],
                    |row| row.get(0),
                )?
            } else {
                false
            };
            if orientation_changed {
                clear_terminal_derivative_state(connection, shape.asset_id)?;
            }
            connection.execute(
                "UPDATE assets SET width = CASE WHEN ?2 > 0 THEN ?2 ELSE width END, height = CASE WHEN ?3 > 0 THEN ?3 ELSE height END, orientation = COALESCE(?4, orientation), representative_rgb = COALESCE(?5, representative_rgb), shape_status = ?6 \
                 WHERE id = ?1",
                params![
                    shape.asset_id.as_uuid().as_bytes(),
                    i64::from(shape.width),
                    i64::from(shape.height),
                    shape.orientation.map(i64::from),
                    shape.representative_rgb.map(i64::from),
                    shape.shape_status.as_str(),
                ],
            )?;
            Ok(())
        }
        CatalogIndexRecord::Coloured(colour) => {
            ensure_asset_library(connection, colour.asset_id, generation)?;
            connection.execute(
                "UPDATE assets SET representative_rgb = ?2 WHERE id = ?1",
                params![
                    colour.asset_id.as_uuid().as_bytes(),
                    i64::from(colour.representative_rgb)
                ],
            )?;
            Ok(())
        }
        CatalogIndexRecord::Metadata(metadata) => {
            ensure_asset_library(connection, metadata.asset_id, generation)?;
            apply_metadata(connection, metadata)
        }
        CatalogIndexRecord::Warning(warning) => {
            if let (Some(asset_id), Some((library, generation))) = (warning.asset_id, generation)
                && matches!(
                    warning.code.as_str(),
                    "source_missing"
                        | "source_unreadable"
                        | "source_check_failed"
                        | "image_open_failed"
                        | "exif_open_failed"
                        | "exif_io_failed"
                        | "xmp_open_failed"
                        | "xmp_read_failed"
                )
            {
                // Keep generation-local failure even when a newer overlapping scan
                // suppresses this warning's stale shared-asset availability write.
                connection.execute(
                    "UPDATE scan_generations SET heif_metadata_retry_required = 1
                     WHERE library_id = ?1 AND generation = ?2 AND completed_at IS NULL
                       AND EXISTS (SELECT 1 FROM assets WHERE id = ?3 AND library_id = ?1 AND media_kind = 'heif')",
                    params![library.as_uuid().as_bytes(), generation, asset_id.as_uuid().as_bytes()],
                )?;
            }
            if let (Some(asset_id), Some((library, generation))) = (warning.asset_id, generation) {
                let availability = match warning.code.as_str() {
                    "source_missing" => Some("missing"),
                    "source_unreadable" => Some("unreadable"),
                    _ => None,
                };
                if availability.is_some() || warning.code == "source_check_failed" {
                    let updated = connection.execute(
                        "UPDATE assets SET availability = COALESCE(?4, availability), last_seen_generation = ?3 WHERE id = ?1 AND library_id = ?2
                         AND last_seen_generation <= ?3 AND EXISTS (
                           SELECT 1 FROM scan_generations WHERE library_id = ?2 AND generation = ?3
                           AND completed_at IS NULL AND source_was_online = 1
                         )",
                        params![asset_id.as_uuid().as_bytes(), library.as_uuid().as_bytes(), generation, availability],
                    )?;
                    if updated == 0 {
                        return Ok(());
                    }
                    connection.execute(
                        "INSERT INTO folder_group_assets (folder_group_id, asset_id, last_seen_generation)
                         SELECT folder_group_id, ?1, ?3 FROM scan_generations
                         WHERE library_id = ?2 AND generation = ?3 AND folder_group_id IS NOT NULL
                         ON CONFLICT(folder_group_id, asset_id) DO UPDATE
                         SET last_seen_generation = MAX(folder_group_assets.last_seen_generation, excluded.last_seen_generation)",
                        params![asset_id.as_uuid().as_bytes(), library.as_uuid().as_bytes(), generation],
                    )?;
                }
            }
            if let Some(asset_id) = warning.asset_id {
                ensure_asset_library(connection, asset_id, generation)?;
            }
            connection.execute(
                "INSERT INTO warnings (library_id, asset_id, code, message, occurred_at) \
                 VALUES (?1, ?2, ?3, ?4, unixepoch())",
                params![
                    warning.library_id.as_uuid().as_bytes(),
                    warning.asset_id.map(|id| id.as_uuid().as_bytes().to_vec()),
                    warning.code,
                    warning.message,
                ],
            )?;
            Ok(())
        }
    }
}

fn clear_terminal_derivative_state(
    connection: &Connection,
    asset_id: AssetId,
) -> Result<(), CatalogError> {
    connection.execute(
        "DELETE FROM derivative_failures WHERE asset_id = ?1",
        [asset_id.as_uuid().as_bytes()],
    )?;
    connection.execute(
        "DELETE FROM warnings WHERE asset_id = ?1 AND code = 'derivative_generation_terminal'",
        [asset_id.as_uuid().as_bytes()],
    )?;
    Ok(())
}

fn ensure_asset_library(
    connection: &Connection,
    asset_id: AssetId,
    generation: Option<(LibraryId, i64)>,
) -> Result<(), CatalogError> {
    let Some((library_id, _generation)) = generation else {
        return Ok(());
    };
    let found: Option<Vec<u8>> = connection
        .query_row(
            "SELECT library_id FROM assets WHERE id = ?1",
            [asset_id.as_uuid().as_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(found) = found else {
        return Err(CatalogError::InvalidData(
            "index update targets an unknown asset".into(),
        ));
    };
    if found.as_slice() != library_id.as_uuid().as_bytes() {
        return Err(CatalogError::InvalidData(
            "index update targets an asset from another library".into(),
        ));
    }
    Ok(())
}

fn apply_metadata(
    connection: &Connection,
    metadata: &AssetMetadataUpdate,
) -> Result<(), CatalogError> {
    invalidate_disabled_heif_metadata(connection, metadata.asset_id)?;
    for table in ["metadata_provenance", "asset_keywords"] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE asset_id = ?1 AND EXISTS (SELECT 1 FROM assets WHERE id = ?1 AND media_kind = 'heif')"),
            [metadata.asset_id.as_uuid().as_bytes()],
        )?;
    }
    connection.execute(
        "UPDATE assets SET captured_at_utc = ?2, rating = ?3 WHERE id = ?1",
        params![
            metadata.asset_id.as_uuid().as_bytes(),
            metadata.captured_at_utc,
            metadata.rating.map(i64::from),
        ],
    )?;
    for keyword in &metadata.keywords {
        connection.execute(
            "INSERT INTO asset_keywords (asset_id, normalized, display_value, hierarchy) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(asset_id, normalized, hierarchy) DO UPDATE SET \
                display_value = excluded.display_value",
            params![
                metadata.asset_id.as_uuid().as_bytes(),
                keyword.normalized,
                keyword.display_value,
                keyword.hierarchy.as_deref().unwrap_or(""),
            ],
        )?;
    }
    for provenance in &metadata.provenance {
        connection.execute(
            "INSERT INTO metadata_provenance \
                (asset_id, field_name, source_kind, raw_value, chosen) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(asset_id, field_name, source_kind, raw_value) DO UPDATE SET \
                chosen = excluded.chosen",
            params![
                metadata.asset_id.as_uuid().as_bytes(),
                provenance.field_name,
                provenance.source_kind,
                provenance.raw_value,
                i64::from(provenance.chosen),
            ],
        )?;
    }
    Ok(())
}

fn invalidate_disabled_heif_metadata(
    connection: &Connection,
    asset: AssetId,
) -> Result<(), CatalogError> {
    if !cfg!(feature = "heic") {
        connection.execute(
            "UPDATE folder_groups SET heif_metadata_revision = 0
             WHERE id IN (SELECT folder_group_id FROM folder_group_heif_assets WHERE asset_id = ?1)",
            [asset.as_uuid().as_bytes()],
        )?;
    }
    Ok(())
}
