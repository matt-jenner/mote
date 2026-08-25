use photo_domain::{AssetId, Availability, FolderGroupId, MediaKind};
use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeStatus {
    Pending,
    Ready,
    Fallback,
}

impl ShapeStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Fallback => "fallback",
        }
    }
    pub(crate) fn decode(value: &str, column: usize) -> Result<Self, rusqlite::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "fallback" => Ok(Self::Fallback),
            _ => Err(rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(crate::CatalogError::InvalidData(format!(
                    "invalid shape status {value}"
                ))),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WallOrder {
    Provisional,
    CapturedAscending,
    CapturedDescending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WallCursorKey {
    Provisional {
        order: u64,
        id: AssetId,
    },
    Captured {
        captured_at_utc: String,
        display_path: String,
        id: AssetId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WallCatalogRecord {
    pub id: AssetId,
    pub display_path: String,
    pub media_kind: MediaKind,
    pub provisional_order: u64,
    pub captured_at_utc: Option<String>,
    pub width: u32,
    pub height: u32,
    pub representative_rgb: Option<u32>,
    pub availability: Availability,
    pub has_warning: bool,
    pub shape_status: ShapeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WallCatalogPage {
    pub items: Vec<WallCatalogRecord>,
    pub next: Option<WallCursorKey>,
}

impl Catalog {
    pub fn wall_records_for_assets(
        &self,
        group: FolderGroupId,
        assets: &[AssetId],
    ) -> Result<Vec<WallCatalogRecord>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT id, display_path, media_kind, provisional_order, captured_at_utc, width, height, representative_rgb, availability, shape_status, \
                    EXISTS(SELECT 1 FROM warnings WHERE warnings.asset_id = assets.id) \
             FROM assets WHERE folder_group_id = ?1 AND id = ?2 \
               AND shape_status IN ('ready','fallback') AND width IS NOT NULL AND height IS NOT NULL",
        )?;
        let mut records = Vec::with_capacity(assets.len());
        for asset in assets {
            let record = statement
                .query_row(
                    params![group.as_uuid().as_bytes(), asset.as_uuid().as_bytes()],
                    decode_wall_record,
                )
                .optional()?;
            if let Some(record) = record {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub fn wall_page(
        &self,
        group: FolderGroupId,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<WallCatalogPage, CatalogError> {
        let mut sql = String::from(
            "SELECT id, display_path, media_kind, provisional_order, captured_at_utc, width, height, representative_rgb, availability, shape_status, \
                    EXISTS(SELECT 1 FROM warnings WHERE warnings.asset_id = assets.id) \
             FROM assets WHERE folder_group_id = ?1 AND shape_status IN ('ready','fallback') AND width IS NOT NULL AND height IS NOT NULL",
        );
        match order {
            WallOrder::Provisional => {
                if cursor.is_some() {
                    sql.push_str(
                        " AND (provisional_order > ?2 OR (provisional_order = ?2 AND id > ?3))",
                    );
                }
            }
            WallOrder::CapturedAscending => {
                if cursor.is_some() {
                    sql.push_str(" AND (captured_at_utc, display_path, id) > (?2, ?3, ?4)");
                }
            }
            WallOrder::CapturedDescending => {
                if cursor.is_some() {
                    sql.push_str(" AND (captured_at_utc < ?2 OR (captured_at_utc = ?2 AND (display_path > ?3 OR (display_path = ?3 AND id > ?4))))");
                }
            }
        }
        sql.push_str(match order { WallOrder::Provisional => " ORDER BY provisional_order, id", WallOrder::CapturedAscending => " AND captured_at_utc IS NOT NULL ORDER BY captured_at_utc ASC, display_path ASC, id ASC", WallOrder::CapturedDescending => " AND captured_at_utc IS NOT NULL ORDER BY captured_at_utc DESC, display_path ASC, id ASC" });
        sql.push_str(match (order, cursor.is_some()) {
            (WallOrder::Provisional, false) => " LIMIT ?2",
            (WallOrder::Provisional, true) => " LIMIT ?4",
            (_, false) => " LIMIT ?2",
            (_, true) => " LIMIT ?5",
        });
        let mut stmt = self.connection.prepare(&sql)?;
        let group_uuid = group.as_uuid();
        let group_bytes = group_uuid.as_bytes();
        let mut rows = match cursor {
            None => stmt.query(params![group_bytes, limit as i64])?,
            Some(WallCursorKey::Provisional {
                order: position,
                id,
            }) => stmt.query(params![
                group_bytes,
                position as i64,
                id.as_uuid().as_bytes(),
                limit as i64
            ])?,
            Some(WallCursorKey::Captured {
                captured_at_utc,
                display_path,
                id,
            }) => stmt.query(params![
                group_bytes,
                captured_at_utc,
                display_path,
                id.as_uuid().as_bytes(),
                limit as i64
            ])?,
        };
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(decode_wall_record(row)?);
        }
        let next = items.last().map(|item| match order {
            WallOrder::Provisional => WallCursorKey::Provisional {
                order: item.provisional_order,
                id: item.id,
            },
            _ => WallCursorKey::Captured {
                captured_at_utc: item.captured_at_utc.clone().unwrap_or_default(),
                display_path: item.display_path.clone(),
                id: item.id,
            },
        });
        Ok(WallCatalogPage { items, next })
    }
}

fn decode_wall_record(row: &rusqlite::Row<'_>) -> Result<WallCatalogRecord, rusqlite::Error> {
    let id: Vec<u8> = row.get(0)?;
    Ok(WallCatalogRecord {
        id: AssetId::from_uuid(crate::library_repo::decode_uuid(id, 0)?),
        display_path: row.get(1)?,
        media_kind: crate::asset_repo::decode_media_kind(row.get::<_, String>(2)?.as_str(), 2)?,
        provisional_order: row.get::<_, i64>(3)? as u64,
        captured_at_utc: row.get(4)?,
        width: row.get::<_, i64>(5)? as u32,
        height: row.get::<_, i64>(6)? as u32,
        representative_rgb: row.get::<_, Option<i64>>(7)?.map(|value| value as u32),
        availability: crate::asset_repo::decode_availability(row.get::<_, String>(8)?.as_str(), 8)?,
        shape_status: ShapeStatus::decode(row.get::<_, String>(9)?.as_str(), 9)?,
        has_warning: row.get(10)?,
    })
}
