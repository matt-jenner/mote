use photo_domain::{AssetId, FolderGroupId, NativePathKey};
use rusqlite::{TransactionBehavior, params};

use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoPickRecord {
    pub asset_id: AssetId,
    pub folder_group_id: FolderGroupId,
    pub position: u64,
}

impl Catalog {
    pub fn photo_pick_revision(&self) -> Result<u64, CatalogError> {
        let revision: i64 = self.connection.query_row(
            "SELECT revision FROM photo_pick_preferences WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(revision).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn list_photo_picks(&self) -> Result<Vec<PhotoPickRecord>, CatalogError> {
        list_photo_picks_on(&self.connection)
    }

    pub fn add_photo_pick(
        &mut self,
        asset: AssetId,
        group: FolderGroupId,
    ) -> Result<bool, CatalogError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let added = transaction.execute(
            "INSERT OR IGNORE INTO photo_picks (position, asset_id, folder_group_id)
             VALUES (COALESCE((SELECT MAX(position) + 1 FROM photo_picks), 0), ?1, ?2)",
            params![asset.as_uuid().as_bytes(), group.as_uuid().as_bytes()],
        )? == 1;
        if added {
            increment_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(added)
    }

    pub fn remove_photo_pick(&mut self, asset: AssetId) -> Result<bool, CatalogError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = transaction.execute(
            "DELETE FROM photo_picks WHERE asset_id = ?1",
            [asset.as_uuid().as_bytes()],
        )? == 1;
        if removed {
            increment_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn clear_photo_picks(&mut self) -> Result<Vec<PhotoPickRecord>, CatalogError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cleared = list_photo_picks_on(&transaction)?;
        if !cleared.is_empty() {
            transaction.execute("DELETE FROM photo_picks", [])?;
            increment_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(cleared)
    }

    pub fn restore_photo_picks(&mut self, cleared: &[PhotoPickRecord]) -> Result<(), CatalogError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let added_since_clear = list_photo_picks_on(&transaction)?;
        let restored = merge_photo_picks(cleared, &added_since_clear);

        if restored != added_since_clear {
            transaction.execute("DELETE FROM photo_picks", [])?;
            for (position, pick) in restored.iter().enumerate() {
                let position =
                    i64::try_from(position).map_err(|_| CatalogError::ValueOutOfRange)?;
                transaction.execute(
                    "INSERT INTO photo_picks (position, asset_id, folder_group_id) VALUES (?1, ?2, ?3)",
                    params![
                        position,
                        pick.asset_id.as_uuid().as_bytes(),
                        pick.folder_group_id.as_uuid().as_bytes(),
                    ],
                )?;
            }
            increment_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn last_copy_destination(&self) -> Result<Option<NativePathKey>, CatalogError> {
        let value: Option<Vec<u8>> = self.connection.query_row(
            "SELECT last_copy_destination FROM photo_pick_preferences WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        value
            .map(NativePathKey::from_bytes)
            .transpose()
            .map_err(|error| CatalogError::InvalidData(error.to_string()))
    }

    pub fn set_last_copy_destination(&mut self, value: &NativePathKey) -> Result<(), CatalogError> {
        let changed = self.connection.execute(
            "UPDATE photo_pick_preferences SET last_copy_destination = ?1 WHERE singleton = 1",
            [value.as_bytes()],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(CatalogError::InvalidData(
                "photo pick preferences do not exist".into(),
            ))
        }
    }
}

fn list_photo_picks_on(
    connection: &rusqlite::Connection,
) -> Result<Vec<PhotoPickRecord>, CatalogError> {
    connection
        .prepare("SELECT asset_id, folder_group_id, position FROM photo_picks ORDER BY position")?
        .query_map([], decode_photo_pick)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn decode_photo_pick(row: &rusqlite::Row<'_>) -> Result<PhotoPickRecord, rusqlite::Error> {
    let position: i64 = row.get(2)?;
    let position = u64::try_from(position).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(PhotoPickRecord {
        asset_id: AssetId::from_uuid(decode_uuid(row.get(0)?, 0)?),
        folder_group_id: FolderGroupId::from_uuid(decode_uuid(row.get(1)?, 1)?),
        position,
    })
}

fn increment_revision(connection: &rusqlite::Connection) -> Result<(), CatalogError> {
    let changed = connection.execute(
        "UPDATE photo_pick_preferences SET revision = revision + 1 WHERE singleton = 1",
        [],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CatalogError::InvalidData(
            "photo pick preferences do not exist".into(),
        ))
    }
}

fn merge_photo_picks(
    cleared: &[PhotoPickRecord],
    added_since_clear: &[PhotoPickRecord],
) -> Vec<PhotoPickRecord> {
    let mut restored = Vec::with_capacity(cleared.len() + added_since_clear.len());
    for pick in cleared.iter().chain(added_since_clear) {
        if restored
            .iter()
            .all(|existing: &PhotoPickRecord| existing.asset_id != pick.asset_id)
        {
            restored.push(pick.clone());
        }
    }
    restored
}
