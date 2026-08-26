use photo_domain::{AssetId, LibraryId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::ShapeStatus;
use crate::asset_repo::upsert_asset_on;
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
                Ok(())
            } else {
                upsert_asset_on(connection, asset)
            }
        }
        CatalogIndexRecord::Shaped(shape) => {
            ensure_asset_library(connection, shape.asset_id, generation)?;
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
