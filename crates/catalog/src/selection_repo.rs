use photo_domain::{AssetId, FolderGroupId, LibraryId, RelativePathKey};
use rusqlite::{OptionalExtension, params};

use crate::cache_repo::{DerivativeRecord, decode_derivative};
use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderGroupRecord {
    pub id: FolderGroupId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
}

impl Catalog {
    pub fn folder_group(
        &self,
        id: FolderGroupId,
    ) -> Result<Option<FolderGroupRecord>, CatalogError> {
        self.connection
            .query_row(
                "SELECT id, library_id, relative_path_key, display_path FROM folder_groups WHERE id = ?1",
                [id.as_uuid().as_bytes()],
                decode_folder_group,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn add_asset_membership(
        &mut self,
        group: FolderGroupId,
        asset: AssetId,
        generation: u64,
    ) -> Result<(), CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        let library: Vec<u8> = transaction
            .query_row(
                "SELECT library_id FROM folder_groups WHERE id = ?1",
                [group.as_uuid().as_bytes()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| CatalogError::InvalidData("folder group does not exist".into()))?;
        let asset_library: Vec<u8> = transaction
            .query_row(
                "SELECT library_id FROM assets WHERE id = ?1",
                [asset.as_uuid().as_bytes()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| CatalogError::InvalidData("asset does not exist".into()))?;
        if asset_library != library {
            return Err(CatalogError::InvalidData(
                "asset and folder group belong to different libraries".into(),
            ));
        }
        add_asset_membership_on(&transaction, group, asset, generation)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn finish_group_generation(
        &mut self,
        library: LibraryId,
        group: FolderGroupId,
        generation: u64,
    ) -> Result<(), CatalogError> {
        self.finish_group_generation_count(library, group, generation)
            .map(|_| ())
    }

    pub(crate) fn finish_group_generation_count(
        &mut self,
        library: LibraryId,
        group: FolderGroupId,
        generation: u64,
    ) -> Result<u64, CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        ensure_group_library(&transaction, library, group)?;
        let source_online: i64 = transaction
            .query_row(
                "SELECT source_was_online FROM scan_generations
                 WHERE library_id = ?1 AND folder_group_id = ?2 AND generation = ?3",
                params![
                    library.as_uuid().as_bytes(),
                    group.as_uuid().as_bytes(),
                    generation
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                CatalogError::InvalidData(format!(
                    "scan generation {generation} is missing for folder group"
                ))
            })?;

        let mut marked_missing = 0;
        if source_online != 0 {
            transaction.execute(
                "DELETE FROM folder_group_assets
                 WHERE folder_group_id = ?1 AND last_seen_generation < ?2",
                params![group.as_uuid().as_bytes(), generation],
            )?;
            transaction.execute(
                "UPDATE assets SET availability = 'available'
                 WHERE id IN (
                   SELECT asset_id FROM folder_group_assets
                   WHERE folder_group_id = ?1 AND last_seen_generation = ?2
                 )",
                params![group.as_uuid().as_bytes(), generation],
            )?;
            marked_missing = transaction.execute(
                "UPDATE assets SET availability = 'missing'
                 WHERE library_id = ?1 AND availability <> 'missing'
                   AND NOT EXISTS (
                     SELECT 1 FROM folder_group_assets fga
                     WHERE fga.asset_id = assets.id
                   )",
                [library.as_uuid().as_bytes()],
            )? as u64;
        }
        let completed = transaction.execute(
            "UPDATE scan_generations SET completed_at = unixepoch()
             WHERE library_id = ?1 AND folder_group_id = ?2
               AND generation = ?3 AND completed_at IS NULL",
            params![
                library.as_uuid().as_bytes(),
                group.as_uuid().as_bytes(),
                generation
            ],
        )?;
        if completed != 1 {
            return Err(CatalogError::InvalidData(format!(
                "scan generation {generation} is missing or already complete"
            )));
        }
        if source_online != 0 {
            transaction.execute(
                "UPDATE library_roots SET availability = 'available', last_seen_at = unixepoch()
                 WHERE id = ?1",
                [library.as_uuid().as_bytes()],
            )?;
        }
        transaction.commit()?;
        Ok(marked_missing)
    }

    pub fn link_derivative_group(
        &mut self,
        derivative: photo_domain::DerivativeId,
        group: FolderGroupId,
    ) -> Result<(), CatalogError> {
        let linked = self.connection.execute(
            "INSERT OR IGNORE INTO derivative_folder_groups (derivative_id, folder_group_id)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM derivatives WHERE id = ?1)",
            params![derivative.as_uuid().as_bytes(), group.as_uuid().as_bytes()],
        )?;
        if linked == 1 || self.connection.query_row(
            "SELECT 1 FROM derivative_folder_groups WHERE derivative_id = ?1 AND folder_group_id = ?2",
            params![derivative.as_uuid().as_bytes(), group.as_uuid().as_bytes()],
            |_| Ok(1_i64),
        ).optional()?.is_some() {
            Ok(())
        } else {
            Err(CatalogError::InvalidData(
                "derivative or folder group does not exist".into(),
            ))
        }
    }

    pub fn find_derivative_by_cache_key(
        &self,
        cache_key: &str,
    ) -> Result<Option<DerivativeRecord>, CatalogError> {
        self.connection
            .query_row(
                "SELECT id, asset_id, folder_group_id, kind, cache_key, relative_cache_path,
                        size_bytes, durable
                 FROM derivatives WHERE cache_key = ?1",
                [cache_key],
                decode_derivative,
            )
            .optional()
            .map_err(Into::into)
    }
}

pub(crate) fn add_asset_membership_on(
    connection: &rusqlite::Connection,
    group: FolderGroupId,
    asset: AssetId,
    generation: i64,
) -> Result<(), CatalogError> {
    connection.execute(
        "INSERT INTO folder_group_assets (folder_group_id, asset_id, last_seen_generation)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(folder_group_id, asset_id) DO UPDATE SET
           last_seen_generation = MAX(folder_group_assets.last_seen_generation,
                                      excluded.last_seen_generation)",
        params![
            group.as_uuid().as_bytes(),
            asset.as_uuid().as_bytes(),
            generation
        ],
    )?;
    Ok(())
}

fn ensure_group_library(
    connection: &rusqlite::Connection,
    library: LibraryId,
    group: FolderGroupId,
) -> Result<(), CatalogError> {
    let found: Option<Vec<u8>> = connection
        .query_row(
            "SELECT library_id FROM folder_groups WHERE id = ?1",
            [group.as_uuid().as_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    if found.as_deref() == Some(library.as_uuid().as_bytes()) {
        Ok(())
    } else {
        Err(CatalogError::InvalidData(
            "folder group does not belong to library".into(),
        ))
    }
}

fn decode_folder_group(row: &rusqlite::Row<'_>) -> Result<FolderGroupRecord, rusqlite::Error> {
    let id: Vec<u8> = row.get(0)?;
    let library: Vec<u8> = row.get(1)?;
    let relative_path: Vec<u8> = row.get(2)?;
    Ok(FolderGroupRecord {
        id: FolderGroupId::from_uuid(decode_uuid(id, 0)?),
        library_id: LibraryId::from_uuid(decode_uuid(library, 1)?),
        relative_path: RelativePathKey::from_bytes(relative_path).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Blob,
                Box::new(error),
            )
        })?,
        display_path: row.get(3)?,
    })
}
