use photo_domain::{FolderPolicy, LibraryId};
use rusqlite::params;

use crate::{Catalog, CatalogError};

impl Catalog {
    pub fn save_policies(
        &mut self,
        library: LibraryId,
        policies: &[FolderPolicy],
    ) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM library_policies WHERE library_id = ?1",
            [library.as_uuid().as_bytes()],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO library_policies (library_id, position, policy_json) \
                 VALUES (?1, ?2, ?3)",
            )?;
            for (position, policy) in policies.iter().enumerate() {
                statement.execute(params![
                    library.as_uuid().as_bytes(),
                    i64::try_from(position).map_err(|_| CatalogError::ValueOutOfRange)?,
                    serde_json::to_string(policy)?,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn load_policies(&self, library: LibraryId) -> Result<Vec<FolderPolicy>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT policy_json FROM library_policies \
             WHERE library_id = ?1 ORDER BY position",
        )?;
        let values = statement
            .query_map([library.as_uuid().as_bytes()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|value| serde_json::from_str(&value).map_err(CatalogError::from))
            .collect()
    }
}
