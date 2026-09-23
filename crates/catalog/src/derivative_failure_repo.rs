use photo_domain::{AssetId, Availability};
use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalDerivativeFailure {
    pub asset_id: AssetId,
    pub kind: String,
    pub cache_key: String,
    pub availability: Availability,
    pub failure_code: String,
    pub occurred_at: i64,
}

impl Catalog {
    pub fn terminal_derivative_failures(
        &self,
    ) -> Result<Vec<TerminalDerivativeFailure>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT asset_id, kind, cache_key, availability, failure_code, occurred_at \
             FROM derivative_failures",
        )?;
        statement
            .query_map([], decode_terminal_failure)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CatalogError::from)
    }

    pub fn find_terminal_derivative_failure(
        &self,
        asset_id: AssetId,
        kind: &str,
        cache_key: &str,
        availability: Availability,
    ) -> Result<Option<TerminalDerivativeFailure>, CatalogError> {
        self.connection
            .query_row(
                "SELECT asset_id, kind, cache_key, availability, failure_code, occurred_at \
                 FROM derivative_failures \
                 WHERE asset_id = ?1 AND kind = ?2 AND cache_key = ?3 AND availability = ?4",
                params![
                    asset_id.as_uuid().as_bytes(),
                    kind,
                    cache_key,
                    encode_availability(availability),
                ],
                decode_terminal_failure,
            )
            .optional()
            .map_err(CatalogError::from)
    }

    pub fn record_terminal_derivative_failure(
        &mut self,
        failure: &TerminalDerivativeFailure,
    ) -> Result<(), CatalogError> {
        self.connection.execute(
            "INSERT INTO derivative_failures \
                (asset_id, kind, cache_key, availability, failure_code, occurred_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(asset_id, kind) DO UPDATE SET \
                cache_key = excluded.cache_key, \
                availability = excluded.availability, \
                failure_code = excluded.failure_code, \
                occurred_at = excluded.occurred_at",
            params![
                failure.asset_id.as_uuid().as_bytes(),
                failure.kind,
                failure.cache_key,
                encode_availability(failure.availability),
                failure.failure_code,
                failure.occurred_at,
            ],
        )?;
        Ok(())
    }

    pub fn clear_terminal_derivative_failure(
        &mut self,
        asset_id: AssetId,
        kind: &str,
    ) -> Result<(), CatalogError> {
        self.connection.execute(
            "DELETE FROM derivative_failures WHERE asset_id = ?1 AND kind = ?2",
            params![asset_id.as_uuid().as_bytes(), kind],
        )?;
        Ok(())
    }
}

fn decode_terminal_failure(
    row: &rusqlite::Row<'_>,
) -> Result<TerminalDerivativeFailure, rusqlite::Error> {
    let asset_id: Vec<u8> = row.get(0)?;
    let availability: String = row.get(3)?;
    Ok(TerminalDerivativeFailure {
        asset_id: AssetId::from_uuid(crate::library_repo::decode_uuid(asset_id, 0)?),
        kind: row.get(1)?,
        cache_key: row.get(2)?,
        availability: crate::asset_repo::decode_availability(&availability, 3)?,
        failure_code: row.get(4)?,
        occurred_at: row.get(5)?,
    })
}

fn encode_availability(availability: Availability) -> &'static str {
    match availability {
        Availability::Available => "available",
        Availability::RootOffline => "root_offline",
        Availability::Missing => "missing",
        Availability::Unreadable => "unreadable",
    }
}
