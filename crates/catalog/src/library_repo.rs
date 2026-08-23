use std::path::Path;

use photo_domain::{Availability, LibraryId, LibraryKind, NativePathKey};
use rusqlite::{Connection, OpenFlags, params};

use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewLibrary {
    pub id: LibraryId,
    pub kind: LibraryKind,
    pub display_name: String,
    pub canonical_root_key: NativePathKey,
    pub display_path: String,
}

impl NewLibrary {
    pub fn configured(display_name: impl Into<String>, canonical_root: &Path) -> Self {
        Self::new(LibraryKind::Configured, display_name, canonical_root)
    }

    pub fn recent(display_name: impl Into<String>, canonical_root: &Path) -> Self {
        Self::new(LibraryKind::Recent, display_name, canonical_root)
    }

    fn new(kind: LibraryKind, display_name: impl Into<String>, canonical_root: &Path) -> Self {
        Self {
            id: LibraryId::new(),
            kind,
            display_name: display_name.into(),
            canonical_root_key: NativePathKey::from_path(canonical_root),
            display_path: canonical_root.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryRootRecord {
    pub id: LibraryId,
    pub kind: LibraryKind,
    pub display_name: String,
    pub canonical_root_key: NativePathKey,
    pub display_path: String,
    pub availability: Availability,
    pub last_seen_at: Option<i64>,
}

impl Catalog {
    pub fn read_library_root_paths(path: &Path) -> Result<Vec<std::path::PathBuf>, CatalogError> {
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        crate::connection::ensure_safe_sqlite(&connection)?;
        let has_roots: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'library_roots')",
            [],
            |row| row.get(0),
        )?;
        if !has_roots {
            return Ok(Vec::new());
        }
        let mut statement =
            connection.prepare("SELECT canonical_root_key FROM library_roots ORDER BY id")?;
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .map(|bytes| {
                let key = NativePathKey::from_bytes(bytes?).map_err(|error| {
                    CatalogError::InvalidData(format!("invalid cataloged source root: {error}"))
                })?;
                key.to_path_buf().map_err(|error| {
                    CatalogError::InvalidData(format!("invalid cataloged source root: {error}"))
                })
            })
            .collect()
    }

    pub fn add_library(&mut self, value: &NewLibrary) -> Result<LibraryRootRecord, CatalogError> {
        self.connection.execute(
            "INSERT INTO library_roots (id, kind, display_name, canonical_root_key, display_path, availability) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'available')",
            params![
                value.id.as_uuid().as_bytes(),
                encode_library_kind(value.kind),
                value.display_name,
                value.canonical_root_key.as_bytes(),
                value.display_path,
            ],
        )?;

        Ok(LibraryRootRecord {
            id: value.id,
            kind: value.kind,
            display_name: value.display_name.clone(),
            canonical_root_key: value.canonical_root_key.clone(),
            display_path: value.display_path.clone(),
            availability: Availability::Available,
            last_seen_at: None,
        })
    }

    pub fn list_libraries(&self) -> Result<Vec<LibraryRootRecord>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, display_name, canonical_root_key, display_path, availability, last_seen_at \
             FROM library_roots ORDER BY display_name, id",
        )?;
        let records = statement
            .query_map([], decode_library)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn find_library(&self, id: LibraryId) -> Result<Option<LibraryRootRecord>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, display_name, canonical_root_key, display_path, availability, last_seen_at \
             FROM library_roots WHERE id = ?1",
        )?;
        let mut rows = statement.query([id.as_uuid().as_bytes()])?;
        rows.next()?
            .map(decode_library)
            .transpose()
            .map_err(Into::into)
    }

    pub fn promote_library(
        &mut self,
        id: LibraryId,
        display_name: &str,
    ) -> Result<LibraryRootRecord, CatalogError> {
        self.connection.execute(
            "UPDATE library_roots SET kind = 'configured', display_name = ?2 WHERE id = ?1",
            params![id.as_uuid().as_bytes(), display_name],
        )?;
        self.find_library(id)?.ok_or_else(|| {
            CatalogError::InvalidData(format!("library {} does not exist", id.as_uuid()))
        })
    }

    pub fn set_library_availability(
        &mut self,
        id: LibraryId,
        availability: Availability,
    ) -> Result<(), CatalogError> {
        let changed = self.connection.execute(
            "UPDATE library_roots SET availability = ?2 WHERE id = ?1",
            params![id.as_uuid().as_bytes(), encode_availability(availability)],
        )?;
        if changed == 0 {
            return Err(CatalogError::InvalidData(format!(
                "library {} does not exist",
                id.as_uuid()
            )));
        }
        Ok(())
    }

    pub fn relink_library(
        &mut self,
        id: LibraryId,
        canonical_root: &Path,
        display_root: &Path,
    ) -> Result<LibraryRootRecord, CatalogError> {
        let key = NativePathKey::from_path(canonical_root);
        let display_path = display_root.to_string_lossy();
        let changed = self.connection.execute(
            "UPDATE library_roots \
             SET canonical_root_key = ?2, display_path = ?3, availability = 'available', last_seen_at = NULL \
             WHERE id = ?1",
            params![id.as_uuid().as_bytes(), key.as_bytes(), display_path],
        )?;
        if changed == 0 {
            return Err(CatalogError::InvalidData(format!(
                "library {} does not exist",
                id.as_uuid()
            )));
        }
        self.find_library(id)?.ok_or_else(|| {
            CatalogError::InvalidData(format!("library {} does not exist", id.as_uuid()))
        })
    }
}

