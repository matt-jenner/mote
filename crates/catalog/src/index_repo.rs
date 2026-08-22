use photo_domain::{AssetId, LibraryId};
use rusqlite::{Connection, params};

use crate::asset_repo::upsert_asset_on;
use crate::{Catalog, CatalogError, NewAsset};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetShapeUpdate {
    pub asset_id: AssetId,
    pub width: u32,
    pub height: u32,
    pub orientation: Option<u16>,
    pub representative_rgb: Option<u32>,
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
pub enum CatalogIndexRecord {
    Discovered(NewAsset),
    Shaped(AssetShapeUpdate),
    Metadata(AssetMetadataUpdate),
    Warning(CatalogWarningRecord),
}

impl Catalog {
    pub fn apply_index_batch(
        &mut self,
        records: &[CatalogIndexRecord],
    ) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        for record in records {
            apply_record(&transaction, record)?;
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

fn apply_record(connection: &Connection, record: &CatalogIndexRecord) -> Result<(), CatalogError> {
    match record {
        CatalogIndexRecord::Discovered(asset) => upsert_asset_on(connection, asset),
        CatalogIndexRecord::Shaped(shape) => {
            connection.execute(
                "UPDATE assets SET width = ?2, height = ?3, orientation = ?4, representative_rgb = ?5 \
                 WHERE id = ?1",
                params![
                    shape.asset_id.as_uuid().as_bytes(),
                    i64::from(shape.width),
                    i64::from(shape.height),
                    shape.orientation.map(i64::from),
                    shape.representative_rgb.map(i64::from),
                ],
            )?;
            Ok(())
        }
        CatalogIndexRecord::Metadata(metadata) => apply_metadata(connection, metadata),
        CatalogIndexRecord::Warning(warning) => {
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
