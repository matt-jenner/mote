use photo_domain::{AssetId, Availability, FolderGroupId, GalleryScope, MediaKind};
use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError};

#[cfg(feature = "heic")]
const WALL_MEDIA_KINDS: &str = "('jpeg','png','tiff','heif','webp')";
#[cfg(not(feature = "heic"))]
const WALL_MEDIA_KINDS: &str = "('jpeg','png','tiff','webp')";

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
    pub warning_code: Option<String>,
    pub shape_status: ShapeStatus,
    pub rating: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WallCatalogPage {
    pub items: Vec<WallCatalogRecord>,
    pub next: Option<WallCursorKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoAssetIdPage {
    pub items: Vec<AssetId>,
    pub next: Option<WallCursorKey>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WallPreviewCounts {
    pub wall_ready: u64,
    pub screen_ready: u64,
}

impl Catalog {
    pub fn wall_photo_count_scoped(
        &self,
        group: FolderGroupId,
        scope: GalleryScope,
    ) -> Result<u64, CatalogError> {
        let mut sql = format!(
            "SELECT COUNT(*) \
             FROM assets JOIN folder_group_assets fga ON fga.asset_id = assets.id \
             WHERE fga.folder_group_id = ?1 \
               AND media_kind IN {WALL_MEDIA_KINDS} \
               AND NOT EXISTS (SELECT 1 FROM derivative_failures \
                               WHERE derivative_failures.asset_id = assets.id \
                                 AND derivative_failures.kind = 'wall_thumbnail' \
                                 AND derivative_failures.availability = 'available' \
                                 AND derivative_failures.availability = assets.availability)",
        );
        if scope == GalleryScope::CurrentFolder {
            sql.push_str(
                " AND relative_parent_key = (SELECT relative_path_key FROM folder_groups WHERE id = ?1)",
            );
        }
        let count: i64 = self
            .connection
            .query_row(&sql, [group.as_uuid().as_bytes()], |row| row.get(0))?;
        u64::try_from(count).map_err(|_| CatalogError::ValueOutOfRange)
    }

    pub fn wall_preview_counts_scoped(
        &self,
        group: FolderGroupId,
        scope: GalleryScope,
    ) -> Result<WallPreviewCounts, CatalogError> {
        let mut sql = format!(
            "SELECT COALESCE(SUM(EXISTS(SELECT 1 FROM derivatives derivative \
                                        WHERE derivative.asset_id = assets.id \
                                          AND derivative.kind = 'wall_thumbnail')), 0), \
                    COALESCE(SUM(EXISTS(SELECT 1 FROM derivatives derivative \
                                        WHERE derivative.asset_id = assets.id \
                                          AND derivative.kind = 'screen_preview')), 0) \
             FROM assets JOIN folder_group_assets fga ON fga.asset_id = assets.id \
             WHERE fga.folder_group_id = ?1 \
               AND media_kind IN {WALL_MEDIA_KINDS}"
        );
        if scope == GalleryScope::CurrentFolder {
            sql.push_str(
                " AND relative_parent_key = (SELECT relative_path_key FROM folder_groups WHERE id = ?1)",
            );
        }
        let (wall_ready, screen_ready): (i64, i64) =
            self.connection
                .query_row(&sql, [group.as_uuid().as_bytes()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
        Ok(WallPreviewCounts {
            wall_ready: u64::try_from(wall_ready).map_err(|_| CatalogError::ValueOutOfRange)?,
            screen_ready: u64::try_from(screen_ready).map_err(|_| CatalogError::ValueOutOfRange)?,
        })
    }

    pub fn wall_records_for_assets(
        &self,
        group: FolderGroupId,
        assets: &[AssetId],
    ) -> Result<Vec<WallCatalogRecord>, CatalogError> {
        self.wall_records_for_assets_scoped(group, GalleryScope::IncludeSubfolders, assets)
    }

    pub fn wall_records_for_assets_scoped(
        &self,
        group: FolderGroupId,
        scope: GalleryScope,
        assets: &[AssetId],
    ) -> Result<Vec<WallCatalogRecord>, CatalogError> {
        let mut sql = format!(
            "SELECT id, display_path, media_kind, provisional_order, captured_at_utc, width, height, representative_rgb, availability, shape_status, rating, \
                    EXISTS(SELECT 1 FROM warnings WHERE warnings.asset_id = assets.id), \
                    (SELECT code FROM warnings WHERE warnings.asset_id = assets.id ORDER BY CASE code WHEN 'derivative_generation_failed' THEN 0 ELSE 1 END, occurred_at DESC, id DESC LIMIT 1) \
             FROM assets JOIN folder_group_assets fga ON fga.asset_id = assets.id \
             WHERE fga.folder_group_id = ?1 AND assets.id = ?2 \
               AND media_kind IN {WALL_MEDIA_KINDS} \
               AND NOT EXISTS (SELECT 1 FROM derivative_failures \
                               WHERE derivative_failures.asset_id = assets.id \
                                 AND derivative_failures.kind = 'wall_thumbnail' \
                                 AND derivative_failures.availability = 'available' \
                                 AND derivative_failures.availability = assets.availability) \
               AND shape_status IN ('ready','fallback') AND width IS NOT NULL AND height IS NOT NULL",
        );
        if scope == GalleryScope::CurrentFolder {
            sql.push_str(
                " AND relative_parent_key = (SELECT relative_path_key FROM folder_groups WHERE id = ?1)",
            );
        }
        let mut statement = self.connection.prepare(&sql)?;
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
        self.wall_page_scoped(group, GalleryScope::IncludeSubfolders, order, cursor, limit)
    }

    pub fn wall_page_scoped(
        &self,
        group: FolderGroupId,
        scope: GalleryScope,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<WallCatalogPage, CatalogError> {
        if cursor.as_ref().is_some_and(|cursor| {
            !matches!(
                (order, cursor),
                (WallOrder::Provisional, WallCursorKey::Provisional { .. })
                    | (
                        WallOrder::CapturedAscending | WallOrder::CapturedDescending,
                        WallCursorKey::Captured { .. }
                    )
            )
        }) {
            return Err(CatalogError::WallCursorOrderMismatch);
        }
        let mut sql = format!(
            "SELECT id, display_path, media_kind, provisional_order, captured_at_utc, width, height, representative_rgb, availability, shape_status, rating, \
                    EXISTS(SELECT 1 FROM warnings WHERE warnings.asset_id = assets.id), \
                    (SELECT code FROM warnings WHERE warnings.asset_id = assets.id ORDER BY CASE code WHEN 'derivative_generation_failed' THEN 0 ELSE 1 END, occurred_at DESC, id DESC LIMIT 1) \
             FROM assets JOIN folder_group_assets fga ON fga.asset_id = assets.id \
             WHERE fga.folder_group_id = ?1 \
               AND media_kind IN {WALL_MEDIA_KINDS} \
               AND NOT EXISTS (SELECT 1 FROM derivative_failures \
                               WHERE derivative_failures.asset_id = assets.id \
                                 AND derivative_failures.kind = 'wall_thumbnail' \
                                 AND derivative_failures.availability = 'available' \
                                 AND derivative_failures.availability = assets.availability) \
               AND shape_status IN ('ready','fallback') AND width IS NOT NULL AND height IS NOT NULL",
        );
        if scope == GalleryScope::CurrentFolder {
            sql.push_str(
                " AND relative_parent_key = (SELECT relative_path_key FROM folder_groups WHERE id = ?1)",
            );
        }
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

    pub fn photo_asset_ids_page(
        &self,
        group: FolderGroupId,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<PhotoAssetIdPage, CatalogError> {
        self.photo_asset_ids_page_scoped(
            group,
            GalleryScope::IncludeSubfolders,
            order,
            cursor,
            limit,
        )
    }

    pub fn photo_asset_ids_page_scoped(
        &self,
        group: FolderGroupId,
        scope: GalleryScope,
        order: WallOrder,
        cursor: Option<WallCursorKey>,
        limit: u32,
    ) -> Result<PhotoAssetIdPage, CatalogError> {
        let page = self.wall_page_scoped(group, scope, order, cursor, limit)?;
        Ok(PhotoAssetIdPage {
            items: page.items.into_iter().map(|row| row.id).collect(),
            next: page.next,
        })
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
        rating: row
            .get::<_, Option<i64>>(10)?
            .map(|value| {
                u8::try_from(value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })
            })
            .transpose()?,
        has_warning: row.get(11)?,
        warning_code: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::Catalog;

    fn plan_details(catalog: &Catalog, sql: &str) -> Vec<String> {
        let mut statement = catalog.connection.prepare(sql).unwrap();
        statement
            .query_map(params![vec![0_u8; 16], 10_i64], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn provisional_wall_query_uses_group_order_index() {
        let catalog = Catalog::open_in_memory().unwrap();
        let details = plan_details(
            &catalog,
            "EXPLAIN QUERY PLAN
             SELECT id FROM assets
             WHERE folder_group_id = ?1
               AND media_kind <> 'video'
               AND shape_status IN ('ready', 'fallback')
               AND width IS NOT NULL
               AND height IS NOT NULL
             ORDER BY provisional_order, id
             LIMIT ?2",
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("assets_group_provisional_photo")),
            "expected provisional wall index in query plan: {details:?}"
        );
    }

    #[test]
    fn captured_wall_query_uses_group_capture_index() {
        let catalog = Catalog::open_in_memory().unwrap();
        let details = plan_details(
            &catalog,
            "EXPLAIN QUERY PLAN
             SELECT id FROM assets
             WHERE folder_group_id = ?1
               AND media_kind <> 'video'
               AND shape_status IN ('ready', 'fallback')
               AND width IS NOT NULL
               AND height IS NOT NULL
               AND captured_at_utc IS NOT NULL
             ORDER BY captured_at_utc ASC, display_path ASC, id ASC
             LIMIT ?2",
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("assets_group_capture_photo")),
            "expected captured wall index in query plan: {details:?}"
        );
    }

    #[test]
    fn descending_captured_wall_query_uses_its_mixed_direction_index() {
        let catalog = Catalog::open_in_memory().unwrap();
        let details = plan_details(
            &catalog,
            "EXPLAIN QUERY PLAN
             SELECT id FROM assets
             WHERE folder_group_id = ?1
               AND media_kind <> 'video'
               AND shape_status IN ('ready', 'fallback')
               AND width IS NOT NULL
               AND height IS NOT NULL
               AND captured_at_utc IS NOT NULL
             ORDER BY captured_at_utc DESC, display_path ASC, id ASC
             LIMIT ?2",
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("assets_group_capture_desc_photo")),
            "expected descending capture wall index in query plan: {details:?}"
        );
        assert!(
            details.iter().all(|detail| !detail.contains("TEMP B-TREE")),
            "descending wall order should not need a temporary sort: {details:?}"
        );
    }

    #[test]
    fn cursor_bearing_wall_queries_keep_using_group_keyset_indexes() {
        let catalog = Catalog::open_in_memory().unwrap();
        let group = vec![0_u8; 16];
        let asset = vec![1_u8; 16];
        let mut provisional = catalog
            .connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM assets
                 WHERE folder_group_id = ?1
                   AND media_kind <> 'video'
                   AND shape_status IN ('ready', 'fallback')
                   AND width IS NOT NULL
                   AND height IS NOT NULL
                   AND (provisional_order > ?2 OR (provisional_order = ?2 AND id > ?3))
                 ORDER BY provisional_order, id
                 LIMIT ?4",
            )
            .unwrap();
        let provisional_details = provisional
            .query_map(params![group, 42_i64, asset, 10_i64], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            provisional_details
                .iter()
                .any(|detail| detail.contains("assets_group_provisional_photo")),
            "expected provisional keyset index in query plan: {provisional_details:?}"
        );

        let group = vec![0_u8; 16];
        let asset = vec![1_u8; 16];
        let mut captured = catalog
            .connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM assets
                 WHERE folder_group_id = ?1
                   AND media_kind <> 'video'
                   AND shape_status IN ('ready', 'fallback')
                   AND width IS NOT NULL
                   AND height IS NOT NULL
                   AND (captured_at_utc, display_path, id) > (?2, ?3, ?4)
                   AND captured_at_utc IS NOT NULL
                 ORDER BY captured_at_utc ASC, display_path ASC, id ASC
                 LIMIT ?5",
            )
            .unwrap();
        let captured_details = captured
            .query_map(
                params![group, "2026-01-01T00:00:00Z", "photo.jpg", asset, 10_i64],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            captured_details
                .iter()
                .any(|detail| detail.contains("assets_group_capture_photo")),
            "expected capture keyset index in query plan: {captured_details:?}"
        );
        assert!(
            captured_details
                .iter()
                .all(|detail| !detail.contains("TEMP B-TREE")),
            "capture keyset query should not need a temporary sort: {captured_details:?}"
        );

        let mut descending = catalog
            .connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM assets
                 WHERE folder_group_id = ?1
                   AND media_kind <> 'video'
                   AND shape_status IN ('ready', 'fallback')
                   AND width IS NOT NULL
                   AND height IS NOT NULL
                   AND (captured_at_utc < ?2 OR (captured_at_utc = ?2 AND (display_path > ?3 OR (display_path = ?3 AND id > ?4))))
                   AND captured_at_utc IS NOT NULL
                 ORDER BY captured_at_utc DESC, display_path ASC, id ASC
                 LIMIT ?5",
            )
            .unwrap();
        let descending_details = descending
            .query_map(
                params![
                    vec![0_u8; 16],
                    "2026-01-01T00:00:00Z",
                    "photo.jpg",
                    vec![1_u8; 16],
                    10_i64
                ],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            descending_details
                .iter()
                .any(|detail| detail.contains("assets_group_capture_desc_photo")),
            "expected descending capture keyset index in query plan: {descending_details:?}"
        );
        assert!(
            descending_details
                .iter()
                .all(|detail| !detail.contains("TEMP B-TREE")),
            "descending capture keyset query should not need a temporary sort: {descending_details:?}"
        );
    }
}
