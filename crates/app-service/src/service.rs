use crate::{AppConfig, BootstrapState, SettingsState, SourceAvailability, SourceSummary};
use photo_cache::{CacheBudget, CacheError, CacheWriter, ProtectedGroups};
use photo_catalog::{Catalog, CatalogError, NewFolderGroup, StoredSourceSelection, WallOrder};
use photo_core::{AddLibraryError, LibraryService, LocalStateError, RealSourceFs};
use photo_domain::{Appearance, AssetId, Availability};
use photo_indexer::{DefaultMetadataReader, IndexScheduler, MetadataReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub(crate) struct ServiceState {
    pub(crate) libraries: LibraryService<RealSourceFs>,
    pub(crate) active_scan: Option<(photo_domain::LibraryId, u64)>,
    pub(crate) active_cancel: Option<watch::Sender<bool>>,
    pub(crate) protected_group: Option<photo_domain::FolderGroupId>,
    pub(crate) recent_derivative_ids: Vec<AssetId>,
}

#[derive(Clone)]
pub(crate) struct ReaderAdapter(Arc<dyn MetadataReader>);

impl MetadataReader for ReaderAdapter {
    fn read(
        &self,
        media_path: &Path,
        sidecar_path: Option<&Path>,
    ) -> Result<photo_metadata::MetadataBundle, photo_metadata::MetadataReadWarning> {
        self.0.read(media_path, sidecar_path)
    }
}

#[derive(Clone)]
pub struct AppService {
    pub(crate) state: Arc<Mutex<ServiceState>>,
    pub(crate) scheduler: Arc<IndexScheduler>,
    pub(crate) updates: tokio::sync::broadcast::Sender<crate::WallUpdate>,
    pub(crate) catalog_path: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_budget: CacheBudget,
    pub(crate) protected_groups: ProtectedGroups,
    pub(crate) derivative_queue: Arc<crate::derivatives::DerivativeQueue>,
    pub(crate) metadata_reader: ReaderAdapter,
}

#[derive(Debug, thiserror::Error)]
pub enum AppServiceError {
    #[error("local state setup failed: {0}")]
    LocalState(#[from] LocalStateError),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] CatalogError),
    #[error("cache setup failed: {0}")]
    Cache(#[from] CacheError),
    #[error("library service setup failed: {0}")]
    LibrarySetup(#[source] std::io::Error),
    #[error("folder selection failed: {0}")]
    OpenRecent(#[from] AddLibraryError),
    #[error("service state lock was poisoned")]
    StatePoisoned,
    #[error("invalid wall cursor")]
    InvalidCursor,
    #[error("wall cursor serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("wall page limit must be between 1 and 250")]
    InvalidLimit,
    #[error("invalid asset identifier")]
    InvalidAssetId,
    #[error("asset is not in the active wall")]
    ForeignAsset,
    #[error("asset was not found")]
    UnknownAsset,
    #[error("derivative generation failed")]
    DerivativeFailed,
}

impl AppService {
    pub fn open(config: AppConfig) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        Self::open_with_reader(config, Arc::new(DefaultMetadataReader))
    }

    pub fn open_with_reader(
        config: AppConfig,
        reader: Arc<dyn MetadataReader>,
    ) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let should_reconcile = catalog.load_app_state()?.active_selection.is_some();
        let libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;
        let (updates, _) = tokio::sync::broadcast::channel(256);
        let cache_root = config.cache_dir().to_owned();
        let cache_budget = CacheBudget::automatic(&cache_root)?;
        let catalog_path = config.catalog_path();
        let service = Self {
            state: Arc::new(Mutex::new(ServiceState {
                libraries,
                active_scan: None,
                active_cancel: None,
                protected_group: None,
                recent_derivative_ids: Vec::new(),
            })),
            scheduler: Arc::new(IndexScheduler::new(Default::default())),
            updates,
            catalog_path,
            cache_root,
            cache_budget,
            protected_groups: ProtectedGroups::default(),
            derivative_queue: Arc::new(crate::derivatives::DerivativeQueue::default()),
            metadata_reader: ReaderAdapter(reader),
        };
        if should_reconcile {
            let active_group = {
                let state = service.state()?;
                let selection = state.libraries.catalog().load_app_state()?.active_selection;
                selection
                    .map(|selection| {
                        state
                            .libraries
                            .catalog()
                            .folder_group_for_path(selection.library_id, &selection.relative_folder)
                    })
                    .transpose()?
                    .flatten()
            };
            if let Some(group) = active_group {
                service.protected_groups.protect(group)?;
                service.state()?.protected_group = Some(group);
            }
        }
        if should_reconcile && tokio::runtime::Handle::try_current().is_ok() {
            let startup = service.clone();
            tokio::spawn(async move {
                startup.reconcile_existing().await;
            });
        }
        Ok(service)
    }

    pub(crate) fn state(&self) -> Result<std::sync::MutexGuard<'_, ServiceState>, AppServiceError> {
        self.state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)
    }

    fn bootstrap_locked(state: &ServiceState) -> Result<BootstrapState, AppServiceError> {
        let stored = state.libraries.catalog().load_app_state()?;
        let active_source = match stored.active_selection {
            Some(selection) => match state
                .libraries
                .catalog()
                .find_library(selection.library_id)?
            {
                Some(library) => {
                    let relative_folder =
                        selection.relative_folder.to_path_buf().map_err(|error| {
                            CatalogError::InvalidData(format!("invalid active folder key: {error}"))
                        })?;
                    let display_name = relative_folder
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or(library.display_name);
                    Some(SourceSummary {
                        id: library.id.as_uuid().hyphenated().to_string(),
                        display_name,
                        availability: map_availability(library.availability),
                    })
                }
                None => None,
            },
            None => None,
        };
        Ok(BootstrapState {
            settings: SettingsState {
                appearance: stored.appearance,
            },
            active_source,
        })
    }

    pub fn bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        let state = self.state()?;
        Self::bootstrap_locked(&state)
    }

    /// Synchronous compatibility entry point. Task 5 will move scan launch out of this call.
    pub fn open_recent(&self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        if let Ok(mut state) = self.state.lock() {
            if let Some(cancel) = state.active_cancel.take() {
                let _ = cancel.send(true);
            }
            state.active_scan = None;
        }
        let mut state = self.state()?;
        let selection = state.libraries.open_recent(folder)?;
        state
            .libraries
            .catalog_mut()
            .set_active_selection(Some(&StoredSourceSelection {
                library_id: selection.library_id,
                relative_folder: selection.relative_folder.clone(),
            }))?;
        let display_path = folder
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Selected folder".to_owned());
        let group = state
            .libraries
            .catalog_mut()
            .upsert_folder_group(&NewFolderGroup {
                id: photo_domain::FolderGroupId::new(),
                library_id: selection.library_id,
                relative_path: selection.relative_folder,
                display_path,
                last_viewed_at: None,
            })?;
        let previous = state.protected_group.replace(group);
        let bootstrap = Self::bootstrap_locked(&state)?;
        drop(state);
        if previous != Some(group) {
            if let Some(previous) = previous {
                self.protected_groups.unprotect(previous)?;
            }
            self.protected_groups.protect(group)?;
        }
        Ok(bootstrap)
    }

    pub fn subscribe_wall_updates(&self) -> tokio::sync::broadcast::Receiver<crate::WallUpdate> {
        self.updates.subscribe()
    }

    pub async fn query_wall(
        &self,
        request: crate::WallQueryRequest,
    ) -> Result<crate::WallPage, AppServiceError> {
        if !(1..=250).contains(&request.limit) {
            return Err(AppServiceError::InvalidLimit);
        }
        let state = self.state()?;
        let stored = state.libraries.catalog().load_app_state()?;
        let Some(selection) = stored.active_selection else {
            return Ok(empty_page());
        };
        let Some(group) = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)?
        else {
            return Ok(empty_page());
        };
        let settled = state
            .libraries
            .catalog()
            .has_completed_generation_for_library(selection.library_id)?;
        let order = if settled {
            match request.direction {
                crate::SortDirection::OldestFirst => WallOrder::CapturedAscending,
                crate::SortDirection::NewestFirst => WallOrder::CapturedDescending,
            }
        } else {
            WallOrder::Provisional
        };
        let cursor = request
            .cursor
            .as_deref()
            .map(|v| crate::wall::decode_cursor(v, request.direction, order))
            .transpose()?;
        let page = state
            .libraries
            .catalog()
            .wall_page(group, order, cursor, request.limit)?;
        let derivatives = state
            .libraries
            .catalog()
            .all_derivatives()?
            .into_iter()
            .filter(|d| d.folder_group_id == group && d.kind == "wall_thumbnail")
            .collect::<Vec<_>>();
        let items = page
            .items
            .iter()
            .map(|item| crate::WallAsset {
                id: item.id.as_uuid().hyphenated().to_string(),
                captured_at_utc: item.captured_at_utc.clone(),
                width: item.width,
                height: item.height,
                wall_thumbnail: derivatives.iter().find(|d| d.asset_id == item.id).map(|d| {
                    crate::DerivativeReference {
                        asset_id: item.id.as_uuid().hyphenated().to_string(),
                        kind: crate::DerivativeClass::WallThumbnail,
                        key: d.cache_key.clone(),
                    }
                }),
            })
            .collect();
        let next_cursor = page
            .next
            .as_ref()
            .map(|k| crate::wall::encode_cursor(request.direction, k))
            .transpose()?;
        Ok(crate::WallPage {
            items,
            next_cursor,
            order_state: if settled {
                crate::OrderState::Settled
            } else {
                crate::OrderState::Provisional
            },
        })
    }

    pub fn update_appearance(
        &self,
        appearance: Appearance,
    ) -> Result<BootstrapState, AppServiceError> {
        let mut state = self.state()?;
        state.libraries.catalog_mut().set_appearance(appearance)?;
        Self::bootstrap_locked(&state)
    }
}

fn empty_page() -> crate::WallPage {
    crate::WallPage {
        items: Vec::new(),
        next_cursor: None,
        order_state: crate::OrderState::Provisional,
    }
}

fn map_availability(availability: Availability) -> SourceAvailability {
    match availability {
        Availability::Available => SourceAvailability::Available,
        Availability::RootOffline => SourceAvailability::RootOffline,
        Availability::Missing => SourceAvailability::Missing,
        Availability::Unreadable => SourceAvailability::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, AppService};

    #[test]
    fn reopened_service_restores_the_active_protected_group_marker() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        service.open_recent(&source).unwrap();
        drop(service);

        let reopened = AppService::open(config).unwrap();
        assert!(reopened.state().unwrap().protected_group.is_some());
    }
}
