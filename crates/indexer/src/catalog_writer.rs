use std::time::Duration;

use photo_catalog::{
    AssetMetadataUpdate, AssetShapeUpdate, Catalog, CatalogError, CatalogIndexRecord,
    CatalogKeyword, CatalogProvenance, CatalogWarningRecord,
};
use photo_domain::LibraryId;
use tokio::sync::mpsc;

use crate::IndexEvent;

const MAX_BATCH_EVENTS: usize = 500;
const MAX_BATCH_WAIT: Duration = Duration::from_millis(50);

pub struct CatalogWriter<'a> {
    catalog: &'a mut Catalog,
    library_id: LibraryId,
}

impl<'a> CatalogWriter<'a> {
    pub fn new(catalog: &'a mut Catalog, library_id: LibraryId) -> Self {
        Self {
            catalog,
            library_id,
        }
    }

    pub fn apply_batch(&mut self, events: &[IndexEvent]) -> Result<(), CatalogError> {
        let records = events
            .iter()
            .filter_map(|event| to_catalog_record(event, self.library_id))
            .collect::<Vec<_>>();
        for batch in records.chunks(MAX_BATCH_EVENTS) {
            self.catalog.apply_index_batch(batch)?;
        }
        Ok(())
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
        IndexEvent::Shaped {
            asset_id,
            width,
            height,
            orientation,
            representative_rgb,
        } => Some(CatalogIndexRecord::Shaped(AssetShapeUpdate {
            asset_id: *asset_id,
            width: *width,
            height: *height,
            orientation: *orientation,
            representative_rgb: representative_rgb.map(|colour| {
                (u32::from(colour.red) << 16)
                    | (u32::from(colour.green) << 8)
                    | u32::from(colour.blue)
            }),
        })),
        IndexEvent::MetadataReady { asset_id, metadata } => {
            Some(CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                asset_id: *asset_id,
                captured_at_utc: metadata.captured_at.map(|value| value.to_rfc3339()),
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
    }
}
