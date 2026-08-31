use photo_domain::{FolderGroupId, LibraryId};
use rusqlite::{OptionalExtension, params};

use crate::asset_repo::upsert_asset_on;
use crate::{AssetRecord, Catalog, CatalogError, NewAsset};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationCompletion {
    pub marked_missing: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FolderGroupRecoveryState {
    pub requested: u64,
    pub reconciled: u64,
}

impl Catalog {
    pub fn has_completed_generation_for_library(
        &self,
        library: LibraryId,
    ) -> Result<bool, CatalogError> {
        let completed: Option<i64> = self.connection.query_row(
            "SELECT completed_at FROM scan_generations WHERE library_id = ?1 AND completed_at IS NOT NULL ORDER BY generation DESC LIMIT 1",
            params![library.as_uuid().as_bytes()],
            |row| row.get(0),
        ).optional()?;
        Ok(completed.is_some())
    }

    pub fn has_completed_generation_for_group(
        &self,
        library: LibraryId,
        folder_group: FolderGroupId,
    ) -> Result<bool, CatalogError> {
        let completed: Option<i64> = self
            .connection
            .query_row(
                "SELECT completed_at FROM scan_generations \
             WHERE library_id = ?1 AND folder_group_id = ?2 AND completed_at IS NOT NULL \
             ORDER BY generation DESC LIMIT 1",
                params![
                    library.as_uuid().as_bytes(),
                    folder_group.as_uuid().as_bytes()
                ],
                |row| row.get(0),
            )
            .optional()?;
        Ok(completed.is_some())
    }

    /// Legacy library-wide query retained for callers and pre-v5 generations.
    pub fn has_completed_generation(
        &self,
        library: LibraryId,
        generation: u64,
    ) -> Result<bool, CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let completed: Option<i64> = self
            .connection
            .query_row(
                "SELECT completed_at FROM scan_generations \
             WHERE library_id = ?1 AND generation = ?2",
                params![library.as_uuid().as_bytes(), generation],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        Ok(completed.is_some())
    }

    pub fn has_completed_group_generation(
        &self,
        library: LibraryId,
        folder_group: FolderGroupId,
        generation: u64,
    ) -> Result<bool, CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let completed: Option<i64> = self
            .connection
            .query_row(
                "SELECT completed_at FROM scan_generations \
             WHERE library_id = ?1 AND folder_group_id = ?2 AND generation = ?3",
                params![
                    library.as_uuid().as_bytes(),
                    folder_group.as_uuid().as_bytes(),
                    generation
                ],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        Ok(completed.is_some())
    }

    pub fn begin_generation(&mut self, library: LibraryId) -> Result<u64, CatalogError> {
        self.begin_generation_record(library, None, 0)
    }

    pub fn begin_generation_for_group(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
    ) -> Result<u64, CatalogError> {
        self.begin_generation_record(library, Some(folder_group), 0)
    }

    pub fn begin_generation_for_group_at_recovery(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
        recovery_token: u64,
    ) -> Result<u64, CatalogError> {
        self.begin_generation_record(library, Some(folder_group), recovery_token)
    }

    fn begin_generation_record(
        &mut self,
        library: LibraryId,
        folder_group: Option<FolderGroupId>,
        recovery_token: u64,
    ) -> Result<u64, CatalogError> {
        let recovery_token =
            i64::try_from(recovery_token).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        if let Some(folder_group) = folder_group {
            ensure_group_belongs_to_library(&transaction, library, folder_group)?;
            let requested: i64 = transaction.query_row(
                "SELECT recovery_requested FROM folder_groups WHERE id = ?1",
                [folder_group.as_uuid().as_bytes()],
                |row| row.get(0),
            )?;
            if recovery_token > requested {
                return Err(CatalogError::InvalidData(
                    "scan recovery token is newer than the folder request".to_owned(),
                ));
            }
        } else if recovery_token != 0 {
            return Err(CatalogError::InvalidData(
                "library-wide generations cannot carry a folder recovery token".to_owned(),
            ));
        }
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
                (library_id, folder_group_id, generation, started_at, source_was_online, recovery_token) \
             VALUES (?1, ?2, ?3, unixepoch(), 1, ?4)",
            params![
                library.as_uuid().as_bytes(),
                folder_group.map(|group| group.as_uuid().as_bytes().to_vec()),
                generation,
                recovery_token,
            ],
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
                 WHERE id = ?1 AND library_id = ?3",
                params![
                    asset.id.as_uuid().as_bytes(),
                    generation,
                    library.as_uuid().as_bytes()
                ],
            )?;
            if let Some(group) = asset.folder_group_id {
                crate::selection_repo::add_asset_membership_on(
                    &transaction,
                    group,
                    asset.id,
                    generation,
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_generation_assets_for_group(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
        generation: u64,
        assets: &[NewAsset],
    ) -> Result<(), CatalogError> {
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        ensure_generation_group(&transaction, library, folder_group, generation)?;
        for asset in assets {
            if asset.library_id != library || asset.folder_group_id != Some(folder_group) {
                return Err(CatalogError::InvalidData(
                    "generation batch contains an asset from another library or folder group"
                        .into(),
                ));
            }
            upsert_asset_on(&transaction, asset)?;
            transaction.execute(
                "UPDATE assets SET last_seen_generation = ?2, availability = 'available' \
                 WHERE id = ?1 AND library_id = ?3 AND folder_group_id = ?4",
                params![
                    asset.id.as_uuid().as_bytes(),
                    generation,
                    library.as_uuid().as_bytes(),
                    folder_group.as_uuid().as_bytes()
                ],
            )?;
            crate::selection_repo::add_asset_membership_on(
                &transaction,
                folder_group,
                asset.id,
                generation,
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
        self.complete_generation_record(library, None, generation, 0)
    }

    pub fn complete_generation_for_group(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
        generation: u64,
    ) -> Result<GenerationCompletion, CatalogError> {
        self.complete_generation_record(library, Some(folder_group), generation, 0)
    }

    pub fn complete_generation_for_group_at_recovery(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
        generation: u64,
        recovery_token: u64,
    ) -> Result<GenerationCompletion, CatalogError> {
        self.complete_generation_record(library, Some(folder_group), generation, recovery_token)
    }

    fn complete_generation_record(
        &mut self,
        library: LibraryId,
        folder_group: Option<FolderGroupId>,
        generation: u64,
        recovery_token: u64,
    ) -> Result<GenerationCompletion, CatalogError> {
        if let Some(folder_group) = folder_group {
            let marked_missing = self.finish_group_generation_count_at_recovery(
                library,
                folder_group,
                generation,
                recovery_token,
            )?;
            return Ok(GenerationCompletion { marked_missing });
        }
        if recovery_token != 0 {
            return Err(CatalogError::InvalidData(
                "library-wide completion cannot carry a folder recovery token".to_owned(),
            ));
        }
        let generation = i64::try_from(generation).map_err(|_| CatalogError::ValueOutOfRange)?;
        let transaction = self.connection.transaction()?;
        let source_online: i64 = transaction
            .query_row(
                "SELECT source_was_online FROM scan_generations
                 WHERE library_id = ?1 AND folder_group_id IS NULL AND generation = ?2",
                params![library.as_uuid().as_bytes(), generation],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                CatalogError::InvalidData(format!("scan generation {generation} is missing"))
            })?;
        let marked_missing = if source_online != 0 {
            transaction.execute(
                "UPDATE assets SET availability = 'available' \
                 WHERE library_id = ?1 AND last_seen_generation = ?2",
                params![library.as_uuid().as_bytes(), generation],
            )?;
            transaction.execute(
                "UPDATE assets SET availability = 'missing'
                 WHERE library_id = ?1 AND last_seen_generation <> ?2
                   AND availability <> 'missing'",
                params![library.as_uuid().as_bytes(), generation],
            )?
        } else {
            0
        };
        let completed = transaction.execute(
            "UPDATE scan_generations SET completed_at = unixepoch() \
             WHERE library_id = ?1 AND folder_group_id IS NULL \
               AND generation = ?2 AND completed_at IS NULL",
            params![library.as_uuid().as_bytes(), generation],
        )?;
        if completed != 1 {
            return Err(CatalogError::InvalidData(format!(
                "scan generation {generation} is missing or already complete"
            )));
        }
        if source_online != 0 {
            transaction.execute(
                "UPDATE library_roots SET availability = 'available', last_seen_at = unixepoch() \
                 WHERE id = ?1",
                [library.as_uuid().as_bytes()],
            )?;
        }
        transaction.commit()?;
        Ok(GenerationCompletion {
            marked_missing: marked_missing as u64,
        })
    }

    pub fn mark_root_offline(&mut self, library: LibraryId) -> Result<u64, CatalogError> {
        let transaction = self.connection.transaction()?;
        let previous: String = transaction
            .query_row(
                "SELECT availability FROM library_roots WHERE id = ?1",
                [library.as_uuid().as_bytes()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| CatalogError::InvalidData("library does not exist".to_owned()))?;
        if previous != "root_offline" {
            ensure_recovery_increment_available(&transaction, library, None)?;
            transaction.execute(
                "UPDATE folder_groups
                 SET recovery_requested = recovery_requested + 1
                 WHERE library_id = ?1",
                [library.as_uuid().as_bytes()],
            )?;
        }
        transaction.execute(
            "UPDATE library_roots SET availability = 'root_offline' WHERE id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        let retained = transaction.execute(
            "UPDATE assets SET availability = 'root_offline' WHERE library_id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        transaction.execute(
            "UPDATE scan_generations SET source_was_online = 0
             WHERE library_id = ?1 AND completed_at IS NULL",
            [library.as_uuid().as_bytes()],
        )?;
        transaction.commit()?;
        Ok(retained as u64)
    }

    pub fn mark_group_offline(
        &mut self,
        library: LibraryId,
        folder_group: FolderGroupId,
    ) -> Result<u64, CatalogError> {
        let transaction = self.connection.transaction()?;
        ensure_group_belongs_to_library(&transaction, library, folder_group)?;
        let (requested, reconciled): (i64, i64) = transaction.query_row(
            "SELECT recovery_requested, recovery_reconciled
             FROM folder_groups WHERE id = ?1",
            [folder_group.as_uuid().as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if requested == reconciled {
            ensure_recovery_increment_available(&transaction, library, Some(folder_group))?;
            transaction.execute(
                "UPDATE folder_groups SET recovery_requested = recovery_requested + 1
                 WHERE id = ?1 AND library_id = ?2",
                params![
                    folder_group.as_uuid().as_bytes(),
                    library.as_uuid().as_bytes()
                ],
            )?;
        }
        let retained = transaction.execute(
            "UPDATE assets SET availability = 'root_offline'
             WHERE library_id = ?1 AND EXISTS (
               SELECT 1 FROM folder_group_assets fga
               WHERE fga.asset_id = assets.id AND fga.folder_group_id = ?2
             )",
            params![
                library.as_uuid().as_bytes(),
                folder_group.as_uuid().as_bytes()
            ],
        )?;
        transaction.execute(
            "UPDATE scan_generations SET source_was_online = 0
             WHERE library_id = ?1 AND folder_group_id = ?2 AND completed_at IS NULL",
            params![
                library.as_uuid().as_bytes(),
                folder_group.as_uuid().as_bytes()
            ],
        )?;
        transaction.commit()?;
        Ok(retained as u64)
    }

    pub fn folder_group_recovery_state(
        &self,
        library: LibraryId,
        folder_group: FolderGroupId,
    ) -> Result<FolderGroupRecoveryState, CatalogError> {
        ensure_group_belongs_to_library(&self.connection, library, folder_group)?;
        let (requested, reconciled): (i64, i64) = self.connection.query_row(
            "SELECT recovery_requested, recovery_reconciled
             FROM folder_groups WHERE id = ?1",
            [folder_group.as_uuid().as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(FolderGroupRecoveryState {
            requested: u64::try_from(requested).map_err(|_| {
                CatalogError::InvalidData("negative requested recovery token".to_owned())
            })?,
            reconciled: u64::try_from(reconciled).map_err(|_| {
                CatalogError::InvalidData("negative reconciled recovery token".to_owned())
            })?,
        })
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

fn ensure_recovery_increment_available(
    connection: &rusqlite::Connection,
    library: LibraryId,
    folder_group: Option<FolderGroupId>,
) -> Result<(), CatalogError> {
    let at_limit: bool = match folder_group {
        Some(folder_group) => connection.query_row(
            "SELECT recovery_requested = ?3 FROM folder_groups
             WHERE id = ?1 AND library_id = ?2",
            params![
                folder_group.as_uuid().as_bytes(),
                library.as_uuid().as_bytes(),
                i64::MAX,
            ],
            |row| row.get(0),
        )?,
        None => connection.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM folder_groups
               WHERE library_id = ?1 AND recovery_requested = ?2
             )",
            params![library.as_uuid().as_bytes(), i64::MAX],
            |row| row.get(0),
        )?,
    };
    if at_limit {
        Err(CatalogError::ValueOutOfRange)
    } else {
        Ok(())
    }
}

fn ensure_group_belongs_to_library(
    connection: &rusqlite::Connection,
    library: LibraryId,
    folder_group: FolderGroupId,
) -> Result<(), CatalogError> {
    let belongs: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM folder_groups WHERE id = ?1 AND library_id = ?2",
            params![
                folder_group.as_uuid().as_bytes(),
                library.as_uuid().as_bytes()
            ],
            |row| row.get(0),
        )
        .optional()?;
    if belongs.is_some() {
        Ok(())
    } else {
        Err(CatalogError::InvalidData(
            "folder group does not belong to library".into(),
        ))
    }
}

fn ensure_generation_group(
    connection: &rusqlite::Connection,
    library: LibraryId,
    folder_group: FolderGroupId,
    generation: i64,
) -> Result<(), CatalogError> {
    ensure_group_belongs_to_library(connection, library, folder_group)?;
    let matches: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM scan_generations \
             WHERE library_id = ?1 AND folder_group_id = ?2 AND generation = ?3",
            params![
                library.as_uuid().as_bytes(),
                folder_group.as_uuid().as_bytes(),
                generation
            ],
            |row| row.get(0),
        )
        .optional()?;
    if matches.is_some() {
        Ok(())
    } else {
        Err(CatalogError::InvalidData(format!(
            "scan generation {generation} is missing for folder group"
        )))
    }
}
