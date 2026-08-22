use std::time::Duration;

use rusqlite::Connection;

use crate::{CatalogError, SqliteVersion};

const MINIMUM_SQLITE_VERSION: SqliteVersion = SqliteVersion::new(3, 51, 3);

pub(crate) fn configure(connection: &Connection, file_backed: bool) -> Result<(), CatalogError> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    if file_backed {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}

pub(crate) fn ensure_safe_sqlite(connection: &Connection) -> Result<(), CatalogError> {
    let found = sqlite_version(connection)?;
    if found < MINIMUM_SQLITE_VERSION {
        return Err(CatalogError::UnsafeSqliteVersion {
            found,
            minimum: MINIMUM_SQLITE_VERSION,
        });
    }
    Ok(())
}

pub(crate) fn sqlite_version(connection: &Connection) -> Result<SqliteVersion, CatalogError> {
    let raw: String = connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let mut parts = raw.split('.');
    let parse = |value: Option<&str>| value.and_then(|part| part.parse::<u32>().ok());
    let Some(major) = parse(parts.next()) else {
        return Err(CatalogError::InvalidSqliteVersion(raw));
    };
    let Some(minor) = parse(parts.next()) else {
        return Err(CatalogError::InvalidSqliteVersion(raw));
    };
    let Some(patch) = parse(parts.next()) else {
        return Err(CatalogError::InvalidSqliteVersion(raw));
    };

    Ok(SqliteVersion::new(major, minor, patch))
}
