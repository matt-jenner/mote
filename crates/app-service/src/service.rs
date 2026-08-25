use crate::{AppConfig, BootstrapState, SettingsState, SourceAvailability, SourceSummary};
use photo_cache::{CacheBudget, CacheError, CacheWriter, ProtectedGroups};
use photo_catalog::{Catalog, CatalogError, NewFolderGroup, StoredSourceSelection, WallOrder};
use photo_core::{
    AddLibraryError, LibraryService, LocalStateError, RealSourceFs, SourceFs, SourceValidator,
    ValidatedSourceFolder,
};
use photo_domain::{Appearance, AssetId, Availability, MediaKind};
use photo_indexer::{DefaultMetadataReader, IndexScheduler, MetadataReader};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub(crate) struct ServiceState {
    pub(crate) libraries: LibraryService<RealSourceFs>,
    pub(crate) active_scan: Option<ScanOwner>,
    pub(crate) active_cancel: Option<watch::Sender<bool>>,
    pub(crate) protected_group: Option<photo_domain::FolderGroupId>,
    pub(crate) recent_derivative_ids: Vec<AssetId>,
    pub(crate) selection_epoch: u64,
    pub(crate) published_wall_cache_warning: Option<SelectionToken>,
    pub(crate) published_screen_cache_warning: Option<SelectionToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionToken {
    pub(crate) library_id: photo_domain::LibraryId,
    pub(crate) group_id: photo_domain::FolderGroupId,
    pub(crate) epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanOwner {
    pub(crate) selection: SelectionToken,
    pub(crate) generation: u64,
}

trait RecentSourceValidator: Send + Sync {
    fn validate_recent(&self, folder: &Path) -> Result<ValidatedSourceFolder, AddLibraryError>;
}

impl<F: SourceFs> RecentSourceValidator for SourceValidator<F> {
    fn validate_recent(&self, folder: &Path) -> Result<ValidatedSourceFolder, AddLibraryError> {
        SourceValidator::validate_recent(self, folder)
    }
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
    source_validator: Arc<dyn RecentSourceValidator>,
    selection_request_sequence: Arc<AtomicU64>,
    latest_validated_selection: Arc<AtomicU64>,
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
    #[error("folder selection was superseded by a newer valid selection")]
    SelectionSuperseded,
}

impl AppService {
    pub fn open(config: AppConfig) -> Result<Self, AppServiceError> {
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
        let source_validator: Arc<dyn RecentSourceValidator> =
            Arc::new(libraries.source_validator());
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
                selection_epoch: u64::from(should_reconcile),
                published_wall_cache_warning: None,
                published_screen_cache_warning: None,
            })),
            scheduler: Arc::new(IndexScheduler::new(Default::default())),
            updates,
            catalog_path,
            cache_root,
            cache_budget,
            protected_groups: ProtectedGroups::default(),
            derivative_queue: Arc::new(crate::derivatives::DerivativeQueue::default()),
            metadata_reader: ReaderAdapter(reader),
            source_validator,
            selection_request_sequence: Arc::new(AtomicU64::new(0)),
            latest_validated_selection: Arc::new(AtomicU64::new(0)),
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
                let mut state = service.state()?;
                state
                    .libraries
                    .catalog_mut()
                    .touch_folder_group(group, unix_timestamp())?;
                state.protected_group = Some(group);
            }
        }
        if should_reconcile && tokio::runtime::Handle::try_current().is_ok() {
            let startup = service.clone();
            tokio::spawn(async move {
                tokio::task::yield_now().await;
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

    /// Synchronous host compatibility entry point. It persists the selection everywhere, but it
    /// can launch reconciliation only when the caller already runs inside a Tokio runtime. Async
    /// hosts should prefer `start_scan`, which reports scan-start failures to the caller.
    pub fn open_recent(&self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let (bootstrap, selection) = self.select_recent(folder)?;
        if tokio::runtime::Handle::try_current().is_ok() {
            let service = self.clone();
            tokio::spawn(async move {
                let _ = service.start_selected_scan(selection).await;
            });
        }
        Ok(bootstrap)
    }

    pub(crate) fn select_recent(
        &self,
        folder: &Path,
    ) -> Result<(BootstrapState, SelectionToken), AppServiceError> {
        let request_id = self
            .selection_request_sequence
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1)
            .max(1);
        let validated = self.source_validator.validate_recent(folder)?;
        let mut state = self.state()?;
        let prepared = state.libraries.prepare_validated_recent(validated)?;
        self.latest_validated_selection
            .fetch_max(request_id, Ordering::SeqCst);
        if self.latest_validated_selection.load(Ordering::SeqCst) != request_id {
            return Err(AppServiceError::SelectionSuperseded);
        }
        let selection = prepared.open()?;
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
                relative_path: selection.relative_folder.clone(),
                display_path,
                last_viewed_at: Some(unix_timestamp()),
            })?;
        let previous = state.protected_group;
        if previous != Some(group) {
            self.protected_groups.protect(group)?;
        }
        if let Err(error) =
            state
                .libraries
                .catalog_mut()
                .set_active_selection(Some(&StoredSourceSelection {
                    library_id: selection.library_id,
                    relative_folder: selection.relative_folder,
                }))
        {
            if previous != Some(group) {
                let _ = self.protected_groups.unprotect(group);
            }
            return Err(error.into());
        }
        if let Some(cancel) = state.active_cancel.take() {
            let _ = cancel.send(true);
        }
        state.active_scan = None;
        state.published_wall_cache_warning = None;
        state.published_screen_cache_warning = None;
        state.selection_epoch = state.selection_epoch.wrapping_add(1).max(1);
        if previous != Some(group) {
            if let Some(previous) = previous {
                self.protected_groups.unprotect(previous)?;
            }
            state.protected_group = Some(group);
        }
        let token = SelectionToken {
            library_id: selection.library_id,
            group_id: group,
            epoch: state.selection_epoch,
        };
        let bootstrap = Self::bootstrap_locked(&state)?;
        drop(state);
        if tokio::runtime::Handle::try_current().is_ok() {
            let queue = self.derivative_queue.clone();
            tokio::spawn(async move {
                queue.invalidate_except(token).await;
            });
        }
        Ok((bootstrap, token))
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
            .has_completed_generation_for_group(selection.library_id, group)?;
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
            .map(|v| crate::wall::decode_cursor(v, request.direction, group, order))
            .transpose()?;
        let page = state
            .libraries
            .catalog()
            .wall_page(group, order, cursor, request.limit)?;
        let date_state = if settled {
            crate::OrderState::Settled
        } else {
            crate::OrderState::Provisional
        };
        let items =
            wall_assets_with_derivatives(state.libraries.catalog(), &page.items, date_state)?;
        let next_cursor = page
            .next
            .as_ref()
            .map(|k| crate::wall::encode_cursor(request.direction, group, k))
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

pub(crate) fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
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

pub(crate) fn wall_asset_from_record(
    item: &photo_catalog::WallCatalogRecord,
    date_state: crate::OrderState,
    wall_key: Option<&str>,
    screen_key: Option<&str>,
) -> crate::WallAsset {
    let id = item.id.as_uuid().hyphenated().to_string();
    let display_name = std::path::Path::new(&item.display_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Photo".to_owned());
    let warning = if item.shape_status == photo_catalog::ShapeStatus::Fallback {
        Some(crate::WallWarningState {
            code: "shapeFallback".to_owned(),
            retryable: true,
        })
    } else if item.availability != Availability::Available {
        Some(crate::WallWarningState {
            code: "sourceUnavailable".to_owned(),
            retryable: true,
        })
    } else if item.has_warning {
        Some(crate::WallWarningState {
            code: "assetWarning".to_owned(),
            retryable: true,
        })
    } else {
        None
    };
    crate::WallAsset {
        id: id.clone(),
        display_name,
        media_kind: map_media_kind(item.media_kind),
        provisional_order: item.provisional_order,
        captured_at_utc: item.captured_at_utc.clone(),
        date_state,
        width: item.width,
        height: item.height,
        representative_rgb: item.representative_rgb,
        shape_state: match item.shape_status {
            photo_catalog::ShapeStatus::Ready => crate::WallShapeState::Ready,
            photo_catalog::ShapeStatus::Fallback => crate::WallShapeState::Fallback,
            photo_catalog::ShapeStatus::Pending => crate::WallShapeState::Fallback,
        },
        availability: map_availability(item.availability),
        warning,
        wall_thumbnail: wall_key.map(|key| crate::DerivativeReference {
            asset_id: id.clone(),
            kind: crate::DerivativeClass::WallThumbnail,
            key: key.to_owned(),
        }),
        screen_preview: screen_key.map(|key| crate::DerivativeReference {
            asset_id: id,
            kind: crate::DerivativeClass::ScreenPreview,
            key: key.to_owned(),
        }),
    }
}

pub(crate) fn wall_assets_with_derivatives(
    catalog: &Catalog,
    records: &[photo_catalog::WallCatalogRecord],
    date_state: crate::OrderState,
) -> Result<Vec<crate::WallAsset>, CatalogError> {
    let asset_ids = records.iter().map(|item| item.id).collect::<Vec<_>>();
    let latest_by_asset =
        |kind| -> Result<HashMap<AssetId, photo_catalog::DerivativeRecord>, CatalogError> {
            Ok(catalog
                .derivatives_for_assets(&asset_ids, kind)?
                .into_iter()
                .fold(HashMap::new(), |mut rows, derivative| {
                    rows.entry(derivative.asset_id).or_insert(derivative);
                    rows
                }))
        };
    let wall_derivatives = latest_by_asset("wall_thumbnail")?;
    let screen_derivatives = latest_by_asset("screen_preview")?;
    Ok(records
        .iter()
        .map(|item| {
            wall_asset_from_record(
                item,
                date_state,
                wall_derivatives
                    .get(&item.id)
                    .map(|row| row.cache_key.as_str()),
                screen_derivatives
                    .get(&item.id)
                    .map(|row| row.cache_key.as_str()),
            )
        })
        .collect())
}

fn map_media_kind(kind: MediaKind) -> crate::WallMediaKind {
    match kind {
        MediaKind::Jpeg => crate::WallMediaKind::Jpeg,
        MediaKind::Png => crate::WallMediaKind::Png,
        MediaKind::Tiff => crate::WallMediaKind::Tiff,
        MediaKind::Heif => crate::WallMediaKind::Heif,
        MediaKind::Webp => crate::WallMediaKind::Webp,
        MediaKind::Avif => crate::WallMediaKind::Avif,
        MediaKind::Raw => crate::WallMediaKind::Raw,
        MediaKind::Video => crate::WallMediaKind::Video,
        MediaKind::Unknown => crate::WallMediaKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;

    use chrono::{FixedOffset, TimeZone};
    use image::{ImageBuffer, Rgb};
    use photo_catalog::{AssetShapeUpdate, CatalogIndexRecord, NewAsset, ShapeStatus};
    use photo_domain::{Appearance, MediaKind, RelativePathKey};
    use photo_metadata::{MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource};

    use super::{AppConfig, AppService, AppServiceError, RecentSourceValidator};

    struct BlockingSourceValidator {
        delegate: Arc<dyn RecentSourceValidator>,
        target: std::path::PathBuf,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl BlockingSourceValidator {
        fn install(
            service: &mut AppService,
            target: std::path::PathBuf,
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        ) {
            service.source_validator = Arc::new(Self {
                delegate: service.source_validator.clone(),
                target,
                entered,
                release: Mutex::new(release),
            });
        }
    }

    impl RecentSourceValidator for BlockingSourceValidator {
        fn validate_recent(
            &self,
            folder: &Path,
        ) -> Result<photo_core::ValidatedSourceFolder, photo_core::AddLibraryError> {
            if folder == self.target {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            self.delegate.validate_recent(folder)
        }
    }

    #[derive(Clone)]
    struct BlockingReader(Arc<(Mutex<bool>, Condvar)>);

    impl photo_indexer::MetadataReader for BlockingReader {
        fn read(
            &self,
            _media_path: &Path,
            _sidecar_path: Option<&Path>,
        ) -> Result<MetadataBundle, MetadataReadWarning> {
            let (lock, changed) = &*self.0;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = changed.wait(released).unwrap();
            }
            let captured = FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(1_700_000_000, 0)
                .single()
                .unwrap();
            Ok(MetadataBundle {
                capture_dates: vec![MetadataCandidate {
                    value: captured,
                    source: MetadataSource::FilesystemModified,
                    raw_value: captured.to_rfc3339(),
                }],
                ..MetadataBundle::default()
            })
        }
    }

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_source_validation_does_not_block_cached_wall_or_settings_access() {
        let temp = tempfile::tempdir().unwrap();
        let active = temp.path().join("active");
        let stalled = temp.path().join("stalled");
        std::fs::create_dir_all(&active).unwrap();
        std::fs::create_dir_all(&stalled).unwrap();
        let mut service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_, selection) = service.select_recent(&active).unwrap();
        seed_wall_asset(&service, selection, "cached.jpg");
        let (entered_send, entered_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        BlockingSourceValidator::install(
            &mut service,
            stalled.clone(),
            entered_send,
            release_receive,
        );

        let selecting_service = service.clone();
        let selecting_path = stalled.clone();
        let selecting =
            std::thread::spawn(move || selecting_service.select_recent(&selecting_path));
        entered_receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let wall = tokio::time::timeout(
            Duration::from_millis(250),
            service.query_wall(crate::WallQueryRequest::oldest_first()),
        )
        .await
        .expect("cached wall access must not wait for source validation")
        .unwrap();
        assert_eq!(wall.items.len(), 1);
        let settings_service = service.clone();
        let (settings_send, settings_receive) = mpsc::channel();
        std::thread::spawn(move || {
            settings_send
                .send(settings_service.update_appearance(Appearance::Dark))
                .unwrap();
        });
        assert_eq!(
            settings_receive
                .recv_timeout(Duration::from_millis(250))
                .expect("bounded catalog updates must not wait for source validation")
                .unwrap()
                .settings
                .appearance,
            Appearance::Dark
        );

        release_send.send(()).unwrap();
        selecting.join().unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn newer_valid_selection_supersedes_an_older_stalled_selection() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(first.join("first.jpg"))
            .unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([30_u8, 20, 10]))
            .save(second.join("second.jpg"))
            .unwrap();
        let reader_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut service = AppService::open_with_reader(
            AppConfig::new(temp.path().join("data"), temp.path().join("cache")),
            Arc::new(BlockingReader(reader_gate.clone())),
        )
        .unwrap();
        let (entered_send, entered_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        BlockingSourceValidator::install(
            &mut service,
            first.clone(),
            entered_send,
            release_receive,
        );

        let first_service = service.clone();
        let first_path = first.clone();
        let first_call = std::thread::spawn(move || first_service.select_recent(&first_path));
        entered_receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let second_token = service.select_recent(&second).unwrap().1;
        release_send.send(()).unwrap();
        assert!(matches!(
            first_call.join().unwrap(),
            Err(AppServiceError::SelectionSuperseded)
        ));

        let final_token = {
            let state = service.state().unwrap();
            let stored = state
                .libraries
                .catalog()
                .load_app_state()
                .unwrap()
                .active_selection
                .unwrap();
            let stored_group = state
                .libraries
                .catalog()
                .folder_group_for_path(stored.library_id, &stored.relative_folder)
                .unwrap()
                .unwrap();
            let final_token = second_token;
            assert_eq!(final_token.library_id, stored.library_id);
            assert_eq!(final_token.group_id, stored_group);
            assert_eq!(state.protected_group, Some(stored_group));
            final_token
        };

        let mut updates = service.subscribe_wall_updates();
        service.start_selected_scan(final_token).await.unwrap();
        assert_eq!(
            service.state().unwrap().active_scan.unwrap().selection,
            final_token,
            "the scan owner must match the final persisted selection"
        );
        let (lock, changed) = &*reader_gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
        let mut published = Vec::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(2), updates.recv())
                .await
                .unwrap()
                .unwrap()
            {
                crate::WallUpdate::CatalogBatch { assets, .. } => {
                    published.extend(assets.into_iter().map(|asset| asset.id));
                }
                crate::WallUpdate::MetadataSettled { .. } => break,
                _ => {}
            }
        }
        let active = service
            .query_wall(crate::WallQueryRequest::oldest_first())
            .await
            .unwrap();
        assert_eq!(active.items.len(), 1);
        assert!(
            published.iter().all(|id| id == &active.items[0].id),
            "only the latest valid selection may publish wall rows"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn newer_catalog_invalid_selection_does_not_supersede_an_older_valid_selection() {
        let temp = tempfile::tempdir().unwrap();
        let existing_parent = temp.path().join("existing-parent");
        let existing = existing_parent.join("existing-root");
        let older_valid = temp.path().join("older-valid");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::create_dir_all(&older_valid).unwrap();
        let mut service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        service.select_recent(&existing).unwrap();
        let (entered_send, entered_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        BlockingSourceValidator::install(
            &mut service,
            older_valid.clone(),
            entered_send,
            release_receive,
        );

        let older_service = service.clone();
        let older_path = older_valid.clone();
        let older_call = std::thread::spawn(move || older_service.select_recent(&older_path));
        entered_receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        assert!(matches!(
            service.select_recent(&existing_parent),
            Err(AppServiceError::OpenRecent(
                photo_core::AddLibraryError::Overlaps { .. }
            ))
        ));
        release_send.send(()).unwrap();
        let older_token = older_call.join().unwrap().unwrap().1;

        let state = service.state().unwrap();
        let stored = state
            .libraries
            .catalog()
            .load_app_state()
            .unwrap()
            .active_selection
            .unwrap();
        assert_eq!(stored.library_id, older_token.library_id);
        assert_eq!(state.selection_epoch, older_token.epoch);
        assert_eq!(state.protected_group, Some(older_token.group_id));
    }

    fn seed_wall_asset(service: &AppService, selection: super::SelectionToken, name: &str) {
        let mut state = service.state().unwrap();
        let mut asset = NewAsset::minimal(
            selection.library_id,
            RelativePathKey::from_relative_path(Path::new(name)).unwrap(),
            name.to_owned(),
            MediaKind::Jpeg,
            1,
        );
        asset.folder_group_id = Some(selection.group_id);
        let id = asset.id;
        state.libraries.catalog_mut().upsert_asset(&asset).unwrap();
        state
            .libraries
            .catalog_mut()
            .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                asset_id: id,
                width: 16,
                height: 9,
                orientation: Some(1),
                representative_rgb: None,
                shape_status: ShapeStatus::Ready,
            })])
            .unwrap();
    }
}
