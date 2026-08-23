use crate::{Catalog, CatalogError};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CatalogHealthSnapshot {
    pub available_sources: u64,
    pub unavailable_sources: u64,
    pub active_warnings: u64,
}

impl Catalog {
    pub fn health_snapshot(&self) -> Result<CatalogHealthSnapshot, CatalogError> {
        let (available, unavailable): (i64, i64) = self.connection.query_row(
            "SELECT \
                COALESCE(SUM(CASE WHEN availability = 'available' THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN availability <> 'available' THEN 1 ELSE 0 END), 0) \
             FROM library_roots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let active_warnings: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM warnings", [], |row| row.get(0))?;
        Ok(CatalogHealthSnapshot {
            available_sources: checked_count(available)?,
            unavailable_sources: checked_count(unavailable)?,
            active_warnings: checked_count(active_warnings)?,
        })
    }

    pub fn unavailable_asset_count(
        &self,
        library: photo_domain::LibraryId,
    ) -> Result<u64, CatalogError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM assets WHERE library_id = ?1 AND availability <> 'available'",
            [library.as_uuid().as_bytes()],
            |row| row.get(0),
        )?;
        checked_count(count)
    }
}

fn checked_count(value: i64) -> Result<u64, CatalogError> {
    u64::try_from(value).map_err(|_| CatalogError::ValueOutOfRange)
}
