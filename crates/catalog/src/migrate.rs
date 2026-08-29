use std::ffi::OsString;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::Connection;

use crate::CatalogError;
use crate::connection;

pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_catalog.sql"),
    include_str!("../migrations/0002_unavailable_assets.sql"),
    include_str!("../migrations/0003_app_state.sql"),
    include_str!("../migrations/0004_wall_projection.sql"),
    include_str!("../migrations/0005_group_scoped_generations.sql"),
    include_str!("../migrations/0006_wall_state_indexes.sql"),
    include_str!("../migrations/0007_derivative_coordinator.sql"),
    include_str!("../migrations/0008_gallery_scope.sql"),
];

pub(crate) fn migrate_with(path: &Path, migrations: &[&str]) -> Result<Connection, CatalogError> {
    let mut database = Connection::open(path)?;
    connection::ensure_safe_sqlite(&database)?;
    let old_version = user_version(&database)?;
    let needs_migration = old_version < migrations.len();
    let backup = if needs_migration && path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        let backup = backup_path(path, old_version);
        database.backup("main", &backup, None)?;
        Some(backup)
    } else {
        None
    };

    let result = connection::configure(&database, true)
        .and_then(|()| apply_migrations(&mut database, migrations));
    if let Err(error) = result {
        drop(database);
        if let Some(backup) = backup {
            restore_backup(path, &backup).map_err(|restore_error| {
                CatalogError::MigrationFailed(format!(
                    "{error}; restoring backup failed: {restore_error}"
                ))
            })?;
        }
        return Err(CatalogError::MigrationFailed(error.to_string()));
    }

    Ok(database)
}

pub(crate) fn apply_migrations(
    connection: &mut Connection,
    migrations: &[&str],
) -> Result<(), CatalogError> {
    let current = user_version(connection)?;
    if current > migrations.len() {
        return Err(CatalogError::MigrationFailed(format!(
            "database schema version {current} is newer than supported version {}",
            migrations.len()
        )));
    }

    for (offset, migration) in migrations.iter().skip(current).enumerate() {
        let version = current + offset + 1;
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        if version == 4 {
            normalize_legacy_capture_dates(&transaction)?;
        }
        if version == 8 {
            backfill_relative_parent_keys(&transaction)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

fn backfill_relative_parent_keys(
    connection: &rusqlite::Transaction<'_>,
) -> Result<(), CatalogError> {
    let mut statement = connection.prepare("SELECT id, relative_path_key FROM assets")?;
    let values = statement
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    for (id, relative_path) in values {
        let relative = photo_domain::RelativePathKey::from_bytes(relative_path)
            .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
        let path = relative
            .to_path_buf()
            .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let parent_key = photo_domain::RelativePathKey::from_relative_path(parent)
            .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
        connection.execute(
            "UPDATE assets SET relative_parent_key = ?2 WHERE id = ?1",
            rusqlite::params![id, parent_key.as_bytes()],
        )?;
    }
    Ok(())
}

fn normalize_legacy_capture_dates(
    connection: &rusqlite::Transaction<'_>,
) -> Result<(), CatalogError> {
    let mut statement = connection
        .prepare("SELECT id, captured_at_utc FROM assets WHERE captured_at_utc IS NOT NULL")?;
    let values = statement
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    for (id, value) in values {
        let Ok(parsed) = DateTime::<FixedOffset>::parse_from_rfc3339(&value) else {
            continue;
        };
        let normalized = parsed.with_timezone(&Utc).to_rfc3339();
        if normalized != value {
            connection.execute(
                "UPDATE assets SET captured_at_utc = ?2 WHERE id = ?1",
                rusqlite::params![id, normalized],
            )?;
        }
    }
    Ok(())
}

fn user_version(connection: &Connection) -> Result<usize, CatalogError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    usize::try_from(version).map_err(|_| CatalogError::InvalidData("negative user_version".into()))
}

fn backup_path(path: &Path, version: usize) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| OsString::from("catalog.sqlite"), OsString::from);
    name.push(format!(".backup-v{version}"));
    path.with_file_name(name)
}

fn restore_backup(path: &Path, backup: &Path) -> Result<(), std::io::Error> {
    remove_sqlite_sidecar(path, "-wal")?;
    remove_sqlite_sidecar(path, "-shm")?;

    let mut temp_name = path
        .file_name()
        .map_or_else(|| OsString::from("catalog.sqlite"), OsString::from);
    temp_name.push(format!(".restore-{}", uuid::Uuid::new_v4()));
    let temp = path.with_file_name(temp_name);
    std::fs::copy(backup, &temp)?;

    #[cfg(unix)]
    std::fs::rename(temp, path)?;

    #[cfg(windows)]
    {
        let mut failed_name = path
            .file_name()
            .map_or_else(|| OsString::from("catalog.sqlite"), OsString::from);
        failed_name.push(format!(".failed-{}", uuid::Uuid::new_v4()));
        let failed = path.with_file_name(failed_name);
        std::fs::rename(path, &failed)?;
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::rename(&failed, path);
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        std::fs::remove_file(failed)?;
    }

    Ok(())
}

fn remove_sqlite_sidecar(path: &Path, suffix: &str) -> Result<(), std::io::Error> {
    let mut sidecar_name = path
        .file_name()
        .map_or_else(|| OsString::from("catalog.sqlite"), OsString::from);
    sidecar_name.push(suffix);
    let sidecar = path.with_file_name(sidecar_name);
    match std::fs::remove_file(sidecar) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::migrate_with;

    #[test]
    fn failed_migration_restores_the_pre_migration_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("catalog.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel(value TEXT);\
                 INSERT INTO sentinel VALUES ('safe');\
                 PRAGMA user_version=0;",
            )
            .unwrap();
        drop(connection);

        assert!(migrate_with(&path, &["BROKEN SQL"]).is_err());

        let restored = Connection::open(&path).unwrap();
        let value: String = restored
            .query_row("SELECT value FROM sentinel", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "safe");
    }
}
