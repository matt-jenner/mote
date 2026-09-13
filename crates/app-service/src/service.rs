use crate::{
    AppConfig, BootstrapState, DerivativeClass, InteractionState, SettingsState,
    SourceAvailability, SourceSummary,
};
use photo_cache::{CacheBudget, CacheError, CacheWriter, ProtectedGroups};
use photo_catalog::{Catalog, CatalogError, NewFolderGroup};
use photo_core::{
    AddLibraryError, LibraryService, LocalStateError, RealSourceFs, SourceFs, SourceValidator,
    ValidatedSourceFolder,
};
use photo_domain::{Appearance, AssetId, Availability, GalleryScope, MediaKind};
use photo_indexer::{DefaultMetadataReader, IndexScheduler, MetadataReader};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(any(test, debug_assertions))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::Notify;

pub(crate) struct ServiceState {
    pub(crate) libraries: LibraryService<RealSourceFs>,
    pub(crate) folder_status: HashMap<String, crate::FolderAccess>,
    pub(crate) folder_revision: u64,
    pub(crate) active_scan: Option<ScanOwner>,
    pub(crate) protected_group: Option<photo_domain::FolderGroupId>,
    pub(crate) selection_epoch: u64,
    pub(crate) published_wall_cache_warning: Option<SelectionToken>,
    pub(crate) published_screen_cache_warning: Option<SelectionToken>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SelectionToken {
    pub(crate) library_id: photo_domain::LibraryId,
    pub(crate) group_id: photo_domain::FolderGroupId,
    pub(crate) epoch: u64,
}

impl SelectionToken {
    pub(crate) fn selection_id(self) -> String {
        opaque_selection_id(
            self.library_id.as_uuid().as_bytes(),
            self.group_id.as_uuid().as_bytes(),
            self.epoch,
        )
    }
}

fn opaque_selection_id(library: &[u8], group: &[u8], epoch: u64) -> String {
    let mut identity = Vec::with_capacity(library.len() + group.len() + 8);
    identity.extend_from_slice(library);
    identity.extend_from_slice(group);
    identity.extend_from_slice(&epoch.to_be_bytes());
    format!(
        "selection-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &identity).hyphenated()
    )
}

fn fallback_selection_id(
    library: photo_domain::LibraryId,
    relative_folder: &photo_domain::RelativePathKey,
    epoch: u64,
) -> String {
    opaque_selection_id(
        library.as_uuid().as_bytes(),
        relative_folder.as_bytes(),
        epoch,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanOwner {
    pub(crate) selection: SelectionToken,
    pub(crate) generation: u64,
}

pub(crate) struct DesktopBridgeEntry {
    pub(crate) id: u64,
    pub(crate) abort: tokio::task::AbortHandle,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
pub(crate) struct DerivativeTestGate {
    pub(crate) blocked_asset: Option<AssetId>,
    pub(crate) class: Option<DerivativeClass>,
    pub(crate) entered: Arc<tokio::sync::Notify>,
    pub(crate) release: Arc<tokio::sync::Notify>,
    pub(crate) starts: Option<Arc<AtomicUsize>>,
}

#[derive(Clone, Copy)]
pub(crate) struct CollectionDriverIntent {
    pub(crate) selection: SelectionToken,
    pub(crate) full_group: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CollectionDriverOwnerId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CollectionDriverAdmission {
    pub(crate) owner_id: CollectionDriverOwnerId,
    pub(crate) initial_request_generation: u64,
}

pub(crate) enum CollectionDriverTake {
    Pending(CollectionDriverIntent),
    Released,
    Stale,
}

struct CollectionDriverControlState {
    pending: Option<CollectionDriverIntent>,
    pending_request_generation: Option<u64>,
    // Selection epochs are issued monotonically. Retain the latest one even
    // after its intent has been taken so a delayed older caller cannot regress
    // the driver's work while it is between batches.
    latest_selection: Option<SelectionToken>,
    next_owner_id: u64,
    next_request_generation: u64,
    admitted: Option<CollectionDriverAdmission>,
}

pub(crate) struct CollectionDriverControl {
    state: Mutex<CollectionDriverControlState>,
    wake: Notify,
}

impl CollectionDriverControl {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(CollectionDriverControlState {
                pending: None,
                pending_request_generation: None,
                latest_selection: None,
                next_owner_id: 1,
                next_request_generation: 1,
                admitted: None,
            }),
            wake: Notify::new(),
        }
    }

    pub(crate) fn request(
        &self,
        selection: SelectionToken,
        full_group: bool,
    ) -> Option<CollectionDriverAdmission> {
        let admission = {
            let mut state = self
                .state
                .lock()
                .expect("collection driver control state poisoned");
            let is_current = state
                .latest_selection
                .is_none_or(|current| selection == current);
            let is_newer = state
                .latest_selection
                .is_some_and(|current| selection.epoch > current.epoch);
            let accepted = state.latest_selection.is_none() || is_current || is_newer;
            if !accepted {
                None
            } else {
                state.latest_selection = Some(selection);
                let request_generation = state.next_request_generation;
                state.next_request_generation =
                    state.next_request_generation.wrapping_add(1).max(1);
                match state.pending.as_mut() {
                    Some(intent) if intent.selection == selection => {
                        intent.full_group |= full_group;
                    }
                    _ => {
                        state.pending = Some(CollectionDriverIntent {
                            selection,
                            full_group,
                        });
                    }
                }
                state.pending_request_generation = Some(request_generation);
                if state.admitted.is_some() {
                    None
                } else {
                    let admission = CollectionDriverAdmission {
                        owner_id: CollectionDriverOwnerId(state.next_owner_id),
                        initial_request_generation: request_generation,
                    };
                    state.next_owner_id = state.next_owner_id.wrapping_add(1).max(1);
                    state.admitted = Some(admission);
                    Some(admission)
                }
            }
        };
        self.wake.notify_waiters();
        admission
    }

    pub(crate) fn take_pending_or_release(
        &self,
        owner_id: CollectionDriverOwnerId,
    ) -> CollectionDriverTake {
        let mut state = self
            .state
            .lock()
            .expect("collection driver control state poisoned");
        if state
            .admitted
            .is_none_or(|admission| admission.owner_id != owner_id)
        {
            return CollectionDriverTake::Stale;
        }
        if let Some(intent) = state.pending.take() {
            state.pending_request_generation = None;
            return CollectionDriverTake::Pending(intent);
        }
        state.admitted = None;
        CollectionDriverTake::Released
    }

    pub(crate) fn abort_owner(
        &self,
        admission: CollectionDriverAdmission,
        intent_consumed: bool,
    ) -> Option<CollectionDriverAdmission> {
        let mut state = self
            .state
            .lock()
            .expect("collection driver control state poisoned");
        if state.admitted != Some(admission) {
            return None;
        }
        let preserve_pending = intent_consumed
            || state.pending_request_generation != Some(admission.initial_request_generation);
        if preserve_pending && state.pending.is_some() {
            let successor = CollectionDriverAdmission {
                owner_id: CollectionDriverOwnerId(state.next_owner_id),
                initial_request_generation: state
                    .pending_request_generation
                    .expect("pending collection intent has a request generation"),
            };
            state.next_owner_id = state.next_owner_id.wrapping_add(1).max(1);
            state.admitted = Some(successor);
            Some(successor)
        } else {
            state.pending = None;
            state.pending_request_generation = None;
            state.admitted = None;
            None
        }
    }

    pub(crate) fn abandon_owner(&self, admission: CollectionDriverAdmission) {
        let mut state = self
            .state
            .lock()
            .expect("collection driver control state poisoned");
        if state.admitted == Some(admission) {
            state.pending = None;
            state.pending_request_generation = None;
            state.admitted = None;
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn is_admitted(&self) -> bool {
        self.state
            .lock()
            .expect("collection driver control state poisoned")
            .admitted
            .is_some()
    }
}

#[cfg(any(test, debug_assertions))]
pub(crate) struct DerivativeTaskTracker {
    active: AtomicUsize,
    #[cfg(debug_assertions)]
    waiting: AtomicUsize,
    #[cfg(debug_assertions)]
    quiesced: Notify,
}

#[cfg(any(test, debug_assertions))]
pub(crate) struct DerivativeTaskGuard {
    tracker: Arc<DerivativeTaskTracker>,
}

#[cfg(debug_assertions)]
struct DerivativeQuiescenceWaiter<'a> {
    tracker: &'a DerivativeTaskTracker,
}

#[cfg(any(test, debug_assertions))]
impl DerivativeTaskTracker {
    pub(crate) fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            waiting: AtomicUsize::new(0),
            #[cfg(debug_assertions)]
            quiesced: Notify::new(),
        }
    }

    pub(crate) fn start(self: &Arc<Self>) -> DerivativeTaskGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        DerivativeTaskGuard {
            tracker: Arc::clone(self),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) fn waiting_count(&self) -> usize {
        self.waiting.load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    pub(crate) async fn wait_for_zero(&self) {
        let notified = self.quiesced.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        self.waiting.fetch_add(1, Ordering::AcqRel);
        let _waiter = DerivativeQuiescenceWaiter { tracker: self };
        if self.active_count() == 0 {
            return;
        }
        notified.await;
        loop {
            let notified = self.quiesced.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active_count() == 0 {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(debug_assertions)]
impl Drop for DerivativeQuiescenceWaiter<'_> {
    fn drop(&mut self) {
        self.tracker.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(any(test, debug_assertions))]
impl Drop for DerivativeTaskGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        let previous = self.tracker.active.fetch_sub(1, Ordering::AcqRel);
        #[cfg(not(debug_assertions))]
        let _ = self.tracker.active.fetch_sub(1, Ordering::AcqRel);
        #[cfg(debug_assertions)]
        if previous == 1 {
            // Broadcast the transition to zero so every concurrent
            // wait_for_zero caller can finish. Each waiter enables its
            // notification future before checking the count, so this cannot
            // be lost between the check and registration.
            self.tracker.quiesced.notify_waiters();
        } else {
            self.tracker.quiesced.notify_one();
        }
    }
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
pub(crate) struct CollectionTestGate {
    pub(crate) entered: Arc<Notify>,
    pub(crate) release: Arc<Notify>,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
pub(crate) struct ManagedCommitTestGate {
    pub(crate) started: Arc<Notify>,
    pub(crate) release: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(debug_assertions)]
#[derive(Clone)]
pub(crate) struct DerivativeRequestTestHook {
    pub(crate) started: Arc<Notify>,
    pub(crate) release: Option<Arc<Notify>>,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ScanCompletionWakeTestHook {
    pub(crate) selection: SelectionToken,
    pub(crate) marker: Arc<Notify>,
}

#[cfg(test)]
pub(crate) struct BlockingTestGate {
    pub(crate) entered: std::sync::mpsc::Sender<()>,
    pub(crate) release: std::sync::mpsc::Receiver<()>,
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

impl ReaderAdapter {
    pub(crate) fn new(reader: Arc<dyn MetadataReader>) -> Self {
        Self(reader)
    }
}

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
    pub(crate) gallery: crate::GalleryEngine,
    pub(crate) pick_gallery: crate::GalleryEngine,
    pub(crate) state: Arc<Mutex<ServiceState>>,
    pub(crate) scheduler: Arc<IndexScheduler>,
    #[allow(dead_code)]
    pub(crate) coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    pub(crate) derivative_runtime: Option<Arc<crate::hosted_runtime::SelectionRuntime>>,
    pub(crate) updates: tokio::sync::broadcast::Sender<crate::WallUpdate>,
    pub(crate) catalog_path: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_budget: CacheBudget,
    pub(crate) protected_groups: ProtectedGroups,
    pub(crate) derivative_driver: Arc<tokio::sync::Mutex<()>>,
    pub(crate) collection_driver: Arc<CollectionDriverControl>,
    pub(crate) gallery_scope_update: Arc<TokioMutex<()>>,
    #[cfg(test)]
    pub(crate) saved_activation_test_gate: Arc<TokioMutex<Option<CollectionTestGate>>>,
    #[cfg(test)]
    pub(crate) derivative_binding_test_gate: Arc<Mutex<Option<BlockingTestGate>>>,
    #[cfg(test)]
    pub(crate) scan_admission_test_gate: Arc<TokioMutex<Option<CollectionTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_test_gate: Arc<TokioMutex<Option<DerivativeTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_task_tracker: Arc<DerivativeTaskTracker>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) collection_driver_tracker: Arc<DerivativeTaskTracker>,
    #[cfg(debug_assertions)]
    pub(crate) collection_driver_exit_test_gate: Arc<TokioMutex<Option<CollectionTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) collection_publish_test_gate: Arc<TokioMutex<Option<CollectionTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) collection_enqueue_test_gate: Arc<TokioMutex<Option<CollectionTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) screen_preview_commit_test_counter: Arc<TokioMutex<Option<Arc<AtomicUsize>>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) screen_preview_encode_test_counter: Arc<TokioMutex<Option<Arc<AtomicUsize>>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) screen_preview_post_encode_test_gate: Arc<TokioMutex<Option<DerivativeTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) screen_preview_post_admission_test_gate: Arc<TokioMutex<Option<DerivativeTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_completion_test_hook: Arc<TokioMutex<Option<Arc<Notify>>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_attempt_abort_handle: Arc<
        TokioMutex<
            Option<(
                crate::derivative_coordinator::WorkTicket,
                tokio::task::AbortHandle,
            )>,
        >,
    >,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_panic_after_admission_test_hook: Arc<TokioMutex<bool>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) managed_commit_test_gate: Arc<TokioMutex<Option<ManagedCommitTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    pub(crate) derivative_panic_in_blocking_commit_test_hook: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(debug_assertions)]
    pub(crate) derivative_request_test_hook: Arc<TokioMutex<Option<DerivativeRequestTestHook>>>,
    #[cfg(debug_assertions)]
    pub(crate) derivative_visible_queue_test_hook: Arc<TokioMutex<Option<Arc<Notify>>>>,
    #[cfg(test)]
    pub(crate) scan_completion_wake_test_hook: Arc<Mutex<Option<ScanCompletionWakeTestHook>>>,
    pub(crate) desktop_bridges: Arc<Mutex<HashMap<SelectionToken, DesktopBridgeEntry>>>,
    pub(crate) next_bridge_id: Arc<AtomicU64>,
    source_validator: Arc<dyn RecentSourceValidator>,
    pub(crate) folder_access: crate::FolderAccessCoordinator,
    pub(crate) selection_transition: Arc<Mutex<()>>,
    pub(crate) selection_request_sequence: Arc<AtomicU64>,
    pub(crate) latest_validated_selection: Arc<AtomicU64>,
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
    #[error("asset is not in the authorized folder group")]
    ForeignAsset,
    #[error("asset was not found")]
    UnknownAsset,
    #[error("invalid derivative key")]
    InvalidDerivativeKey,
    #[error("derivative generation failed")]
    DerivativeFailed,
    #[error("one or more requested previews could not be generated")]
    DerivativeUnavailable,
    #[error("folder selection was superseded by a newer valid selection")]
    SelectionSuperseded,
    #[error("original copy preparation failed")]
    CopyPreparationFailed,
    #[error("copy destination is unavailable")]
    CopyDestinationUnavailable,
    #[error("copy destination no longer exists")]
    CopyDestinationMissing,
    #[error("copy destination must be outside every source library")]
    CopyDestinationIsSource,
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
        let scheduler = Arc::new(IndexScheduler::new(Default::default()));
        let coordinator = Arc::new(crate::derivative_coordinator::DerivativeCoordinator::new(
            scheduler.clone(),
        ));
        let state = Arc::new(Mutex::new(ServiceState {
            libraries,
            folder_status: HashMap::new(),
            folder_revision: 0,
            active_scan: None,
            protected_group: None,
            selection_epoch: u64::from(should_reconcile),
            published_wall_cache_warning: None,
            published_screen_cache_warning: None,
        }));
        let protected_groups = ProtectedGroups::default();
        let metadata_reader = ReaderAdapter::new(reader);
        let gallery = crate::GalleryEngine::from_shared_state(
            state.clone(),
            scheduler.clone(),
            metadata_reader.clone(),
            coordinator.clone(),
            cache_root.clone(),
            catalog_path.clone(),
        );
        let pick_gallery = gallery.for_photo_picks(protected_groups.clone());
        let service = Self {
            gallery,
            pick_gallery,
            state,
            scheduler,
            coordinator,
            derivative_runtime: None,
            updates,
            catalog_path,
            cache_root,
            cache_budget,
            protected_groups,
            derivative_driver: Arc::new(tokio::sync::Mutex::new(())),
            collection_driver: Arc::new(CollectionDriverControl::new()),
            gallery_scope_update: Arc::new(TokioMutex::new(())),
            #[cfg(test)]
            saved_activation_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(test)]
            derivative_binding_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            scan_admission_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_task_tracker: Arc::new(DerivativeTaskTracker::new()),
            #[cfg(any(test, debug_assertions))]
            collection_driver_tracker: Arc::new(DerivativeTaskTracker::new()),
            #[cfg(debug_assertions)]
            collection_driver_exit_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            collection_publish_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            collection_enqueue_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            screen_preview_commit_test_counter: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            screen_preview_encode_test_counter: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            screen_preview_post_encode_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            screen_preview_post_admission_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_completion_test_hook: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_attempt_abort_handle: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_panic_after_admission_test_hook: Arc::new(TokioMutex::new(false)),
            #[cfg(any(test, debug_assertions))]
            managed_commit_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            derivative_panic_in_blocking_commit_test_hook: Arc::new(
                std::sync::atomic::AtomicBool::new(false),
            ),
            #[cfg(debug_assertions)]
            derivative_request_test_hook: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            derivative_visible_queue_test_hook: Arc::new(TokioMutex::new(None)),
            #[cfg(test)]
            scan_completion_wake_test_hook: Arc::new(Mutex::new(None)),
            desktop_bridges: Arc::new(Mutex::new(HashMap::new())),
            next_bridge_id: Arc::new(AtomicU64::new(0)),
            source_validator,
            selection_transition: Arc::new(Mutex::new(())),
            folder_access: crate::FolderAccessCoordinator::default(),
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
            let verification = Self::restore_verification_locked(&*service.state()?)?;
            service.spawn_restore_verification(verification);
        }
        Ok(service)
    }

    pub(crate) fn state(&self) -> Result<std::sync::MutexGuard<'_, ServiceState>, AppServiceError> {
        self.state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)
    }

    pub(crate) fn bootstrap_locked(
        state: &ServiceState,
    ) -> Result<BootstrapState, AppServiceError> {
        let stored = state.libraries.catalog().load_app_state()?;
        let saved_folders = Self::saved_folders_locked(state)?;
        let mut active_source = match stored.active_selection {
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
                    let group = state
                        .libraries
                        .catalog()
                        .folder_group_for_path(selection.library_id, &selection.relative_folder)?;
                    let selection_id = group
                        .map(|group_id| {
                            SelectionToken {
                                library_id: selection.library_id,
                                group_id,
                                epoch: state.selection_epoch,
                            }
                            .selection_id()
                        })
                        .unwrap_or_else(|| {
                            fallback_selection_id(
                                selection.library_id,
                                &selection.relative_folder,
                                state.selection_epoch,
                            )
                        });
                    Some(SourceSummary {
                        id: library.id.as_uuid().hyphenated().to_string(),
                        selection_id,
                        display_name,
                        availability: map_availability(library.availability),
                    })
                }
                None => None,
            },
            None => None,
        };
        if let Some(active) = &mut active_source
            && let Some(entry) = saved_folders
                .entries
                .iter()
                .find(|e| Some(&e.id) == saved_folders.active_entry_id.as_ref())
        {
            active.display_name = entry
                .custom_label
                .clone()
                .unwrap_or_else(|| entry.name.clone());
            if let Some(access) = saved_folders.access.get(&entry.folder_id) {
                active.availability = match access.state {
                    crate::FolderAccessState::Available => SourceAvailability::Available,
                    crate::FolderAccessState::Missing => SourceAvailability::Missing,
                    crate::FolderAccessState::Unreadable => SourceAvailability::Unreadable,
                    crate::FolderAccessState::RootOffline => SourceAvailability::RootOffline,
                    _ => active.availability,
                };
            }
        }
        Ok(BootstrapState {
            saved_folders,
            settings: SettingsState {
                appearance: stored.appearance,
                gallery_scope: stored.gallery_scope,
            },
            active_source,
        })
    }

    pub fn bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        let state = self.state()?;
        Self::bootstrap_locked(&state)
    }

    #[doc(hidden)]
    pub fn runtime_count_for_test(&self) -> usize {
        self.gallery.runtime_count_for_test()
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
        let _transition = self
            .selection_transition
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
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
        if let Err(error) = state
            .libraries
            .catalog_mut()
            .save_and_activate_folder(group)
        {
            if previous != Some(group) {
                let _ = self.protected_groups.unprotect(group);
            }
            return Err(error.into());
        }
        state.folder_revision += 1;
        state.folder_status.insert(
            group.as_uuid().to_string(),
            crate::FolderAccess {
                folder_id: group.as_uuid().to_string(),
                state: crate::FolderAccessState::Available,
                generation: 0,
                retry_after_ms: 0,
            },
        );
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
        self.gallery
            .folder_jobs
            .set_foreground(Some((token.library_id, token.group_id)));
        Ok((bootstrap, token))
    }

    pub fn subscribe_wall_updates(&self) -> tokio::sync::broadcast::Receiver<crate::WallUpdate> {
        self.updates.subscribe()
    }

    pub fn active_selection_id(&self) -> Option<String> {
        let state = self.state.lock().ok()?;
        let selection = state
            .libraries
            .catalog()
            .load_app_state()
            .ok()?
            .active_selection?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)
            .ok()?;
        Some(match group {
            Some(group_id) => SelectionToken {
                library_id: selection.library_id,
                group_id,
                epoch: state.selection_epoch,
            }
            .selection_id(),
            None => fallback_selection_id(
                selection.library_id,
                &selection.relative_folder,
                state.selection_epoch,
            ),
        })
    }

    /// Read a generated derivative from the managed cache after revalidating its active wall
    /// identity. This method deliberately returns bytes, never a native cache path.
    pub fn read_derivative(
        &self,
        asset_id: &str,
        class: DerivativeClass,
        key: &str,
    ) -> Result<Vec<u8>, AppServiceError> {
        if key.is_empty() || key.len() > 256 || !key.is_ascii() {
            return Err(AppServiceError::InvalidDerivativeKey);
        }
        let asset_id = uuid::Uuid::parse_str(asset_id)
            .map(photo_domain::AssetId::from_uuid)
            .map_err(|_| AppServiceError::InvalidAssetId)?;
        let kind = match class {
            DerivativeClass::WallThumbnail => "wall_thumbnail",
            DerivativeClass::ScreenPreview => "screen_preview",
        };
        let state = self.state()?;
        let selection = state
            .libraries
            .catalog()
            .load_app_state()?
            .active_selection
            .ok_or(AppServiceError::UnknownAsset)?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let asset = state
            .libraries
            .catalog()
            .find_asset(asset_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if asset.library_id != selection.library_id
            || !state.libraries.catalog().asset_is_member(group, asset_id)?
        {
            return Err(AppServiceError::ForeignAsset);
        }
        let record = state
            .libraries
            .catalog()
            .find_derivative(asset_id, kind, key)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if !state
            .libraries
            .catalog()
            .derivative_is_linked_to_group(record.id, group)?
        {
            return Err(AppServiceError::ForeignAsset);
        }
        let relative_path = record.relative_cache_path.clone();
        drop(state);
        CacheWriter::new(&self.cache_root)?
            .read_checked(&relative_path)
            .map_err(AppServiceError::from)
    }

    pub async fn query_wall(
        &self,
        request: crate::WallQueryRequest,
    ) -> Result<crate::WallPage, AppServiceError> {
        let (selection, scope) = {
            let state = self.state()?;
            let stored = state.libraries.catalog().load_app_state()?;
            let Some(selection) = stored.active_selection else {
                return Ok(empty_page());
            };
            let group = state
                .libraries
                .catalog()
                .folder_group_for_path(selection.library_id, &selection.relative_folder)?;
            let Some(group) = group else {
                return Ok(empty_page());
            };
            (
                crate::GallerySelection {
                    id: SelectionToken {
                        library_id: selection.library_id,
                        group_id: group,
                        epoch: state.selection_epoch,
                    }
                    .selection_id(),
                    library_id: selection.library_id,
                    group_id: group,
                    relative_folder: selection.relative_folder,
                    epoch: state.selection_epoch,
                },
                stored.gallery_scope,
            )
        };
        self.gallery.query_wall(&selection, scope, request).await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn coordinator_test_snapshot(&self) -> crate::CoordinatorTestSnapshot {
        self.derivative_context()
            .unwrap_or_else(|_| self.clone())
            .coordinator
            .test_snapshot()
            .await
    }

    pub fn update_appearance(
        &self,
        appearance: Appearance,
    ) -> Result<BootstrapState, AppServiceError> {
        let mut state = self.state()?;
        state.libraries.catalog_mut().set_appearance(appearance)?;
        Self::bootstrap_locked(&state)
    }

    pub async fn update_gallery_scope(
        &self,
        scope: GalleryScope,
    ) -> Result<BootstrapState, AppServiceError> {
        let _scope_update = self.gallery_scope_update.lock().await;
        let (changed, has_selection) = {
            let mut state = self.state()?;
            let stored = state.libraries.catalog().load_app_state()?;
            let changed = stored.gallery_scope != scope;
            if changed {
                state.libraries.catalog_mut().set_gallery_scope(scope)?;
            }
            (changed, stored.active_selection.is_some())
        };
        if changed && has_selection {
            let bound = self.derivative_context()?;
            let selection = self.active_selection_token()?;
            if let Ok(selection_value) = self.gallery.selection_from_token(selection) {
                let _ = self
                    .gallery
                    .update_client_interaction_desktop(
                        &selection_value,
                        &format!("desktop-{}", selection_value.id()),
                        scope,
                        InteractionState::Active,
                    )
                    .await?;
            }
            let runtime_selection = bound.active_selection_token()?;
            if let Some(runtime) = &bound.derivative_runtime {
                *runtime
                    .desktop_scope
                    .lock()
                    .map_err(|_| AppServiceError::StatePoisoned)? = scope;
            }
            bound.coordinator.ensure_selection(runtime_selection).await;
            bound.coordinator.invalidate_background().await;
            bound
                .coordinator
                .reset_collection_for_scope(runtime_selection)
                .await;
            bound.start_collection_driver(true, runtime_selection);
        }
        let state = self.state()?;
        Self::bootstrap_locked(&state)
    }

    /// Retires desktop wall and Picks folder jobs without waiting for
    /// already-started blocking source reads or admitted commits. Those owners
    /// retain their drain rules.
    pub async fn shutdown(&self) {
        let (runtimes, pick_runtimes) = {
            let _transition = self
                .selection_transition
                .lock()
                .expect("selection transition poisoned");
            let runtimes = self.gallery.folder_jobs.shutdown();
            let pick_runtimes = self.pick_gallery.folder_jobs.shutdown();
            if let Ok(mut state) = self.state() {
                state.active_scan = None;
            }
            for (_, bridge) in self
                .desktop_bridges
                .lock()
                .expect("desktop bridges poisoned")
                .drain()
            {
                bridge.abort.abort();
            }
            (runtimes, pick_runtimes)
        };
        for runtime in runtimes.into_iter().chain(pick_runtimes) {
            runtime.coordinator.close_folder().await;
        }
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
        source_warnings: Vec::new(),
        total_count: 0,
        preview_counts: crate::WallPreviewCounts::default(),
    }
}

pub(crate) fn map_availability(availability: Availability) -> SourceAvailability {
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
        Some(map_asset_warning_code(
            item.warning_code.as_deref().unwrap_or(""),
        ))
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
        rating: item.rating,
    }
}

