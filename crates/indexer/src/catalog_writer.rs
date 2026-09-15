use std::time::Duration;

use photo_catalog::{
    AssetColourUpdate, AssetMetadataUpdate, AssetShapeUpdate, Catalog, CatalogError,
    CatalogIndexRecord, CatalogKeyword, CatalogProvenance, CatalogWarningRecord, ShapeStatus,
};
use photo_domain::LibraryId;
use tokio::sync::mpsc;

use crate::IndexEvent;

const MAX_BATCH_EVENTS: usize = 200;
const MAX_BATCH_WAIT: Duration = Duration::from_millis(50);

pub struct CatalogWriter<'a> {
    catalog: &'a mut Catalog,
    library_id: LibraryId,
    generation: u64,
}

impl<'a> CatalogWriter<'a> {
    pub fn new(catalog: &'a mut Catalog, library_id: LibraryId, generation: u64) -> Self {
        Self {
            catalog,
            library_id,
            generation,
        }
    }

    pub fn apply_batch(&mut self, events: &[IndexEvent]) -> Result<(), CatalogError> {
        let mut records = Vec::new();
        for event in events {
            if let IndexEvent::ShapeFallback {
                asset_id,
                code: "source_missing" | "source_unreadable" | "source_check_failed",
                ..
            } = event
                && self.catalog.find_asset(*asset_id)?.is_some_and(|asset| {
                    asset.library_id == self.library_id
                        && asset.media_kind == photo_domain::MediaKind::Heif
                })
            {
                continue;
            }
            if let Some(record) = to_catalog_record(event, self.library_id) {
                records.push(record);
            }
        }
        self.catalog
            .apply_index_batch_for_generation(self.library_id, self.generation, &records)
    }

    pub async fn run(
        &mut self,
        mut events: mpsc::Receiver<IndexEvent>,
    ) -> Result<(), CatalogError> {
        while let Some(first) = events.recv().await {
            let mut batch = vec![first];
            let deadline = tokio::time::sleep(MAX_BATCH_WAIT);
            tokio::pin!(deadline);
            while batch.len() < MAX_BATCH_EVENTS {
                tokio::select! {
                    () = &mut deadline => break,
                    next = events.recv() => match next {
                        Some(event) => batch.push(event),
                        None => break,
                    }
                }
            }
            self.apply_batch(&batch)?;
        }
        Ok(())
    }
}

fn to_catalog_record(event: &IndexEvent, library_id: LibraryId) -> Option<CatalogIndexRecord> {
    match event {
        IndexEvent::Discovered { asset } => Some(CatalogIndexRecord::Discovered(asset.clone())),
        IndexEvent::ShapeReady {
            asset_id,
            width,
            height,
            orientation,
        } => Some(CatalogIndexRecord::Shaped(AssetShapeUpdate {
            asset_id: *asset_id,
            width: *width,
            height: *height,
            orientation: Some(*orientation),
            representative_rgb: None,
            shape_status: ShapeStatus::Ready,
        })),
        IndexEvent::ShapeFallback {
            asset_id,
            width,
            height,
            ..
        } => Some(CatalogIndexRecord::Shaped(AssetShapeUpdate {
            asset_id: *asset_id,
            width: *width,
            height: *height,
            orientation: None,
            representative_rgb: None,
            shape_status: ShapeStatus::Fallback,
        })),
        IndexEvent::ColourReady {
            asset_id,
            representative_rgb,
        } => Some(CatalogIndexRecord::Coloured(AssetColourUpdate {
            asset_id: *asset_id,
            representative_rgb: (u32::from(representative_rgb.red) << 16)
                | (u32::from(representative_rgb.green) << 8)
                | u32::from(representative_rgb.blue),
        })),
        IndexEvent::MetadataReady { asset_id, metadata } => {
            Some(CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                asset_id: *asset_id,
                captured_at_utc: metadata
                    .captured_at
                    .map(|value| value.with_timezone(&chrono::Utc).to_rfc3339()),
                rating: metadata.rating,
                keywords: metadata
                    .keywords
                    .iter()
                    .map(|keyword| CatalogKeyword {
                        normalized: keyword.normalized.clone(),
                        display_value: keyword.display_value.clone(),
                        hierarchy: keyword.hierarchy.clone(),
                    })
                    .collect(),
                provenance: metadata
                    .provenance
                    .iter()
                    .map(|record| CatalogProvenance {
                        field_name: record.field.clone(),
                        source_kind: format!("{:?}", record.source),
                        raw_value: record.raw_value.clone(),
                        chosen: record.chosen,
                    })
                    .collect(),
            }))
        }
        IndexEvent::Warning {
            asset_id,
            code,
            message,
        } => Some(CatalogIndexRecord::Warning(CatalogWarningRecord {
            library_id,
            asset_id: *asset_id,
            code: (*code).to_owned(),
            message: message.clone(),
        })),
        IndexEvent::Completed(_) => None,
        IndexEvent::Progress(_) => None,
    }
}
