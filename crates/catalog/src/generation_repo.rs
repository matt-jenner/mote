use photo_domain::LibraryId;
use rusqlite::params;

use crate::asset_repo::upsert_asset_on;
use crate::{AssetRecord, Catalog, CatalogError, NewAsset};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationCompletion {
    pub marked_missing: u64,
}

impl Catalog {
    pub fn begin_generation(&mut self, library: LibraryId) -> Result<u64, CatalogError> {
        let transaction = self.connection.transaction()?;
        let current: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(generation), 0) FROM scan_generations WHERE library_id = ?1",
            [library.as_uuid().as_bytes()],
            |row| row.get(0),
        )?;
        let generation = current
            .checked_add(1)
            .ok_or(CatalogError::ValueOutOfRange)?;
        transaction.execute(
            "INSERT INTO scan_generations \
                (library_id, generation, started_at, source_was_online) \
             VALUES (?1, ?2, unixepoch(), 1)",
            params![library.as_uuid().as_bytes(), generation],
        )?;
        transaction.commit()?;
        u64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn record_generation_assets(
        &mut self,
        library: LibraryId,
        generation: u64,
        assets: &[NewAsset],
    ) -> Result<(), CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        for asset in assets {
            if asset.library_id != library {
                return Err(CatalogError::InvalidData(
                    "generation batch contains an asset from another library".into(),
                ));
            }
            upsert_asset_on(&transaction, asset)?;
            transaction.execute(
                "UPDATE assets SET last_seen_generation = ?2, availability = 'available' \
                 WHERE id = ?1",
                params![asset.id.as_uuid().as_bytes(), generation],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn complete_generation(
        &mut self,
        library: LibraryId,
        generation: u64,
    ) -> Result<GenerationCompletion, CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE assets SET availability = 'available' \
             WHERE library_id = ?1 AND last_seen_generation = ?2",
            params![library.as_uuid().as_bytes(), generation],
        )?;
        let marked_missing = transaction.execute(
            "UPDATE assets SET availability = 'missing' \
             WHERE library_id = ?1 AND last_seen_generation <> ?2 AND availability <> 'missing'",
            params![library.as_uuid().as_bytes(), generation],
        )?;
        let completed = transaction.execute(
            "UPDATE scan_generations SET completed_at = unixepoch() \
             WHERE library_id = ?1 AND generation = ?2 AND completed_at IS NULL",
            params![library.as_uuid().as_bytes(), generation],
        )?;
        if completed != 1 {
            return Err(CatalogError::InvalidData(format!(
                "scan generation {generation} is missing or already complete"
            )));
        }
        transaction.execute(
            "UPDATE library_roots SET availability = 'available', last_seen_at = unixepoch() \
             WHERE id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        transaction.commit()?;
        Ok(GenerationCompletion {
            marked_missing: marked_missing as u64,
        })
    }

    pub fn mark_root_offline(&mut self, library: LibraryId) -> Result<u64, CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE library_roots SET availability = 'root_offline' WHERE id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        let retained = transaction.execute(
            "UPDATE assets SET availability = 'root_offline' WHERE library_id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        transaction.commit()?;
        Ok(retained as u64)
    }

    pub fn asset_count(&self, library: LibraryId) -> Result<u64, CatalogError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM assets WHERE library_id = ?1",
            [library.as_uuid().as_bytes()],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn assets(&self, library: LibraryId) -> Result<Vec<AssetRecord>, CatalogError> {
        self.list_assets_page(library, None, u32::MAX)
    }
}