fn map_asset_warning_code(code: &str) -> crate::WallWarningState {
    let (public_code, retryable) = match code {
        "derivative_generation_failed" => ("derivativeUnavailable", true),
        "derivative_generation_terminal" => ("derivativeUnavailable", false),
        _ => ("assetWarning", true),
    };
    crate::WallWarningState {
        code: public_code.to_owned(),
        retryable,
    }
}

pub(crate) fn map_source_warning_code(code: &str) -> crate::WallWarningState {
    let public_code = match code {
        "wall_thumbnail_cache_unavailable" => "wallThumbnailCacheUnavailable",
        "screen_preview_cache_unavailable" => "screenPreviewCacheUnavailable",
        _ => "sourceWarning",
    };
    crate::WallWarningState {
        code: public_code.to_owned(),
        retryable: true,
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

    #[cfg(debug_assertions)]
    use super::DerivativeTaskTracker;
    use super::{AppConfig, AppService, AppServiceError, RecentSourceValidator};
    use crate::{WallUpdate, WallWarningState};

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derivative_task_tracker_broadcasts_quiescence_to_all_waiters() {
        let tracker = Arc::new(DerivativeTaskTracker::new());
        let guard = tracker.start();
        let first_tracker = tracker.clone();
        let first = tokio::spawn(async move {
            first_tracker.wait_for_zero().await;
        });
        let second_tracker = tracker.clone();
        let second = tokio::spawn(async move {
            second_tracker.wait_for_zero().await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while tracker.waiting_count() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both quiescence waiters should start");

        drop(guard);
        tokio::time::timeout(Duration::from_secs(1), async {
            first.await.unwrap();
            second.await.unwrap();
        })
        .await
        .expect("all concurrent quiescence waiters should observe zero");
        assert_eq!(tracker.waiting_count(), 0);
    }

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_scan_token_is_rejected_before_bridge_or_scan_admission() {
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
        let service = AppService::open_with_reader(
            AppConfig::new(temp.path().join("data"), temp.path().join("cache")),
            Arc::new(BlockingReader(reader_gate.clone())),
        )
        .unwrap();
        let first_token = service.select_recent(&first).unwrap().1;
        service.select_recent(&second).unwrap();
        let mut updates = service.subscribe_wall_updates();

        let start_result = tokio::time::timeout(
            Duration::from_secs(1),
            service.start_selected_scan(first_token),
        )
        .await;

        let (released, changed) = &*reader_gate;
        *released.lock().unwrap() = true;
        changed.notify_all();

        start_result
            .expect("stale scan admission must not block")
            .unwrap();
        assert!(
            service.state().unwrap().active_scan.is_none(),
            "a stale scan token must not install the desktop active marker"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), updates.recv())
                .await
                .is_err(),
            "a stale scan token must not install a bridge or forward updates"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn returning_live_folder_scope_invalidates_only_its_old_collection_generation() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(a.join("photo.jpg"))
            .unwrap();
        let reader_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let service = AppService::open_with_reader(
            AppConfig::new(temp.path().join("data"), temp.path().join("cache")),
            Arc::new(BlockingReader(reader_gate.clone())),
        )
        .unwrap();
        service
            .update_gallery_scope(crate::GalleryScope::CurrentFolder)
            .await
            .unwrap();
        let a_bootstrap = service.start_scan(&a).await.unwrap();
        let bound_a = service.derivative_context().unwrap();
        let a_generation = bound_a.coordinator.background_generation().await;
        service.start_scan(&b).await.unwrap();
        service
            .update_gallery_scope(crate::GalleryScope::IncludeSubfolders)
            .await
            .unwrap();
        let bound_b = service.derivative_context().unwrap();
        let b_generation = bound_b.coordinator.background_generation().await;
        service
            .activate_saved_folder(a_bootstrap.saved_folders.active_entry_id.as_ref().unwrap())
            .await
            .unwrap();
        let returned = service.derivative_context().unwrap();
        let changed_generation = returned.coordinator.background_generation().await;
        let (released, changed) = &*reader_gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
        assert!(Arc::ptr_eq(&returned.coordinator, &bound_a.coordinator));
        assert!(
            changed_generation > a_generation,
            "returning live A with a new scope must invalidate its old collection generation"
        );
        assert_eq!(
            bound_b.coordinator.background_generation().await,
            b_generation
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_race_derivative_binding_rejects_without_stranding_receivers() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir(&source).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        service
            .set_interaction(crate::InteractionState::Active)
            .await;
        let mut updates = service.subscribe_wall_updates();
        service.start_scan(&source).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !matches!(
                updates.recv().await.unwrap(),
                WallUpdate::MetadataSettled { .. }
            ) {}
        })
        .await
        .unwrap();
        let asset = service
            .query_wall(crate::WallQueryRequest::oldest_first())
            .await
            .unwrap()
            .items[0]
            .id
            .clone();
        let (entered_send, entered_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        *service.derivative_binding_test_gate.lock().unwrap() = Some(super::BlockingTestGate {
            entered: entered_send,
            release: release_receive,
        });
        let request_service = service.clone();
        let request = tokio::spawn(async move {
            request_service
                .request_derivatives(crate::DerivativeRequest::visible(vec![asset]))
                .await
        });
        entered_receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        service.shutdown().await;
        release_send.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .expect(
                "shutdown racing binding must resolve the request, not strand its queued receiver",
            )
            .unwrap();
        assert!(result.is_err());
        assert_eq!(service.runtime_count_for_test(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_race_scan_activation_cannot_install_a_late_bridge() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir(&source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let token = service.select_recent(&source).unwrap().1;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *service.scan_admission_test_gate.lock().await = Some(super::CollectionTestGate {
            entered: entered.clone(),
            release: release.clone(),
        });
        let scan_service = service.clone();
        let scan = tokio::spawn(async move { scan_service.start_selected_scan(token).await });
        entered.notified().await;
        service.shutdown().await;
        let bridge_id = service
            .next_bridge_id
            .load(std::sync::atomic::Ordering::Acquire);
        release.notify_waiters();
        scan.await.unwrap().unwrap();
        assert_eq!(
            service
                .next_bridge_id
                .load(std::sync::atomic::Ordering::Acquire),
            bridge_id,
            "shutdown must prevent even a transient bridge installation"
        );
        assert!(
            service.desktop_bridges.lock().unwrap().is_empty(),
            "shutdown between transition sections must prevent bridge installation"
        );
        assert!(service.state().unwrap().active_scan.is_none());
        assert_eq!(service.runtime_count_for_test(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn service_shutdown_retires_all_folder_jobs_and_rejects_new_work() {
        let temp = tempfile::tempdir().unwrap();
        let reader_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let service = AppService::open_with_reader(
            AppConfig::new(temp.path().join("data"), temp.path().join("cache")),
            Arc::new(BlockingReader(reader_gate.clone())),
        )
        .unwrap();
        let mut runtimes = Vec::new();
        let mut waiters = Vec::new();
        let mut last_folder = None;
        for name in ["a", "b"] {
            let folder = temp.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
                .save(folder.join("photo.jpg"))
                .unwrap();
            service.start_scan(&folder).await.unwrap();
            let token = service.active_selection_token().unwrap();
            let bound = service
                .gallery
                .bind_desktop_derivatives(&service, token)
                .unwrap();
            let runtime = bound.derivative_runtime.unwrap();
            assert_eq!(
                *runtime.scan_lifecycle.borrow(),
                crate::hosted_runtime::ScanLifecycle::Running
            );
            waiters.push(
                runtime
                    .coordinator
                    .enqueue(
                        crate::derivative_coordinator::WorkKey {
                            selection: runtime.selection.token(),
                            asset_id: photo_domain::AssetId::from_uuid(uuid::Uuid::new_v4()),
                            class: crate::DerivativeClass::WallThumbnail,
                            cache_key: format!("shutdown-{name}"),
                            availability: photo_domain::Availability::Available,
                            scope: crate::GalleryScope::IncludeSubfolders,
                        },
                        crate::derivative_coordinator::WorkLane::VisibleWall,
                    )
                    .await,
            );
            runtimes.push(runtime);
            last_folder = Some(folder);
        }
        service.shutdown().await;
        service.shutdown().await;
        for runtime in &runtimes {
            assert!(runtime.cancellation_requested());
            assert_eq!(
                *runtime.scan_lifecycle.borrow(),
                crate::hosted_runtime::ScanLifecycle::Cancelled
            );
            assert_eq!(runtime.coordinator.pending_job_count().await, 0);
        }
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap(), None);
        }
        assert_eq!(service.runtime_count_for_test(), 0);
        assert!(service.state().unwrap().active_scan.is_none());
        assert!(service.desktop_bridges.lock().unwrap().is_empty());
        service
            .start_scan(last_folder.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(
            service.runtime_count_for_test(),
            0,
            "shutdown must not admit a successor"
        );
        let (released, changed) = &*reader_gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn removed_background_folder_bridge_observes_terminal_lifecycle_and_drops_runtime() {
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
        let service = AppService::open_with_reader(
            AppConfig::new(temp.path().join("data"), temp.path().join("cache")),
            Arc::new(BlockingReader(reader_gate.clone())),
        )
        .unwrap();
        let first_token = service.select_recent(&first).unwrap().1;
        let first_selection = service.gallery.selection_from_token(first_token).unwrap();
        let mut lifecycle = {
            service.start_selected_scan(first_token).await.unwrap();
            service
                .gallery
                .runtime_scan_state(&first_selection)
                .expect("admitted scan should retain its runtime")
        };
        assert_eq!(
            *lifecycle.borrow(),
            crate::hosted_runtime::ScanLifecycle::Running
        );

        let mut updates = service.subscribe_wall_updates();
        let entry = service
            .bootstrap()
            .unwrap()
            .saved_folders
            .active_entry_id
            .unwrap();
        service.select_recent(&second).unwrap();
        service.remove_saved_folder(&entry).unwrap();
        let (released, changed) = &*reader_gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
        if *lifecycle.borrow() != crate::hosted_runtime::ScanLifecycle::Cancelled {
            tokio::time::timeout(Duration::from_secs(1), lifecycle.changed())
                .await
                .expect("cancellation did not wake the bridge lifecycle watcher")
                .expect("bridge lifecycle sender closed before cancellation");
        }
        assert_eq!(
            *lifecycle.borrow(),
            crate::hosted_runtime::ScanLifecycle::Cancelled
        );
        assert!(service.state().unwrap().active_scan.is_none());
        tokio::time::timeout(Duration::from_secs(1), async {
            while service.runtime_count_for_test() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled bridge did not release its runtime");

        while updates.try_recv().is_ok() {}
        assert!(
            tokio::time::timeout(Duration::from_millis(150), updates.recv())
                .await
                .is_err(),
            "cancelled desktop bridge forwarded a stale update"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_startup_bridge_exits_and_same_token_retry_has_one_bridge() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let token = service.select_recent(&source).unwrap().1;
        service.gallery.fail_next_start_for_test();
        let mut updates = service.subscribe_wall_updates();

        assert!(service.start_selected_scan(token).await.is_err());
        assert_eq!(
            recv_terminal_warning(&mut updates, token).await,
            terminal_warning(token)
        );
        wait_for_desktop_cleanup(&service).await;
        assert_eq!(count_pending_warnings(&mut updates), 0);

        service.start_selected_scan(token).await.unwrap();
        let (warning_count, settled_count) =
            wait_for_retry_without_warnings(&service, &mut updates).await;
        assert_eq!(warning_count, 0, "retry replayed the old terminal warning");
        assert_eq!(settled_count, 1, "retry left a duplicate desktop bridge");
        wait_for_desktop_cleanup(&service).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_source_check_bridge_exits_before_retry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let token = service.select_recent(&source).unwrap().1;
        service.gallery.fail_next_source_check_for_test();
        let mut updates = service.subscribe_wall_updates();

        assert!(service.start_selected_scan(token).await.is_err());
        assert_eq!(
            recv_terminal_warning(&mut updates, token).await,
            terminal_warning(token)
        );
        wait_for_desktop_cleanup(&service).await;
        assert_eq!(count_pending_warnings(&mut updates), 0);

        service.start_selected_scan(token).await.unwrap();
        let (warning_count, settled_count) =
            wait_for_retry_without_warnings(&service, &mut updates).await;
        assert_eq!(warning_count, 0, "retry replayed the old terminal warning");
        assert_eq!(settled_count, 1, "retry left a duplicate desktop bridge");
        wait_for_desktop_cleanup(&service).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_persistence_bridge_forwards_one_terminal_warning_and_retry_is_clean() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let token = service.select_recent(&source).unwrap().1;
        service.gallery.fail_next_batch_for_test();
        let mut updates = service.subscribe_wall_updates();

        service.start_selected_scan(token).await.unwrap();
        assert_eq!(
            recv_terminal_warning(&mut updates, token).await,
            terminal_warning(token)
        );
        wait_for_desktop_cleanup(&service).await;
        assert_eq!(count_pending_warnings(&mut updates), 0);

        service.start_selected_scan(token).await.unwrap();
        let (warning_count, settled_count) =
            wait_for_retry_without_warnings(&service, &mut updates).await;
        assert_eq!(
            warning_count, 0,
            "terminal persistence warning was duplicated"
        );
        assert_eq!(settled_count, 1, "retry left a duplicate desktop bridge");
        wait_for_desktop_cleanup(&service).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_join_bridge_forwards_one_terminal_warning_and_retry_is_clean() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(2, 2, Rgb([10_u8, 20, 30]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let token = service.select_recent(&source).unwrap().1;
        service.gallery.fail_next_join_for_test();
        let mut updates = service.subscribe_wall_updates();

        service.start_selected_scan(token).await.unwrap();
        assert_eq!(
            recv_terminal_warning(&mut updates, token).await,
            terminal_warning(token)
        );
        wait_for_desktop_cleanup(&service).await;
        assert_eq!(count_pending_warnings(&mut updates), 0);

        service.start_selected_scan(token).await.unwrap();
        let (warning_count, settled_count) =
            wait_for_retry_without_warnings(&service, &mut updates).await;
        assert_eq!(warning_count, 0, "retry replayed the failed join warning");
        assert_eq!(settled_count, 1, "retry left a duplicate desktop bridge");
        wait_for_desktop_cleanup(&service).await;
    }

    fn terminal_warning(selection: super::SelectionToken) -> WallUpdate {
        WallUpdate::Warning {
            selection_id: selection.selection_id(),
            source_id: selection.library_id.as_uuid().hyphenated().to_string(),
            asset_id: None,
            warning: WallWarningState {
                code: "catalogUnavailable".to_owned(),
                retryable: true,
            },
        }
    }

    async fn recv_terminal_warning(
        updates: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
        selection: super::SelectionToken,
    ) -> WallUpdate {
        let update = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let update = updates.recv().await.expect("desktop bridge closed early");
                if matches!(update, WallUpdate::Warning { asset_id: None, .. }) {
                    return update;
                }
            }
        })
        .await
        .expect("failed scan did not forward its terminal warning");
        assert_eq!(update, terminal_warning(selection));
        update
    }

    fn count_pending_warnings(updates: &mut tokio::sync::broadcast::Receiver<WallUpdate>) -> usize {
        let mut count = 0;
        while let Ok(update) = updates.try_recv() {
            if matches!(update, WallUpdate::Warning { .. }) {
                count += 1;
            }
        }
        count
    }

    async fn wait_for_desktop_cleanup(service: &AppService) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if service.desktop_bridge_count_for_test() == 0
                    && service.runtime_count_for_test() == 0
                    && service.state().unwrap().active_scan.is_none()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("desktop failure state did not clean up");
    }

    async fn wait_for_retry_without_warnings(
        service: &AppService,
        updates: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
    ) -> (usize, usize) {
        let mut warning_count = 0;
        let mut settled_count = 0;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                while let Ok(update) = updates.try_recv() {
                    match update {
                        WallUpdate::Warning { .. } => warning_count += 1,
                        WallUpdate::MetadataSettled { .. } => settled_count += 1,
                        _ => {}
                    }
                }
                if service.desktop_bridge_count_for_test() == 0 && settled_count > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retry did not settle and release its bridge");
        (warning_count, settled_count)
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
