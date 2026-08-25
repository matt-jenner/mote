use photo_domain::{AssetId, FolderGroupId};
use rusqlite::params;

use crate::{Catalog, CatalogError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeStatus {
    Ready,
    Fallback,
}

impl ShapeStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Fallback => "fallback",
        }
    }
    pub(crate) fn decode(value: &str, column: usize) -> Result<Self, rusqlite::Error> {
        match value {
            "ready" => Ok(Self::Ready),
            "fallback" | "pending" => Ok(Self::Fallback),
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
    pub provisional_order: u64,
    pub captured_at_utc: Option<String>,
    pub width: u32,
    pub height: u32,
    pub shape_status: ShapeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WallCatalogPage {
    pub items: Vec<WallCatalogRecord>,
    pub next: Option<WallCursorKey>,
}

impl Catalog {
    pub fn wall_page(
        &self,
        group: FolderGroupId,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<WallCatalogPage, CatalogError> {
        let mut sql = String::from(
            "SELECT id, display_path, provisional_order, captured_at_utc, width, height, shape_status FROM assets WHERE folder_group_id = ?1 AND shape_status IN ('ready','fallback') AND width IS NOT NULL AND height IS NOT NULL",
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
        sql.push_str(if cursor.is_some() {
            " LIMIT ?5"
        } else {
            " LIMIT ?2"
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
            let id: Vec<u8> = row.get(0)?;
            items.push(WallCatalogRecord {
                id: AssetId::from_uuid(crate::library_repo::decode_uuid(id, 0)?),
                display_path: row.get(1)?,
                provisional_order: row.get::<_, i64>(2)? as u64,
                captured_at_utc: row.get(3)?,
                width: row.get::<_, i64>(4)? as u32,
                height: row.get::<_, i64>(5)? as u32,
                shape_status: ShapeStatus::decode(row.get::<_, String>(6)?.as_str(), 6)?,
            });
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
