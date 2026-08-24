use photo_domain::{Appearance, LibraryId, RelativePathKey};
use rusqlite::params;

use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSourceSelection {
    pub library_id: LibraryId,
    pub relative_folder: RelativePathKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppStateRecord {
    pub appearance: Appearance,
    pub active_selection: Option<StoredSourceSelection>,
}

impl Catalog {
    pub fn load_app_state(&self) -> Result<AppStateRecord, CatalogError> {
        let (appearance, library_id, relative_folder): (String, Option<Vec<u8>>, Option<Vec<u8>>) =
            self.connection.query_row(
                "SELECT app_state.appearance, active.library_id, active.relative_folder_key \
                 FROM app_state \
                 LEFT JOIN active_source_selection AS active USING (singleton) \
                 WHERE app_state.singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;

        let appearance = decode_appearance(&appearance)?;
        let active_selection = match (library_id, relative_folder) {
            (None, None) => None,
            (Some(library_id), Some(relative_folder)) => Some(StoredSourceSelection {
                library_id: LibraryId::from_uuid(decode_uuid(library_id, 1)?),
                relative_folder: RelativePathKey::from_bytes(relative_folder).map_err(|error| {
                    CatalogError::InvalidData(format!("invalid active folder key: {error}"))
                })?,
            }),
            _ => {
                return Err(CatalogError::InvalidData(
                    "active source selection is incomplete".into(),
                ));
            }
        };

        Ok(AppStateRecord {
            appearance,
            active_selection,
        })
    }

    pub fn set_appearance(&mut self, appearance: Appearance) -> Result<(), CatalogError> {
        let changed = self.connection.execute(
            "UPDATE app_state SET appearance = ?1 WHERE singleton = 1",
            [encode_appearance(appearance)],
        )?;
        require_singleton(changed)
    }

    pub fn set_active_selection(
        &mut self,
        selection: Option<&StoredSourceSelection>,
    ) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM active_source_selection WHERE singleton = 1",
            [],
        )?;
        if let Some(selection) = selection {
            transaction.execute(
                "INSERT INTO active_source_selection \
                 (singleton, library_id, relative_folder_key) VALUES (1, ?1, ?2)",
                params![
                    selection.library_id.as_uuid().as_bytes(),
                    selection.relative_folder.as_bytes(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

fn encode_appearance(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

fn decode_appearance(value: &str) -> Result<Appearance, CatalogError> {
    match value {
        "system" => Ok(Appearance::System),
        "light" => Ok(Appearance::Light),
        "dark" => Ok(Appearance::Dark),
        other => Err(CatalogError::InvalidData(format!(
            "unknown appearance {other}"
        ))),
    }
}

fn require_singleton(changed: usize) -> Result<(), CatalogError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(CatalogError::InvalidData("app state row is missing".into()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use photo_domain::RelativePathKey;
    use rusqlite::params;

    use super::StoredSourceSelection;
    use crate::{Catalog, NewLibrary};

    #[test]
    fn deleting_the_active_library_clears_the_selection() {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::recent("Family", Path::new("/Volumes/Family")))
            .unwrap();
        catalog
            .set_active_selection(Some(&StoredSourceSelection {
                library_id: library.id,
                relative_folder: RelativePathKey::from_relative_path(Path::new("")).unwrap(),
            }))
            .unwrap();

        catalog
            .connection
            .execute(
                "DELETE FROM library_roots WHERE id = ?1",
                params![library.id.as_uuid().as_bytes()],
            )
            .unwrap();

        assert_eq!(catalog.load_app_state().unwrap().active_selection, None);
    }
}