fn decode_library(row: &rusqlite::Row<'_>) -> Result<LibraryRootRecord, rusqlite::Error> {
    let id: Vec<u8> = row.get(0)?;
    let kind: String = row.get(1)?;
    let root_key: Vec<u8> = row.get(3)?;
    let availability: String = row.get(5)?;

    Ok(LibraryRootRecord {
        id: LibraryId::from_uuid(decode_uuid(id, 0)?),
        kind: decode_library_kind(&kind, 1)?,
        display_name: row.get(2)?,
        canonical_root_key: NativePathKey::from_bytes(root_key)
            .map_err(|error| invalid_data(3, error))?,
        display_path: row.get(4)?,
        availability: decode_availability(&availability, 5)?,
        last_seen_at: row.get(6)?,
    })
}

pub(crate) fn decode_uuid(bytes: Vec<u8>, column: usize) -> Result<uuid::Uuid, rusqlite::Error> {
    uuid::Uuid::from_slice(&bytes).map_err(|error| invalid_data(column, error))
}

fn invalid_data(
    column: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Blob, Box::new(error))
}

fn encode_library_kind(kind: LibraryKind) -> &'static str {
    match kind {
        LibraryKind::Configured => "configured",
        LibraryKind::Recent => "recent",
    }
}

fn decode_library_kind(value: &str, column: usize) -> Result<LibraryKind, rusqlite::Error> {
    match value {
        "configured" => Ok(LibraryKind::Configured),
        "recent" => Ok(LibraryKind::Recent),
        other => Err(invalid_data(
            column,
            crate::CatalogError::InvalidData(format!("unknown library kind {other}")),
        )),
    }
}

fn decode_availability(value: &str, column: usize) -> Result<Availability, rusqlite::Error> {
    match value {
        "available" => Ok(Availability::Available),
        "root_offline" => Ok(Availability::RootOffline),
        "missing" => Ok(Availability::Missing),
        "unreadable" => Ok(Availability::Unreadable),
        other => Err(invalid_data(
            column,
            crate::CatalogError::InvalidData(format!("unknown availability {other}")),
        )),
    }
}

fn encode_availability(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "available",
        Availability::RootOffline => "root_offline",
        Availability::Missing => "missing",
        Availability::Unreadable => "unreadable",
    }
}
