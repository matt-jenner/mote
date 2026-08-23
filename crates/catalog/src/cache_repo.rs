use std::path::{Component, PathBuf};

use photo_domain::{AssetId, DerivativeId, FolderGroupId, LibraryId, RelativePathKey};
use rusqlite::{OptionalExtension, params};

use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewFolderGroup {
    pub id: FolderGroupId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
    pub last_viewed_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewDerivative {
    pub id: DerivativeId,
    pub asset_id: AssetId,
    pub folder_group_id: FolderGroupId,
    pub kind: String,
    pub cache_key: String,
    pub relative_cache_path: PathBuf,
    pub size_bytes: u64,
    pub durable: bool,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheEvictionGroup {
    pub id: FolderGroupId,
    pub reclaimable_bytes: u64,
    pub last_viewed_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivativeRecord {
    pub id: DerivativeId,
    pub folder_group_id: FolderGroupId,
    pub relative_cache_path: PathBuf,
    pub size_bytes: u64,
    pub durable: bool,
}

impl Catalog {
    pub fn upsert_folder_group(
        &mut self,
        value: &NewFolderGroup,
    ) -> Result<FolderGroupId, CatalogError> {
        let id = self.connection.query_row(
            "INSERT INTO folder_groups (id, library_id, relative_path_key, display_path, last_viewed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(library_id, relative_path_key) DO UPDATE SET \
                display_path = excluded.display_path, last_viewed_at = excluded.last_viewed_at \
             RETURNING id",
            params![
                value.id.as_uuid().as_bytes(),
                value.library_id.as_uuid().as_bytes(),
                value.relative_path.as_bytes(),
                value.display_path,
                value.last_viewed_at,
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        Ok(FolderGroupId::from_uuid(decode_uuid(id, 0)?))
    }

    pub fn insert_derivative(&mut self, value: &NewDerivative) -> Result<(), CatalogError> {
        if value.relative_cache_path.as_os_str().is_empty()
            || value.relative_cache_path.is_absolute()
            || value
                .relative_cache_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(CatalogError::InvalidData(
                "cache path must remain relative to the cache root".to_owned(),
            ));
        }
        let relative_cache_path = value.relative_cache_path.to_str().ok_or_else(|| {
            CatalogError::InvalidData("cache path is not valid Unicode".to_owned())
        })?;
        let size_bytes =
            i64::try_from(value.size_bytes).map_err(|_| CatalogError::ValueOutOfRange)?;
        self.connection.execute(
            "INSERT INTO derivatives (id, asset_id, folder_group_id, kind, cache_key, \
                relative_cache_path, size_bytes, durable, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                value.id.as_uuid().as_bytes(),
                value.asset_id.as_uuid().as_bytes(),
                value.folder_group_id.as_uuid().as_bytes(),
                value.kind,
                value.cache_key,
                relative_cache_path,
                size_bytes,
                value.durable,
                value.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn cache_eviction_groups(&self) -> Result<Vec<CacheEvictionGroup>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT g.id, SUM(d.size_bytes), g.last_viewed_at \
             FROM folder_groups g \
             JOIN derivatives d ON d.folder_group_id = g.id AND d.durable = 0 \
             GROUP BY g.id, g.last_viewed_at \
             ORDER BY g.last_viewed_at IS NOT NULL, g.last_viewed_at, g.id",
        )?;
        statement
            .query_map([], |row| {
                let id: Vec<u8> = row.get(0)?;
                let bytes: i64 = row.get(1)?;
                Ok(CacheEvictionGroup {
                    id: FolderGroupId::from_uuid(decode_uuid(id, 0)?),
                    reclaimable_bytes: u64::try_from(bytes).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    last_viewed_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn non_durable_derivatives(
        &self,
        groups: &[FolderGroupId],
    ) -> Result<Vec<DerivativeRecord>, CatalogError> {
        let mut records = Vec::new();
        let mut statement = self.connection.prepare(
            "SELECT id, folder_group_id, relative_cache_path, size_bytes, durable \
             FROM derivatives WHERE folder_group_id = ?1 AND durable = 0 ORDER BY id",
        )?;
        for group in groups {
            let rows = statement.query_map([group.as_uuid().as_bytes()], decode_derivative)?;
            records.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }
        Ok(records)
    }

    pub fn all_derivatives(&self) -> Result<Vec<DerivativeRecord>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT id, folder_group_id, relative_cache_path, size_bytes, durable \
             FROM derivatives ORDER BY id",
        )?;
        statement
            .query_map([], decode_derivative)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn delete_derivatives(
        &mut self,
        derivatives: &[DerivativeId],
    ) -> Result<usize, CatalogError> {
        let transaction = self.connection.transaction()?;
        let mut removed = 0;
        {
            let mut statement = transaction.prepare("DELETE FROM derivatives WHERE id = ?1")?;
            for id in derivatives {
                removed += statement.execute([id.as_uuid().as_bytes()])?;
            }
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn derivative_count(
        &self,
        group: FolderGroupId,
        durable: bool,
    ) -> Result<u64, CatalogError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM derivatives WHERE folder_group_id = ?1 AND durable = ?2",
            params![group.as_uuid().as_bytes(), durable],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn folder_group_for_path(
        &self,
        library: LibraryId,
        path: &RelativePathKey,
    ) -> Result<Option<FolderGroupId>, CatalogError> {
        let id = self
            .connection
            .query_row(
                "SELECT id FROM folder_groups WHERE library_id = ?1 AND relative_path_key = ?2",
                params![library.as_uuid().as_bytes(), path.as_bytes()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        id.map(|bytes| decode_uuid(bytes, 0).map(FolderGroupId::from_uuid))
            .transpose()
            .map_err(Into::into)
    }
}

fn decode_derivative(row: &rusqlite::Row<'_>) -> Result<DerivativeRecord, rusqlite::Error> {
    let id: Vec<u8> = row.get(0)?;
    let group: Vec<u8> = row.get(1)?;
    let size_bytes: i64 = row.get(3)?;
    Ok(DerivativeRecord {
        id: DerivativeId::from_uuid(decode_uuid(id, 0)?),
        folder_group_id: FolderGroupId::from_uuid(decode_uuid(group, 1)?),
        relative_cache_path: PathBuf::from(row.get::<_, String>(2)?),
        size_bytes: u64::try_from(size_bytes).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        durable: row.get(4)?,
    })
}
