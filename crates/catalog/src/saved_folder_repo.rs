use photo_domain::FolderGroupId;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{Catalog, CatalogError, library_repo::decode_uuid};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedFolderRecord {
    pub id: Uuid,
    pub folder_group_id: FolderGroupId,
    pub custom_label: Option<String>,
}

impl Catalog {
    pub fn list_saved_folders(&self) -> Result<Vec<SavedFolderRecord>, CatalogError> {
        self.connection
            .prepare("SELECT id, folder_group_id, custom_label FROM saved_folders ORDER BY id")?
            .query_map([], decode_saved_folder)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn find_saved_folder(&self, id: Uuid) -> Result<Option<SavedFolderRecord>, CatalogError> {
        self.connection
            .query_row(
                "SELECT id, folder_group_id, custom_label FROM saved_folders WHERE id = ?1",
                [id.as_bytes()],
                decode_saved_folder,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_folder(&mut self, group: FolderGroupId) -> Result<SavedFolderRecord, CatalogError> {
        let transaction = self.connection.transaction()?;
        let result = save_on(&transaction, group)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn save_and_activate_folder(
        &mut self,
        group: FolderGroupId,
    ) -> Result<SavedFolderRecord, CatalogError> {
        let transaction = self.connection.transaction()?;
        let result = save_on(&transaction, group)?;
        transaction.execute(
            "DELETE FROM active_source_selection WHERE singleton = 1",
            [],
        )?;
        transaction.execute(
            "INSERT INTO active_source_selection (singleton, library_id, relative_folder_key)
            SELECT 1, library_id, relative_path_key FROM folder_groups WHERE id = ?1",
            [group.as_uuid().as_bytes()],
        )?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn rename_saved_folder(
        &mut self,
        id: Uuid,
        label: Option<&str>,
    ) -> Result<(), CatalogError> {
        let label = label.map(str::trim).filter(|value| !value.is_empty());
        let changed = self.connection.execute(
            "UPDATE saved_folders SET custom_label = ?2 WHERE id = ?1",
            params![id.as_bytes(), label],
        )?;
        if changed == 0 {
            return Err(CatalogError::InvalidData(
                "saved folder does not exist".into(),
            ));
        }
        Ok(())
    }

    pub fn remove_saved_folder(&mut self, id: Uuid) -> Result<bool, CatalogError> {
        let transaction = self.connection.transaction()?;
        let active = transaction.execute("DELETE FROM active_source_selection WHERE singleton = 1 AND EXISTS (
            SELECT 1 FROM saved_folders saved JOIN folder_groups folder ON folder.id = saved.folder_group_id
            WHERE saved.id = ?1 AND folder.library_id = active_source_selection.library_id
            AND folder.relative_path_key = active_source_selection.relative_folder_key)", [id.as_bytes()])? > 0;
        transaction.execute("DELETE FROM saved_folders WHERE id = ?1", [id.as_bytes()])?;
        transaction.commit()?;
        Ok(active)
    }

    pub fn has_opened_folder(&self) -> Result<bool, CatalogError> {
        self.connection
            .query_row(
                "SELECT has_opened_folder FROM saved_folder_preferences WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
}

fn save_on(
    connection: &Connection,
    group: FolderGroupId,
) -> Result<SavedFolderRecord, CatalogError> {
    connection.execute(
        "INSERT INTO saved_folders (id, folder_group_id) VALUES (?1, ?2)
        ON CONFLICT(folder_group_id) DO NOTHING",
        params![Uuid::new_v4().as_bytes(), group.as_uuid().as_bytes()],
    )?;
    connection.execute(
        "UPDATE saved_folder_preferences SET has_opened_folder = 1 WHERE singleton = 1",
        [],
    )?;
    connection.query_row("SELECT id, folder_group_id, custom_label FROM saved_folders WHERE folder_group_id = ?1",
        [group.as_uuid().as_bytes()], decode_saved_folder).map_err(Into::into)
}

fn decode_saved_folder(row: &rusqlite::Row<'_>) -> Result<SavedFolderRecord, rusqlite::Error> {
    Ok(SavedFolderRecord {
        id: decode_uuid(row.get(0)?, 0)?,
        folder_group_id: FolderGroupId::from_uuid(decode_uuid(row.get(1)?, 1)?),
        custom_label: row.get(2)?,
    })
}

pub(crate) fn migrate_saved_folders(connection: &Connection) -> Result<(), CatalogError> {
    let active: Option<(Vec<u8>, Vec<u8>, String)> = connection.query_row(
        "SELECT active.library_id, active.relative_folder_key, library.display_name
         FROM active_source_selection active JOIN library_roots library ON library.id = active.library_id WHERE singleton = 1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((library, relative, name)) = active else {
        return Ok(());
    };
    let key = photo_domain::RelativePathKey::from_bytes(relative.clone())
        .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
    let path = key
        .to_path_buf()
        .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
    let display = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(name);
    connection.execute(
        "INSERT INTO folder_groups (id, library_id, relative_path_key, display_path)
        VALUES (?1, ?2, ?3, ?4) ON CONFLICT(library_id, relative_path_key) DO NOTHING",
        params![Uuid::new_v4().as_bytes(), library, relative, display],
    )?;
    let group: Vec<u8> = connection.query_row(
        "SELECT id FROM folder_groups WHERE library_id = ?1 AND relative_path_key = ?2",
        params![library, relative],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO saved_folders (id, folder_group_id) VALUES (?1, ?2)
        ON CONFLICT(folder_group_id) DO NOTHING",
        params![Uuid::new_v4().as_bytes(), group],
    )?;
    connection.execute(
        "UPDATE saved_folder_preferences SET has_opened_folder = 1 WHERE singleton = 1",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewFolderGroup, NewLibrary, StoredSourceSelection, migrate};
    use photo_domain::RelativePathKey;
    use std::path::Path;

    #[test]
    fn upgrade_seeds_only_active_folder_once_with_or_without_an_existing_group() {
        for active in [false, true] {
            for existing_group in [false, true] {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate::apply_migrations(&mut connection, &migrate::MIGRATIONS[..11]).unwrap();
                let mut catalog = Catalog { connection };
                let library = catalog
                    .add_library(&NewLibrary::recent("Photos", Path::new("/Photos")))
                    .unwrap();
                let relative = RelativePathKey::from_relative_path(Path::new("Family")).unwrap();
                if existing_group {
                    catalog
                        .upsert_folder_group(&NewFolderGroup {
                            id: FolderGroupId::new(),
                            library_id: library.id,
                            relative_path: relative.clone(),
                            display_path: "Family".into(),
                            last_viewed_at: None,
                        })
                        .unwrap();
                }
                if active {
                    catalog
                        .set_active_selection(Some(&StoredSourceSelection {
                            library_id: library.id,
                            relative_folder: relative,
                        }))
                        .unwrap();
                }
                migrate::apply_migrations(&mut catalog.connection, migrate::MIGRATIONS).unwrap();
                let entries = catalog.list_saved_folders().unwrap();
                assert_eq!(entries.len(), usize::from(active));
                assert_eq!(catalog.has_opened_folder().unwrap(), active);
                if let Some(entry) = entries.first() {
                    assert!(catalog.remove_saved_folder(entry.id).unwrap());
                    migrate::apply_migrations(&mut catalog.connection, migrate::MIGRATIONS)
                        .unwrap();
                    assert!(catalog.list_saved_folders().unwrap().is_empty());
                }
            }
        }
    }
}
