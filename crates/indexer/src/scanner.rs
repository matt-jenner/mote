use std::path::{Path, PathBuf};
use std::sync::Arc;

use photo_core::{FolderPolicyEngine, FolderStructureSnapshot};
use photo_domain::LibraryId;
use photo_metadata::{
    MediaProbe, MetadataBundle, MetadataReadWarning, MetadataResolver, RepresentativeRgb,
};
use tokio::sync::{mpsc, watch};

use crate::discover::{DiscoveredAsset, discover_asset};
use crate::{IndexError, IndexEvent, ScanSummary};

pub trait MetadataReader: Send + Sync + 'static {
    fn read(
        &self,
        media_path: &Path,
        sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning>;
}

pub struct Indexer<R> {
    reader: Arc<R>,
    policy_engine: Arc<FolderPolicyEngine>,
}

pub struct ScanRequest {
    pub root: PathBuf,
    pub library_id: LibraryId,
}

impl ScanRequest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            library_id: LibraryId::new(),
        }
    }

    pub fn for_library(mut self, library_id: LibraryId) -> Self {
        self.library_id = library_id;
        self
    }
}

pub struct ScanHandle {
    pub events: mpsc::Receiver<IndexEvent>,
    cancel: watch::Sender<bool>,
    join: tokio::task::JoinHandle<Result<ScanSummary, IndexError>>,
}

impl ScanHandle {
    pub fn cancel(&self) -> Result<(), IndexError> {
        self.cancel
            .send(true)
            .map_err(|_| IndexError::CancellationClosed)
    }

    pub async fn join(self) -> Result<ScanSummary, IndexError> {
        let Self {
            events,
            cancel: _,
            join,
        } = self;
        drop(events);
        join.await
            .map_err(|error| IndexError::TaskJoin(error.to_string()))?
    }
}

impl<R: MetadataReader> Indexer<R> {
    pub fn new(reader: R, policy_engine: FolderPolicyEngine) -> Self {
        Self {
            reader: Arc::new(reader),
            policy_engine: Arc::new(policy_engine),
        }
    }

    pub fn start(&self, request: ScanRequest) -> Result<ScanHandle, IndexError> {
        if !request.root.is_dir() {
            return Err(IndexError::RootUnavailable(request.root));
        }
        let root = std::fs::canonicalize(request.root)?;
        let (events_tx, events) = mpsc::channel(128);
        let (discovered_tx, mut discovered_rx) = mpsc::channel(32);
        let (cancel, cancel_rx) = watch::channel(false);
        let reader = self.reader.clone();
        let policy_engine = self.policy_engine.clone();
        let discovery_root = root.clone();
        let discovery_cancel = cancel_rx.clone();
        let library_id = request.library_id;
        let discovery = tokio::task::spawn_blocking(move || {
            discover_all(
                &discovery_root,
                library_id,
                discovered_tx,
                discovery_cancel,
                &policy_engine,
            )
        });
        let join = tokio::spawn(async move {
            let mut summary = ScanSummary::default();
            while let Some(item) = discovered_rx.recv().await {
                if *cancel_rx.borrow() {
                    summary.cancelled = true;
                    break;
                }
                summary.discovered += 1;
                let asset_id = item.asset.id;
                let _ = events_tx
                    .send(IndexEvent::Discovered {
                        asset: item.asset.clone(),
                    })
                    .await;
                if *cancel_rx.borrow() {
                    summary.cancelled = true;
                    break;
                }
                let reader = reader.clone();
                let processed = tokio::task::spawn_blocking(move || process_asset(reader, item))
                    .await
                    .map_err(|error| IndexError::TaskJoin(error.to_string()))?;
                match processed {
                    Ok(processed) => {
                        let _ = events_tx
                            .send(IndexEvent::Shaped {
                                asset_id,
                                width: processed.width,
                                height: processed.height,
                                orientation: processed.orientation,
                                representative_rgb: processed.representative_rgb,
                            })
                            .await;
                        if let Some(warning) = processed.colour_warning {
                            let _ = events_tx
                                .send(IndexEvent::Warning {
                                    asset_id: Some(asset_id),
                                    code: warning.code,
                                    message: warning.message,
                                })
                                .await;
                        }
                        let _ = events_tx
                            .send(IndexEvent::MetadataReady {
                                asset_id,
                                metadata: MetadataResolver::resolve(processed.metadata),
                            })
                            .await;
                    }
                    Err(warning) => {
                        summary.failed += 1;
                        let _ = events_tx
                            .send(IndexEvent::Warning {
                                asset_id: Some(asset_id),
                                code: warning.code,
                                message: warning.message,
                            })
                            .await;
                    }
                }
            }
            if *cancel_rx.borrow() {
                summary.cancelled = true;
            }
            drop(discovered_rx);
            match discovery.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if summary.cancelled => {
                    tracing::debug!(%error, "discovery stopped after cancellation");
                }
                Ok(Err(error)) => return Err(error),
                Err(error) => return Err(IndexError::TaskJoin(error.to_string())),
            }
            let _ = events_tx.send(IndexEvent::Completed(summary)).await;
            Ok(summary)
        });
        Ok(ScanHandle {
            events,
            cancel,
            join,
        })
    }
}

struct ProcessedAsset {
    width: u32,
    height: u32,
    orientation: Option<u16>,
    representative_rgb: Option<RepresentativeRgb>,
    colour_warning: Option<MetadataReadWarning>,
    metadata: MetadataBundle,
}

fn process_asset<R: MetadataReader>(
    reader: Arc<R>,
    item: DiscoveredAsset,
) -> Result<ProcessedAsset, MetadataReadWarning> {
    let shape = MediaProbe::shape(&item.source_path)?;
    let colour = MediaProbe::representative_rgb(&item.source_path);
    let metadata = reader.read(&item.source_path, item.sidecar_path.as_deref())?;
    Ok(ProcessedAsset {
        width: shape.width,
        height: shape.height,
        orientation: metadata.orientation,
        representative_rgb: colour.as_ref().ok().copied(),
        colour_warning: colour.err(),
        metadata,
    })
}

fn discover_all(
    root: &Path,
    library_id: LibraryId,
    sender: mpsc::Sender<DiscoveredAsset>,
    cancel: watch::Receiver<bool>,
    policy_engine: &FolderPolicyEngine,
) -> Result<(), IndexError> {
    let children = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    let structure = FolderStructureSnapshot::new(children);
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        if *cancel.borrow() {
            break;
        }
        let entry = entry.map_err(IndexError::Walk)?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(asset) = discover_asset(root, entry.path(), library_id)? else {
            continue;
        };
        let relative = asset
            .source_path
            .strip_prefix(root)
            .map_err(|_| IndexError::PathOutsideRoot(asset.source_path.clone()))?;
        policy_engine
            .classify(relative, &structure)
            .map_err(|error| IndexError::Policy(error.to_string()))?;
        if sender.blocking_send(asset).is_err() {
            break;
        }
    }
    Ok(())
}
