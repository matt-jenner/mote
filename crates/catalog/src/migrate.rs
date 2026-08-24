use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::CatalogError;
use crate::connection;

pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_catalog.sql"),
    include_str!("../migrations/0002_unavailable_assets.sql"),
    include_str!("../migrations/0003_app_state.sql"),
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

    for migration in migrations.iter().skip(current) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        transaction.commit()?;
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
