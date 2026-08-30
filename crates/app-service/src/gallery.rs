use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{Mutex as TokioMutex, Notify, OwnedSemaphorePermit, Semaphore};

use photo_cache::{
    CacheBudget, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
    ImageDerivativeGenerator,
};
use photo_catalog::{Catalog, CatalogError, NewDerivative, NewFolderGroup, WallOrder};
use photo_core::{LibraryService, RealSourceFs};
use photo_domain::{FolderGroupId, GalleryScope, LibraryId, RelativePathKey};
use photo_indexer::{DefaultMetadataReader, IndexEvent, Indexer, MetadataReader, ScanRequest};

use crate::derivative_coordinator::{WorkKey, WorkLane, WorkTicket};
use crate::hosted_runtime::{SelectionEventSubscription, SelectionRuntime, SubscriptionOrigin};
use crate::service::{ReaderAdapter, ServiceState};
use crate::{
    AppConfig, AppServiceError, DerivativeClass, DerivativePriority, DerivativeReference,
    DerivativeRequest, InteractionState, OrderState, SourceAvailability, WallPage,
    WallQueryRequest, WallUpdate,
};

fn canonicalize_for_identity(path: &Path) -> PathBuf {
    let mut unresolved = Vec::new();
    let mut existing = path.to_owned();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        unresolved.push(name.to_owned());
        if !existing.pop() {
            break;
        }
    }
    let mut canonical = std::fs::canonicalize(&existing).unwrap_or(existing);
    for component in unresolved.iter().rev() {
        canonical.push(component);
    }
    canonical
}

const DERIVATIVE_DECODER_VERSION: &str = "image-0.25-v1";

fn derivative_spec_for_gallery(
    asset: &photo_catalog::AssetRecord,
    class: DerivativeClass,
) -> DerivativeSpec {
    let (kind, edge) = match class {
        DerivativeClass::WallThumbnail => (DerivativeKind::WallThumbnail, 1024),
        DerivativeClass::ScreenPreview => (DerivativeKind::ScreenPreview, 4096),
    };
    DerivativeSpec {
        asset_id: asset.id,
        signature: asset.signature,
        orientation: asset.orientation.unwrap_or(1),
        kind,
        decoder_version: DERIVATIVE_DECODER_VERSION.to_owned(),
        colour_space: "srgb".to_owned(),
        target: DerivativeTarget::LongEdge(edge),
    }
}

fn derivative_kind_name_for_gallery(class: DerivativeClass) -> &'static str {
    match class {
        DerivativeClass::WallThumbnail => "wall_thumbnail",
        DerivativeClass::ScreenPreview => "screen_preview",
    }
}

fn derivative_record_is_valid(root: &Path, record: &photo_catalog::DerivativeRecord) -> bool {
    let Ok(writer) = CacheWriter::new(root) else {
        return false;
    };
    let Ok(mut file) = writer.open_checked(&record.relative_cache_path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if metadata.len() != record.size_bytes {
        return false;
    }
    let mut bytes = Vec::new();
    if std::io::Read::read_to_end(&mut file, &mut bytes).is_err()
        || u64::try_from(bytes.len()).ok() != Some(record.size_bytes)
        || bytes.get(..3) != Some(&[0xff, 0xd8, 0xff])
    {
        return false;
    }
    image::load_from_memory(&bytes).is_ok()
}

struct ProtectedGroupGuard {
    groups: photo_cache::ProtectedGroups,
    group: FolderGroupId,
}

struct HostedDriverOwner {
    coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    generation: u64,
    engine: GalleryEngine,
    runtime: Arc<SelectionRuntime>,
    active_ticket: Arc<Mutex<Option<WorkTicket>>>,
}

impl Drop for HostedDriverOwner {
    fn drop(&mut self) {
        let coordinator = self.coordinator.clone();
        let generation = self.generation;
        let engine = self.engine.clone();
        let runtime = self.runtime.clone();
        let active_ticket = self.active_ticket.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let ticket = active_ticket
                    .lock()
                    .expect("hosted active ticket poisoned")
                    .take();
                if let Some(ticket) = ticket {
                    coordinator.abort_attempt_as_failure(ticket).await;
                }
                coordinator.release_driver_owner(generation).await;
                if coordinator.pending_job_count().await > 0 {
                    engine.start_hosted_derivative_driver(runtime).await;
                }
            });
        }
    }
}

struct HostedAdmission {
    permit: Option<OwnedSemaphorePermit>,
    active: Arc<AtomicUsize>,
    wake: Arc<Notify>,
}

impl Drop for HostedAdmission {
    fn drop(&mut self) {
        self.permit.take();
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.wake.notify_waiters();
    }
}

enum HostedDerivativePrepared {
    Wall(photo_cache::EncodedScreenPreview),
    Screen(photo_cache::EncodedScreenPreview),
}

impl ProtectedGroupGuard {
    fn new(
        groups: photo_cache::ProtectedGroups,
        group: FolderGroupId,
    ) -> Result<Self, AppServiceError> {
        groups.protect(group)?;
        Ok(Self { groups, group })
    }
}

impl Drop for ProtectedGroupGuard {
    fn drop(&mut self) {
        let _ = self.groups.unprotect(self.group);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GallerySelection {
    pub(crate) id: String,
    pub(crate) library_id: LibraryId,
    pub(crate) group_id: FolderGroupId,
    pub(crate) relative_folder: RelativePathKey,
    pub(crate) epoch: u64,
}

impl GallerySelection {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn library_id(&self) -> LibraryId {
        self.library_id
    }
    pub fn group_id(&self) -> FolderGroupId {
        self.group_id
    }
    pub fn relative_folder(&self) -> &RelativePathKey {
        &self.relative_folder
    }
    pub(crate) fn token(&self) -> crate::service::SelectionToken {
        crate::service::SelectionToken {
            library_id: self.library_id,
            group_id: self.group_id,
            epoch: self.epoch,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSummary {
    pub id: String,
    pub source_id: String,
    pub display_name: String,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub availability: SourceAvailability,
}

pub struct ManagedDerivative {
    pub file: std::fs::File,
    pub content_type: &'static str,
    pub content_length: u64,
    pub etag: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBreadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Clone)]
pub struct GalleryEngine {
    pub(crate) state: Arc<Mutex<ServiceState>>,
    pub(crate) scheduler: Arc<photo_indexer::IndexScheduler>,
    pub(crate) runtimes: Arc<Mutex<HashMap<FolderGroupId, Weak<SelectionRuntime>>>>,
    pub(crate) metadata_reader: ReaderAdapter,
    hosted_library_id: Option<LibraryId>,
    pub(crate) shared_coordinator:
        Option<Arc<crate::derivative_coordinator::DerivativeCoordinator>>,
    pub(crate) cache_root: PathBuf,
    pub(crate) catalog_path: PathBuf,
    derivative_admission: Arc<Semaphore>,
    derivative_admission_wake: Arc<Notify>,
    hosted_active: Arc<AtomicUsize>,
    hosted_admission_owner: Arc<TokioMutex<()>>,
    hosted_publication_fence: Arc<TokioMutex<()>>,
    protected_groups: photo_cache::ProtectedGroups,
    #[cfg(debug_assertions)]
    hosted_attempts: Arc<AtomicUsize>,
    #[cfg(test)]
    fail_next_start: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_source_check: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_batch: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_join: Arc<AtomicBool>,
    #[cfg(debug_assertions)]
    hosted_commit_publication_test_gate: Arc<TokioMutex<Option<(Arc<Notify>, Arc<Notify>)>>>,
    #[cfg(debug_assertions)]
    hosted_commit_post_publication_test_gate: Arc<TokioMutex<Option<(Arc<Notify>, Arc<Notify>)>>>,
    #[cfg(debug_assertions)]
    hosted_driver_abort_handle: Arc<TokioMutex<Option<tokio::task::AbortHandle>>>,
    #[cfg(debug_assertions)]
    hosted_panic_before_admission: Arc<TokioMutex<bool>>,
}

impl GalleryEngine {
    pub fn open(config: AppConfig, source_root: PathBuf) -> Result<Self, AppServiceError> {
        Self::open_with_reader(config, source_root, Arc::new(DefaultMetadataReader))
    }

    pub fn open_with_reader(
        config: AppConfig,
        source_root: PathBuf,
        reader: Arc<dyn MetadataReader>,
    ) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let mut libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;
        let current_canonical = std::fs::canonicalize(&source_root).ok();
        let canonical = current_canonical
            .clone()
            .unwrap_or_else(|| canonicalize_for_identity(&source_root));
        let hosted_library_id = if let Some(library) = libraries
            .catalog()
            .list_libraries()?
            .into_iter()
            .find(|library| {
                library.canonical_root_key.to_path_buf().ok().as_deref()
                    == Some(canonical.as_path())
                    || current_canonical.is_none()
                        && Path::new(&library.display_path) == source_root.as_path()
            }) {
            library.id
        } else {
            if !canonical.is_dir() {
                return Err(AppServiceError::OpenRecent(
                    photo_core::AddLibraryError::NotDirectory(canonical),
                ));
            }
            libraries
                .add_configured(
                    &canonical,
                    source_root
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .as_deref()
                        .unwrap_or("Hosted photos"),
                )?
                .id
        };
        let cache_root = config.cache_dir().to_owned();
        let catalog_path = config.catalog_path();
        Ok(Self {
            state: Arc::new(Mutex::new(ServiceState {
                libraries,
                active_scan: None,
                protected_group: None,
                selection_epoch: 0,
                published_wall_cache_warning: None,
                published_screen_cache_warning: None,
            })),
            scheduler: Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            metadata_reader: ReaderAdapter::new(reader),
            hosted_library_id: Some(hosted_library_id),
            shared_coordinator: None,
            cache_root,
            catalog_path,
            derivative_admission: Arc::new(Semaphore::new(4)),
            derivative_admission_wake: Arc::new(Notify::new()),
            hosted_active: Arc::new(AtomicUsize::new(0)),
            hosted_admission_owner: Arc::new(TokioMutex::new(())),
            hosted_publication_fence: Arc::new(TokioMutex::new(())),
            protected_groups: photo_cache::ProtectedGroups::default(),
            #[cfg(debug_assertions)]
            hosted_attempts: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
            #[cfg(debug_assertions)]
            hosted_commit_publication_test_gate: Arc::new(TokioMutex::new(None)),
            hosted_commit_post_publication_test_gate: Arc::new(TokioMutex::new(None)),
            hosted_driver_abort_handle: Arc::new(TokioMutex::new(None)),
            hosted_panic_before_admission: Arc::new(TokioMutex::new(false)),
        })
    }

    pub(crate) fn from_shared_state(
        state: Arc<Mutex<ServiceState>>,
        scheduler: Arc<photo_indexer::IndexScheduler>,
        metadata_reader: ReaderAdapter,
        shared_coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
        cache_root: PathBuf,
        catalog_path: PathBuf,
    ) -> Self {
        Self {
            state,
            scheduler,
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            metadata_reader,
            hosted_library_id: None,
            shared_coordinator: Some(shared_coordinator),
            cache_root,
            catalog_path,
            derivative_admission: Arc::new(Semaphore::new(4)),
            derivative_admission_wake: Arc::new(Notify::new()),
            hosted_active: Arc::new(AtomicUsize::new(0)),
            hosted_admission_owner: Arc::new(TokioMutex::new(())),
            hosted_publication_fence: Arc::new(TokioMutex::new(())),
            protected_groups: photo_cache::ProtectedGroups::default(),
            #[cfg(debug_assertions)]
            hosted_attempts: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
            #[cfg(debug_assertions)]
            hosted_commit_publication_test_gate: Arc::new(TokioMutex::new(None)),
            hosted_commit_post_publication_test_gate: Arc::new(TokioMutex::new(None)),
            hosted_driver_abort_handle: Arc::new(TokioMutex::new(None)),
            hosted_panic_before_admission: Arc::new(TokioMutex::new(false)),
        }
    }

    pub async fn select_relative(
        &self,
        relative: &Path,
    ) -> Result<SelectionSummary, AppServiceError> {
        if relative.is_absolute()
            || relative.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(AppServiceError::OpenRecent(
                photo_core::AddLibraryError::InvalidSelection,
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let hosted_library_id = self
            .hosted_library_id
            .ok_or(AppServiceError::UnknownAsset)?;
        let library = state
            .libraries
            .catalog()
            .find_library(hosted_library_id)?
            .ok_or(AppServiceError::StatePoisoned)?;
        let root = library
            .canonical_root_key
            .to_path_buf()
            .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
        let selected_native = root.join(relative);
        let selected_canonical = std::fs::canonicalize(&selected_native).map_err(|_| {
            AppServiceError::OpenRecent(photo_core::AddLibraryError::NotDirectory(
                selected_native.clone(),
            ))
        })?;
        if !selected_canonical.starts_with(&root) || !selected_canonical.is_dir() {
            return Err(AppServiceError::OpenRecent(
                photo_core::AddLibraryError::InvalidSelection,
            ));
        }
        let canonical_relative = selected_canonical
            .strip_prefix(&root)
            .unwrap_or_else(|_| Path::new("."));
        let key = RelativePathKey::from_relative_path(canonical_relative)
            .map_err(|_| photo_core::AddLibraryError::InvalidSelection)?;
        let display_name = canonical_relative
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| library.display_name.clone());
        let group = state
            .libraries
            .catalog_mut()
            .upsert_folder_group(&NewFolderGroup {
                id: FolderGroupId::new(),
                library_id: hosted_library_id,
                relative_path: key,
                display_path: relative.to_string_lossy().into_owned(),
                last_viewed_at: Some(crate::service::unix_timestamp()),
            })?;
        let selection = GallerySelection {
            id: format!("selection-{}", group.as_uuid().hyphenated()),
            library_id: hosted_library_id,
            group_id: group,
            relative_folder: state
                .libraries
                .catalog()
                .folder_group(group)?
                .ok_or(AppServiceError::StatePoisoned)?
                .relative_path,
            epoch: 0,
        };
        drop(state);
        self.selection_summary(&selection).map(|mut summary| {
            summary.display_name = display_name;
            summary
        })
    }

    pub fn resolve_selection(&self, id: &str) -> Result<GallerySelection, AppServiceError> {
        let uuid = id
            .strip_prefix("selection-")
            .ok_or(AppServiceError::UnknownAsset)
            .and_then(|s| uuid::Uuid::parse_str(s).map_err(|_| AppServiceError::UnknownAsset))?;
        let canonical_id = format!("selection-{}", uuid.hyphenated());
        if id != canonical_id {
            return Err(AppServiceError::UnknownAsset);
        }
        let group = FolderGroupId::from_uuid(uuid);
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let record = state
            .libraries
            .catalog()
            .folder_group(group)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if self
            .hosted_library_id
            .is_some_and(|library_id| record.library_id != library_id)
        {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(GallerySelection {
            id: id.to_owned(),
            library_id: record.library_id,
            group_id: record.id,
            relative_folder: record.relative_path,
            epoch: 0,
        })
    }

    pub(crate) fn selection_from_token(
        &self,
        token: crate::service::SelectionToken,
    ) -> Result<GallerySelection, AppServiceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let record = state
            .libraries
            .catalog()
            .folder_group(token.group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if record.library_id != token.library_id {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(GallerySelection {
            id: token.selection_id(),
            library_id: record.library_id,
            group_id: record.id,
            relative_folder: record.relative_path,
            epoch: token.epoch,
        })
    }

    pub fn selection_summary(
        &self,
        selection: &GallerySelection,
    ) -> Result<SelectionSummary, AppServiceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let group = state
            .libraries
            .catalog()
            .folder_group(selection.group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if self
            .hosted_library_id
            .is_some_and(|library_id| group.library_id != library_id)
            || group.library_id != selection.library_id
            || group.relative_path != selection.relative_folder
        {
            return Err(AppServiceError::UnknownAsset);
        }
        let library = state
            .libraries
            .catalog()
            .find_library(group.library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let relative = group
            .relative_path
            .to_path_buf()
            .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
        let mut breadcrumbs = Vec::new();
        let mut path = PathBuf::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            path.push(name);
            breadcrumbs.push(FolderBreadcrumb {
                name: name.to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
            });
        }
        Ok(SelectionSummary {
            id: selection.id.clone(),
            source_id: group.library_id.as_uuid().hyphenated().to_string(),
            display_name: relative
                .file_name()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or(library.display_name),
            breadcrumbs,
            availability: crate::service::map_availability(library.availability),
        })
    }

    fn runtime(&self, selection: &GallerySelection) -> Arc<SelectionRuntime> {
        let mut runtimes = self.runtimes.lock().expect("runtime registry poisoned");
        runtimes.retain(|_, runtime| runtime.strong_count() > 0);
        if let Some(runtime) = runtimes.get(&selection.group_id).and_then(Weak::upgrade) {
            if runtime.selection == *selection {
                return runtime;
            }
            // The desktop adapter advances its epoch when the same folder is
            // selected again. A retained bridge may still keep the previous
            // runtime alive, but it must not leak that stale selection ID into
            // the new desktop stream.
            if self.shared_coordinator.is_some() {
                runtime.request_cancel();
                runtimes.remove(&selection.group_id);
            } else {
                return runtime;
            }
        }
        // A desktop `start_scan` is an explicit refresh even when the
        // previous generation is complete. Hosted `ensure_running` keeps its
        // restart-idempotent catalog settlement semantics at epoch zero.
        let settled = if self.shared_coordinator.is_some() && selection.epoch != 0 {
            false
        } else {
            self.state
                .lock()
                .ok()
                .and_then(|state| {
                    state
                        .libraries
                        .catalog()
                        .has_completed_generation_for_group(
                            selection.library_id,
                            selection.group_id,
                        )
                        .ok()
                })
                .unwrap_or(false)
        };
        let runtime = match &self.shared_coordinator {
            Some(coordinator) => SelectionRuntime::new_with_coordinator(
                selection.clone(),
                coordinator.clone(),
                settled,
            ),
            None => SelectionRuntime::new(selection.clone(), self.scheduler.clone(), settled),
        };
        runtimes.insert(selection.group_id, Arc::downgrade(&runtime));
        runtime
    }

    pub(crate) fn remove_runtime_if_dead(
        &self,
        group_id: FolderGroupId,
        runtime: &Arc<SelectionRuntime>,
    ) {
        let mut runtimes = self.runtimes.lock().expect("runtime registry poisoned");
        let remove = runtimes.get(&group_id).is_some_and(|weak| {
            weak.upgrade()
                .is_none_or(|current| Arc::ptr_eq(&current, runtime))
        });
        if remove && Arc::strong_count(runtime) <= 1 {
            runtimes.remove(&group_id);
        }
    }

    /// Requests cancellation of the admitted scan for a desktop selection.
    /// The runtime remains responsible for draining and persisting any events
    /// already in flight; this only replaces the old AppService-owned sender.
    pub(crate) fn cancel_runtime_scan(&self, group_id: FolderGroupId) {
        let runtime = self
            .runtimes
            .lock()
            .ok()
            .and_then(|runtimes| runtimes.get(&group_id).and_then(Weak::upgrade));
        let Some(runtime) = runtime else {
            return;
        };
        runtime.request_cancel();
    }

    pub async fn ensure_running(
        &self,
        selection: &GallerySelection,
    ) -> Result<(), AppServiceError> {
        let runtime = self.runtime(selection);
        runtime.begin_scan(self.clone()).await
    }

    pub async fn query_wall(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: WallQueryRequest,
    ) -> Result<WallPage, AppServiceError> {
        if !(1..=250).contains(&request.limit) {
            return Err(AppServiceError::InvalidLimit);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let settled = state
            .libraries
            .catalog()
            .has_completed_generation_for_group(selection.library_id, selection.group_id)?;
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
            .map(|value| {
                crate::wall::decode_cursor(value, request.direction, scope, selection, order)
            })
            .transpose()?;
        let page = state.libraries.catalog().wall_page_scoped(
            selection.group_id,
            scope,
            order,
            cursor,
            request.limit,
        )?;
        let items = crate::service::wall_assets_with_derivatives(
            state.libraries.catalog(),
            &page.items,
            if settled {
                OrderState::Settled
            } else {
                OrderState::Provisional
            },
        )?;
        let next_cursor = page
            .next
            .as_ref()
            .map(|key| crate::wall::encode_cursor(request.direction, scope, selection, key))
            .transpose()?;
        Ok(WallPage {
            items,
            next_cursor,
            order_state: if settled {
                OrderState::Settled
            } else {
                OrderState::Provisional
            },
            source_warnings: state
                .libraries
                .catalog()
                .source_warning_summaries(selection.library_id)?
                .into_iter()
                .map(|w| crate::service::map_source_warning_code(&w.code))
                .collect(),
        })
    }

    /// Generates only the derivatives explicitly requested by this selection.
    /// Membership is checked before any source file is opened. Work is then
    /// admitted through the selection runtime's coordinator, so duplicate
    /// callers share one queued attempt and inherit its cancellation and lane
    /// semantics.
    pub async fn request_derivatives(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        let assets = self.validate_derivative_request(selection, scope, &request)?;

        let runtime = self.runtime(selection);
        let _protected =
            ProtectedGroupGuard::new(self.protected_groups.clone(), selection.group_id)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
        let classes = if request.kind == DerivativeClass::ScreenPreview {
            vec![
                DerivativeClass::WallThumbnail,
                DerivativeClass::ScreenPreview,
            ]
        } else {
            vec![DerivativeClass::WallThumbnail]
        };
        for class in classes {
            let mut receivers = Vec::new();
            for asset in &assets {
                if asset.media_kind == photo_domain::MediaKind::Video {
                    continue;
                }
                let spec = derivative_spec_for_gallery(asset, class);
                let key = DerivativeKey::compute(&spec);
                let existing = async {
                    let _publication = self.hosted_publication_fence.lock().await;
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| AppServiceError::StatePoisoned)?;
                    let existing = state.libraries.catalog().find_derivative(
                        asset.id,
                        derivative_kind_name_for_gallery(class),
                        key.as_str(),
                    )?;
                    let result = if let Some(record) = existing.as_ref() {
                        if derivative_record_is_valid(&self.cache_root, record) {
                            state
                                .libraries
                                .catalog_mut()
                                .link_derivative_group(record.id, selection.group_id)?;
                            Some(record.cache_key.clone())
                        } else {
                            // Remove this requester's link first.  The cache
                            // bytes are immutable and may still be owned by
                            // another folder group; only remove the physical
                            // file after the catalog confirms that no link
                            // remains.
                            let record_id = record.id;
                            let relative_path = record.relative_cache_path.clone();
                            state
                                .libraries
                                .catalog_mut()
                                .remove_derivative_group_links(
                                    &[record_id],
                                    &[selection.group_id],
                                )?;
                            let still_linked = state
                                .libraries
                                .catalog()
                                .find_derivative(
                                    asset.id,
                                    derivative_kind_name_for_gallery(class),
                                    key.as_str(),
                                )?
                                .is_some();
                            if !still_linked {
                                CacheWriter::new(&self.cache_root)?
                                    .remove_checked(&relative_path)?;
                            }
                            None
                        }
                    } else {
                        None
                    };
                    Ok::<_, AppServiceError>(result)
                }
                .await
                .map_err(|_| AppServiceError::DerivativeFailed)?;
                if let Some(key) = existing {
                    runtime
                        .publish(WallUpdate::DerivativesReady {
                            selection_id: selection.id.clone(),
                            derivatives: vec![DerivativeReference {
                                asset_id: asset.id.as_uuid().hyphenated().to_string(),
                                kind: class,
                                key,
                            }],
                        })
                        .await;
                    continue;
                }
                let lane = match (class, request.priority) {
                    (DerivativeClass::WallThumbnail, DerivativePriority::Visible) => {
                        WorkLane::VisibleWall
                    }
                    (DerivativeClass::WallThumbnail, DerivativePriority::NearViewport) => {
                        WorkLane::NearWall
                    }
                    (DerivativeClass::ScreenPreview, DerivativePriority::Visible) => {
                        WorkLane::ViewerPreview
                    }
                    (DerivativeClass::ScreenPreview, DerivativePriority::NearViewport) => {
                        WorkLane::IdlePreview
                    }
                };
                let prerequisite = (class == DerivativeClass::ScreenPreview).then(|| {
                    DerivativeKey::compute(&derivative_spec_for_gallery(
                        asset,
                        DerivativeClass::WallThumbnail,
                    ))
                    .as_str()
                    .to_owned()
                });
                let work_key = WorkKey {
                    selection: selection.token(),
                    asset_id: asset.id,
                    class,
                    cache_key: key.as_str().to_owned(),
                    availability: asset.availability,
                    scope,
                };
                runtime.coordinator.invalidate_completed(&work_key).await;
                let receiver = runtime
                    .coordinator
                    .enqueue_hosted_with_prerequisite_observed(
                        work_key.clone(),
                        lane,
                        prerequisite,
                        None,
                        self.hosted_scope_authorizer(),
                    )
                    .await;
                receivers.push((receiver, work_key));
            }
            if !receivers.is_empty() {
                self.start_hosted_derivative_driver(runtime.clone()).await;
            }
            for (receiver, _work_key) in receivers {
                match receiver.await {
                    Ok(crate::derivative_coordinator::DerivativeResult::Ready(_)) => {}
                    Ok(crate::derivative_coordinator::DerivativeResult::Failed) => {
                        return Err(AppServiceError::DerivativeFailed);
                    }
                    Ok(crate::derivative_coordinator::DerivativeResult::Unavailable) | Err(_) => {
                        return Err(AppServiceError::DerivativeUnavailable);
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn hosted_derivative_attempts_for_test(&self) -> usize {
        self.hosted_attempts.load(Ordering::Acquire)
    }

    fn try_hosted_slot(&self) -> bool {
        let limit = self.scheduler.available_background_permits().clamp(1, 4);
        let mut active = self.hosted_active.load(Ordering::Acquire);
        loop {
            if active >= limit {
                return false;
            }
            match self.hosted_active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(next) => active = next,
            }
        }
    }

    fn hosted_scope_authorizer(&self) -> crate::derivative_coordinator::ScopeAuthorizer {
        let engine = self.clone();
        Arc::new(move |key, scope| engine.hosted_scope_is_current(key, scope))
    }

    fn hosted_scope_is_current(&self, key: &WorkKey, scope: GalleryScope) -> bool {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .libraries
                    .catalog()
                    .wall_records_for_assets_scoped(key.selection.group_id, scope, &[key.asset_id])
                    .ok()
            })
            .is_some_and(|records| !records.is_empty())
    }

    /// Reserves one hosted capacity slot and dequeues a job while the shared
    /// admission owner is held. This makes selection drivers contenders for
    /// one central admission decision instead of letting each driver consume
    /// capacity before discovering that another selection has better work.
    async fn try_admit_hosted_work(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Option<(WorkTicket, HostedAdmission)> {
        let _owner = self.hosted_admission_owner.lock().await;
        let permit = self.derivative_admission.clone().try_acquire_owned().ok()?;
        if !self.try_hosted_slot() {
            drop(permit);
            return None;
        }
        let admission = HostedAdmission {
            permit: Some(permit),
            active: self.hosted_active.clone(),
            wake: self.derivative_admission_wake.clone(),
        };
        let Some(ticket) = runtime.coordinator.next_work().await else {
            drop(admission);
            return None;
        };
        Some((ticket, admission))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_commit_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_commit_publication_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_commit_publication_test_gate(&self) {
        let gate = self.hosted_commit_publication_test_gate.lock().await.take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_commit_post_publication_test_gate(&self) {
        let gate = self
            .hosted_commit_post_publication_test_gate
            .lock()
            .await
            .take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    /// Validates derivative identifiers and selection scope without starting
    /// a scan or touching source media. HTTP callers use this boundary before
    /// admitting work for a selection.
    pub fn validate_derivative_request(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: &DerivativeRequest,
    ) -> Result<Vec<photo_catalog::AssetRecord>, AppServiceError> {
        if !(1..=250).contains(&request.asset_ids.len()) {
            return Err(AppServiceError::InvalidLimit);
        }
        let ids = request
            .asset_ids
            .iter()
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map(photo_domain::AssetId::from_uuid)
                    .map_err(|_| AppServiceError::InvalidAssetId)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let allowed = state.libraries.catalog().wall_records_for_assets_scoped(
            selection.group_id,
            scope,
            &ids,
        )?;
        if allowed.len() != ids.len() {
            return Err(AppServiceError::ForeignAsset);
        }
        ids.iter()
            .map(|id| {
                state
                    .libraries
                    .catalog()
                    .find_asset(*id)?
                    .ok_or(AppServiceError::UnknownAsset)
            })
            .collect::<Result<Vec<_>, AppServiceError>>()
    }

    async fn start_hosted_derivative_driver(&self, runtime: Arc<SelectionRuntime>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let Some(generation) = runtime.coordinator.claim_driver_owner().await else {
            return;
        };
        let engine = self.clone();
        let active_ticket = Arc::new(Mutex::new(None));
        let driver = tokio::spawn(async move {
            let _owner = HostedDriverOwner {
                coordinator: runtime.coordinator.clone(),
                generation,
                engine: engine.clone(),
                runtime: runtime.clone(),
                active_ticket: active_ticket.clone(),
            };
            loop {
                let coordinator = runtime.coordinator.clone();
                let observed_coordinator = coordinator.change_generation();
                let observed_scheduler = engine.scheduler.change_generation();
                let Some((ticket, admission)) = engine.try_admit_hosted_work(&runtime).await else {
                    if coordinator.pending_job_count().await == 0 {
                        if coordinator.release_driver_owner_if_idle(generation).await {
                            break;
                        }
                    }
                    let coordinator_wake = coordinator.wait_for_change_since(observed_coordinator);
                    let scheduler_wake = engine.scheduler.wait_for_change_since(observed_scheduler);
                    let admission_wake = engine.derivative_admission_wake.notified();
                    tokio::pin!(coordinator_wake);
                    tokio::pin!(scheduler_wake);
                    tokio::pin!(admission_wake);
                    tokio::select! {
                        _ = &mut coordinator_wake => {}
                        _ = &mut scheduler_wake => {}
                        _ = &mut admission_wake => {}
                    }
                    continue;
                };
                let lane = coordinator.lane(ticket).await;
                if engine
                    .scheduler
                    .highest_priority_in_family(
                        crate::derivative_coordinator::SCHEDULER_OWNER_PREFIX,
                    )
                    .await
                    .is_some_and(|priority| {
                        priority > crate::derivative_coordinator::scheduler_priority(lane)
                    })
                {
                    let observed = coordinator.change_generation();
                    let observed_scheduler = engine.scheduler.change_generation();
                    coordinator.requeue(ticket).await;
                    drop(admission);
                    let coordinator_wake = coordinator.wait_for_change_since(observed);
                    let scheduler_wake = engine.scheduler.wait_for_change_since(observed_scheduler);
                    tokio::pin!(coordinator_wake);
                    tokio::pin!(scheduler_wake);
                    tokio::select! {
                        _ = &mut coordinator_wake => {}
                        _ = &mut scheduler_wake => {}
                    }
                    continue;
                }
                let Some(key) = coordinator.work_key(ticket).await else {
                    drop(admission);
                    continue;
                };
                *active_ticket.lock().expect("hosted active ticket poisoned") = Some(ticket);
                let attempt = tokio::spawn({
                    let engine = engine.clone();
                    let runtime = runtime.clone();
                    async move {
                        engine
                            .process_hosted_derivative(runtime, ticket, key, admission)
                            .await;
                    }
                });
                if attempt.await.is_err() {
                    runtime.coordinator.abort_attempt_as_failure(ticket).await;
                }
                *active_ticket.lock().expect("hosted active ticket poisoned") = None;
            }
        });
        #[cfg(debug_assertions)]
        {
            *self.hosted_driver_abort_handle.lock().await = Some(driver.abort_handle());
        }
    }

    async fn process_hosted_derivative(
        &self,
        runtime: Arc<SelectionRuntime>,
        ticket: WorkTicket,
        key: WorkKey,
        _admission: HostedAdmission,
    ) {
        let waiter_scopes = runtime.coordinator.waiter_scopes(ticket).await;
        let generation_scope = if waiter_scopes
            .iter()
            .any(|scope| *scope == GalleryScope::IncludeSubfolders)
        {
            GalleryScope::IncludeSubfolders
        } else {
            GalleryScope::CurrentFolder
        };
        let Some((asset, source, spec)) = self.pending_hosted_derivative(&key, generation_scope)
        else {
            runtime.coordinator.discard(ticket).await;
            return;
        };
        #[cfg(debug_assertions)]
        if self.consume_hosted_panic_before_admission_test_hook().await {
            panic!("injected hosted derivative panic before commit admission");
        }
        #[cfg(debug_assertions)]
        self.hosted_attempts.fetch_add(1, Ordering::AcqRel);
        let prerequisite = (key.class == DerivativeClass::ScreenPreview).then(|| {
            DerivativeKey::compute(&derivative_spec_for_gallery(
                &asset,
                DerivativeClass::WallThumbnail,
            ))
            .as_str()
            .to_owned()
        });
        let Some(permit) = runtime
            .coordinator
            .admit_commit(ticket, key.selection, prerequisite.as_deref())
            .await
        else {
            runtime.coordinator.discard(ticket).await;
            return;
        };
        let group = key.selection.group_id;
        let Ok(_protected) = ProtectedGroupGuard::new(self.protected_groups.clone(), group) else {
            runtime.coordinator.fail_commit_with_error(permit).await;
            return;
        };
        let cache_root = self.cache_root.clone();
        let catalog_path = self.catalog_path.clone();
        let protected = self.protected_groups.clone();
        let asset_id = key.asset_id;
        let class = key.class;
        let signature = spec.signature;
        let orientation = spec.orientation;
        let generation_spec = spec.clone();
        let generation_cache_root = cache_root.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let generator = ImageDerivativeGenerator::new(&generation_cache_root)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
            if class == DerivativeClass::ScreenPreview {
                generator
                    .encode_screen_preview(
                        &source,
                        &photo_cache::DerivativeSpec {
                            asset_id,
                            signature,
                            orientation,
                            kind: photo_cache::DerivativeKind::ScreenPreview,
                            decoder_version: DERIVATIVE_DECODER_VERSION.to_owned(),
                            colour_space: "srgb".to_owned(),
                            target: photo_cache::DerivativeTarget::LongEdge(4096),
                        },
                    )
                    .map(HostedDerivativePrepared::Screen)
                    .map_err(|_| AppServiceError::DerivativeFailed)
            } else {
                let encoded = generator
                    .encode_wall_thumbnail(&source, &generation_spec)
                    .map_err(|_| AppServiceError::DerivativeFailed)?;
                Ok(HostedDerivativePrepared::Wall(encoded))
            }
        })
        .await;
        let Some(prepared) = (match prepared {
            Ok(Ok(prepared)) => Some(prepared),
            Ok(Err(_)) | Err(_) => None,
        }) else {
            runtime.coordinator.fail_commit_with_error(permit).await;
            return;
        };
        // Hold the publication fence while checking membership and publishing
        // the immutable cache/catalog state. Runtime scan writes take this
        // same fence, so no in-process scan can cross the decision boundary.
        let _publication = self.hosted_publication_fence.lock().await;
        let admitted_scopes = self.admitted_hosted_scopes(&key, &waiter_scopes);
        if admitted_scopes.is_empty()
            || self
                .pending_hosted_derivative(&key, generation_scope)
                .is_none()
        {
            runtime.coordinator.fail_commit(permit).await;
            return;
        }
        #[cfg(debug_assertions)]
        self.wait_hosted_commit_publication_test_gate().await;
        // The test gate also represents the last externally observable fence:
        // re-read after it so a deterministic membership/signature mutation
        // rejects the staged bytes before either cache or catalog publication.
        let admitted_scopes = self.admitted_hosted_scopes(&key, &waiter_scopes);
        if admitted_scopes.is_empty()
            || self
                .pending_hosted_derivative(&key, generation_scope)
                .is_none()
        {
            runtime.coordinator.fail_commit(permit).await;
            return;
        }
        let result = tokio::task::spawn_blocking(move || {
            let generator = ImageDerivativeGenerator::new(&cache_root)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
            let mut catalog = Catalog::open(&catalog_path).map_err(AppServiceError::Catalog)?;
            match prepared {
                HostedDerivativePrepared::Screen(encoded) => generator
                    .commit_screen_preview_repairing(
                        encoded,
                        &spec,
                        group,
                        &mut catalog,
                        CacheBudget::automatic(&cache_root).map_err(AppServiceError::Cache)?,
                        &protected,
                    )
                    .map_err(|_| AppServiceError::DerivativeFailed),
                HostedDerivativePrepared::Wall(encoded) => {
                    let generated = generator
                        .commit_wall_thumbnail(encoded, &spec)
                        .map_err(|_| AppServiceError::DerivativeFailed)?;
                    catalog.upsert_derivative(&NewDerivative {
                        id: photo_domain::DerivativeId::new(),
                        asset_id,
                        folder_group_id: group,
                        kind: derivative_kind_name_for_gallery(class).to_owned(),
                        cache_key: generated.key.as_str().to_owned(),
                        relative_cache_path: generated.relative_path.clone(),
                        size_bytes: generated.size_bytes,
                        durable: true,
                        created_at: crate::service::unix_timestamp(),
                    })?;
                    Ok(generated)
                }
            }
        })
        .await;
        let result = match result {
            Ok(Ok(generated)) => Some(generated),
            Ok(Err(_)) | Err(_) => None,
        };
        if let Some(generated) = result {
            #[cfg(debug_assertions)]
            self.wait_hosted_commit_post_publication_test_gate().await;
            // Keep a final defensive fence for mutations made through a
            // separate Catalog connection. If it trips, leave immutable
            // shared state intact and suppress this request's publication.
            let admitted_scopes = self.admitted_hosted_scopes(&key, &waiter_scopes);
            if admitted_scopes.is_empty()
                || self
                    .pending_hosted_derivative(&key, generation_scope)
                    .is_none()
            {
                let _ = self.rollback_hosted_publication(&generated, group);
                runtime.coordinator.fail_commit(permit).await;
                return;
            }
            let reference = DerivativeReference {
                asset_id: asset_id.as_uuid().hyphenated().to_string(),
                kind: class,
                key: generated.key.as_str().to_owned(),
            };
            runtime
                .publish(WallUpdate::DerivativesReady {
                    selection_id: runtime.selection.id.clone(),
                    derivatives: vec![reference.clone()],
                })
                .await;
            runtime
                .coordinator
                .complete_commit_filtered(permit, reference, &admitted_scopes)
                .await;
        } else {
            runtime.coordinator.fail_commit_with_error(permit).await;
        }
    }

    fn pending_hosted_derivative(
        &self,
        key: &WorkKey,
        scope: GalleryScope,
    ) -> Option<(
        photo_catalog::AssetRecord,
        PathBuf,
        photo_cache::DerivativeSpec,
    )> {
        let state = self.state.lock().ok()?;
        let asset = state.libraries.catalog().find_asset(key.asset_id).ok()??;
        if asset.media_kind == photo_domain::MediaKind::Video
            || asset.availability != photo_domain::Availability::Available
            || asset.availability != key.availability
            || state
                .libraries
                .catalog()
                .wall_records_for_assets_scoped(key.selection.group_id, scope, &[key.asset_id])
                .ok()?
                .is_empty()
        {
            return None;
        }
        let spec = derivative_spec_for_gallery(&asset, key.class);
        if DerivativeKey::compute(&spec).as_str() != key.cache_key {
            return None;
        }
        if key.class == DerivativeClass::ScreenPreview {
            let wall = derivative_spec_for_gallery(&asset, DerivativeClass::WallThumbnail);
            let wall_key = DerivativeKey::compute(&wall);
            let ready = state
                .libraries
                .catalog()
                .find_derivative(asset.id, "wall_thumbnail", wall_key.as_str())
                .ok()??;
            if !derivative_record_is_valid(&self.cache_root, &ready) {
                return None;
            }
        }
        let library = state
            .libraries
            .catalog()
            .find_library(key.selection.library_id)
            .ok()??;
        let root = library.canonical_root_key.to_path_buf().ok()?;
        let relative = asset.relative_path.to_path_buf().ok()?;
        let source = root.join(relative);
        if !std::fs::symlink_metadata(&source).ok()?.is_file() {
            return None;
        }
        Some((asset, source, spec))
    }

    /// Removes only this requester's link when a post-publication fence finds
    /// that the staged result is no longer authorized. Shared immutable bytes
    /// remain available to any other group link; an unlinked row is removed
    /// before physical cleanup so an orphaned file can never be served by key.
    fn rollback_hosted_publication(
        &self,
        generated: &photo_cache::GeneratedDerivative,
        group: FolderGroupId,
    ) -> Result<(), AppServiceError> {
        let mut catalog = Catalog::open(&self.catalog_path)?;
        let Some(record) = catalog.find_derivative_by_cache_key(generated.key.as_str())? else {
            return Ok(());
        };
        catalog.remove_derivative_group_links(&[record.id], &[group])?;
        if catalog
            .find_derivative_by_cache_key(generated.key.as_str())?
            .is_none()
        {
            CacheWriter::new(&self.cache_root)?.remove_checked(&record.relative_cache_path)?;
        }
        Ok(())
    }

    fn admitted_hosted_scopes(&self, key: &WorkKey, scopes: &[GalleryScope]) -> Vec<GalleryScope> {
        let Ok(state) = self.state.lock() else {
            return Vec::new();
        };
        let mut admitted = Vec::new();
        for scope in scopes.iter().copied() {
            if admitted.contains(&scope) {
                continue;
            }
            let Ok(records) = state.libraries.catalog().wall_records_for_assets_scoped(
                key.selection.group_id,
                scope,
                &[key.asset_id],
            ) else {
                continue;
            };
            if !records.is_empty() {
                admitted.push(scope);
            }
        }
        admitted
    }

    #[cfg(debug_assertions)]
    async fn consume_hosted_panic_before_admission_test_hook(&self) -> bool {
        let mut hook = self.hosted_panic_before_admission.lock().await;
        let installed = *hook;
        *hook = false;
        installed
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_derivative_panic_before_admission_test_hook(&self) {
        *self.hosted_panic_before_admission.lock().await = true;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_commit_post_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_commit_post_publication_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn abort_hosted_derivative_driver_for_test(&self) {
        if let Some(handle) = self.hosted_driver_abort_handle.lock().await.take() {
            handle.abort();
        }
    }

    pub fn open_derivative(&self, opaque_id: &str) -> Result<ManagedDerivative, AppServiceError> {
        if opaque_id.is_empty() || opaque_id.len() > 512 || !opaque_id.is_ascii() {
            return Err(AppServiceError::UnknownAsset);
        }
        let record = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            state
                .libraries
                .catalog()
                .find_derivative_by_cache_key(opaque_id)?
                .ok_or(AppServiceError::UnknownAsset)?
        };
        if !matches!(record.kind.as_str(), "wall_thumbnail" | "screen_preview") {
            return Err(AppServiceError::UnknownAsset);
        }
        let writer = CacheWriter::new(&self.cache_root)?;
        let mut file = writer.open_checked(&record.relative_cache_path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)
            .map_err(|error| AppServiceError::Cache(error.into()))?;
        if u64::try_from(bytes.len()).ok() != Some(record.size_bytes)
            || bytes.get(..3) != Some(&[0xff, 0xd8, 0xff])
            || image::load_from_memory(&bytes).is_err()
        {
            return Err(AppServiceError::UnknownAsset);
        }
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0))
            .map_err(|error| AppServiceError::Cache(error.into()))?;
        let content_length = file
            .metadata()
            .map_err(|error| AppServiceError::Cache(error.into()))?
            .len();
        if content_length != record.size_bytes {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(ManagedDerivative {
            file,
            content_type: "image/jpeg",
            content_length,
            etag: format!("\"{}\"", record.cache_key),
        })
    }

    pub fn subscribe(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_with_origin(
            selection,
            client_id,
            scope,
            after_event_id,
            SubscriptionOrigin::Hosted,
        )
    }

    pub(crate) fn subscribe_desktop(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_with_origin(
            selection,
            client_id,
            scope,
            after_event_id,
            SubscriptionOrigin::Desktop,
        )
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn subscribe_desktop_for_test(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_desktop(selection, client_id, scope, after_event_id)
    }

    fn subscribe_with_origin(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
        origin: SubscriptionOrigin,
    ) -> SelectionEventSubscription {
        let runtime = self.runtime(selection);
        let _publication = runtime
            .publication
            .lock()
            .expect("runtime publication poisoned");
        let receiver = runtime.updates.subscribe();
        let (client_token, scope_receiver) = runtime.register(client_id.clone(), scope, origin);
        let lifecycle = runtime.lifecycle_receiver();
        let history = runtime
            .history
            .lock()
            .expect("runtime history poisoned")
            .clone();
        let head = runtime.next_event_id.load(Ordering::Acquire);
        let oldest = history.front().map(|event| event.id);
        let (backlog, lagged, resync_after) = match after_event_id {
            None => (history.into(), false, None),
            Some(after)
                if after > head || oldest.is_some_and(|first| after.saturating_add(1) < first) =>
            {
                (VecDeque::new(), true, Some(head))
            }
            Some(after) => (
                history
                    .into_iter()
                    .filter(|event| event.id > after)
                    .collect(),
                false,
                None,
            ),
        };
        drop(_publication);
        if origin == SubscriptionOrigin::Hosted {
            runtime.start_lease_reaper(self.clone());
        }
        SelectionEventSubscription {
            backlog,
            receiver,
            runtime,
            client_id,
            client_token,
            scope: scope_receiver,
            lifecycle,
            lagged,
            resync_after,
            resume_after: None,
            engine: self.clone(),
            terminal: None,
            terminal_event_id: None,
            last_seen_event_id: after_event_id
                .filter(|after| *after <= head)
                .unwrap_or(head),
        }
    }

    pub async fn update_client_interaction(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_with_origin(
            selection,
            client_id,
            scope,
            interaction,
            SubscriptionOrigin::Hosted,
        )
        .await
    }

    pub(crate) async fn update_client_interaction_desktop(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_with_origin(
            selection,
            client_id,
            scope,
            interaction,
            SubscriptionOrigin::Desktop,
        )
        .await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn update_client_interaction_desktop_for_test(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_desktop(selection, client_id, scope, interaction)
            .await
    }

    async fn update_client_interaction_with_origin(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
        origin: SubscriptionOrigin,
    ) -> Result<bool, AppServiceError> {
        let runtime = self.runtime(selection);
        let found = {
            let mut demand = runtime
                .client_demand
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let token = demand
                .iter()
                .filter(|(_, value)| value.client_id == client_id && value.origin == origin)
                .map(|(token, _)| *token)
                .max();
            let Some(token) = token else {
                return Ok(false);
            };
            let value = demand.get_mut(&token).expect("demand token was present");
            value.scope = scope;
            let _ = value.scope_sender.send(scope);
            value.interaction = interaction;
            value.lease_until = if origin == SubscriptionOrigin::Hosted
                && interaction == InteractionState::Active
            {
                tokio::time::Instant::now() + std::time::Duration::from_secs(30)
            } else {
                tokio::time::Instant::now()
            };
            true
        };
        if !found {
            return Ok(false);
        }
        if origin == SubscriptionOrigin::Hosted {
            self.refresh_scheduler_interaction().await;
        }
        runtime.lease_wake.notify_waiters();
        Ok(true)
    }

    pub(crate) async fn refresh_scheduler_interaction(&self) {
        let active = self
            .runtimes
            .lock()
            .ok()
            .map(|runtimes| {
                runtimes
                    .values()
                    .filter_map(Weak::upgrade)
                    .any(|runtime| runtime.has_active_lease())
            })
            .unwrap_or(false);
        self.scheduler
            .set_interaction_mode(if active {
                photo_indexer::InteractionMode::Active
            } else {
                photo_indexer::InteractionMode::Idle
            })
            .await;
    }

    pub(crate) async fn start_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
        scan_generation: u64,
    ) -> Result<(), AppServiceError> {
        #[cfg(test)]
        if self.fail_next_start.swap(false, Ordering::AcqRel) {
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test startup failure".to_owned(),
            )));
        }
        if runtime.cancellation_requested() {
            runtime.finish_scan(
                scan_generation,
                crate::hosted_runtime::ScanLifecycle::Cancelled,
            );
            return Ok(());
        }
        if !self.ensure_runtime_source(&runtime).await? {
            return Ok(());
        }
        let (root, selected) = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::StatePoisoned)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let selected = runtime
                .selection
                .relative_folder
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            (root, selected)
        };
        // An unavailable source is a normal cached-browsing state. Check
        // lexical existence before canonicalization so a missing root can
        // still publish SourceUnavailable and leave cached wall rows readable.
        if !root.is_dir() || !root.join(&selected).is_dir() {
            self.mark_runtime_source_unavailable(&runtime).await?;
            return Ok(());
        }
        if runtime.cancellation_requested() {
            runtime.finish_scan(
                scan_generation,
                crate::hosted_runtime::ScanLifecycle::Cancelled,
            );
            return Ok(());
        }
        let generation = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let selection_root = root.join(&selected);
            let canonical_root = std::fs::canonicalize(&root).map_err(|_| {
                AppServiceError::OpenRecent(photo_core::AddLibraryError::NotDirectory(root.clone()))
            })?;
            let canonical_selection_root =
                std::fs::canonicalize(&selection_root).map_err(|_| {
                    AppServiceError::OpenRecent(photo_core::AddLibraryError::NotDirectory(
                        selection_root.clone(),
                    ))
                })?;
            if !canonical_selection_root.starts_with(&canonical_root) {
                return Err(AppServiceError::OpenRecent(
                    photo_core::AddLibraryError::InvalidSelection,
                ));
            }
            let generation = state.libraries.catalog_mut().begin_generation_for_group(
                runtime.selection.library_id,
                runtime.selection.group_id,
            )?;
            generation
        };
        let indexer = Indexer::with_scheduler(
            self.metadata_reader.clone(),
            photo_core::FolderPolicyEngine::new(Vec::new())
                .map_err(|e| AppServiceError::LibrarySetup(std::io::Error::other(e.to_string())))?,
            self.scheduler.clone(),
        );
        let selection_root = root.join(&selected);
        if runtime.cancellation_requested() {
            runtime.finish_scan(
                scan_generation,
                crate::hosted_runtime::ScanLifecycle::Cancelled,
            );
            return Ok(());
        }
        let handle = indexer
            .start(
                ScanRequest::new(selection_root.clone())
                    .for_library(runtime.selection.library_id)
                    .roots(root, selection_root)
                    .for_folder_group(runtime.selection.group_id),
            )
            .map_err(|e| AppServiceError::LibrarySetup(std::io::Error::other(e.to_string())))?;
        runtime.install_scan_sender(scan_generation, handle.cancellation_sender());
        let engine = self.clone();
        tokio::spawn(async move {
            engine
                .drain_runtime_scan(runtime, handle, generation, scan_generation)
                .await;
        });
        Ok(())
    }

    /// Checks source availability even when a completed catalog means no scan
    /// is needed. This is intentionally a cheap filesystem check so cached
    /// wall rows remain queryable while an offline source is reported once.
    pub(crate) async fn ensure_runtime_source(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Result<bool, AppServiceError> {
        #[cfg(test)]
        if self.fail_next_source_check.swap(false, Ordering::AcqRel) {
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test source check failure".to_owned(),
            )));
        }
        let available = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let selected = runtime
                .selection
                .relative_folder
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            root.is_dir() && root.join(selected).is_dir()
        };
        if available {
            runtime
                .source_unavailable_reported
                .store(false, Ordering::Release);
            return Ok(true);
        }
        self.mark_runtime_source_unavailable(runtime).await?;
        Ok(false)
    }

    async fn mark_runtime_source_unavailable(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Result<(), AppServiceError> {
        if runtime.cancellation_requested() {
            runtime.finish_scan(
                runtime.current_generation(),
                crate::hosted_runtime::ScanLifecycle::Cancelled,
            );
            return Ok(());
        }
        let should_publish = {
            let _publication = self.hosted_publication_fence.lock().await;
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            if root.is_dir() {
                state
                    .libraries
                    .catalog_mut()
                    .mark_group_offline(runtime.selection.library_id, runtime.selection.group_id)?;
            } else {
                state
                    .libraries
                    .catalog_mut()
                    .mark_root_offline(runtime.selection.library_id)?;
            }
            runtime
                .source_unavailable_reported
                .swap(true, Ordering::AcqRel)
                == false
        };
        if should_publish {
            runtime
                .publish(WallUpdate::SourceUnavailable {
                    selection_id: runtime.selection.id().to_owned(),
                    source_id: runtime
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                })
                .await;
        }
        runtime.finish_scan(
            runtime.current_generation(),
            crate::hosted_runtime::ScanLifecycle::SourceUnavailable,
        );
        self.remove_runtime_if_dead(runtime.selection.group_id, runtime);
        Ok(())
    }

    #[doc(hidden)]
    pub fn runtime_count_for_test(&self) -> usize {
        self.runtimes
            .lock()
            .expect("runtime registry poisoned")
            .values()
            .filter(|runtime| runtime.upgrade().is_some())
            .count()
    }

    pub(crate) fn current_event_id(&self, selection: &GallerySelection) -> u64 {
        self.runtime(selection)
            .next_event_id
            .load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn current_event_id_for_test(&self, selection: &GallerySelection) -> u64 {
        self.current_event_id(selection)
    }

    pub(crate) fn runtime_scan_state(
        &self,
        selection: &GallerySelection,
    ) -> Option<tokio::sync::watch::Receiver<crate::hosted_runtime::ScanLifecycle>> {
        self.runtimes
            .lock()
            .ok()
            .and_then(|runtimes| runtimes.get(&selection.group_id).and_then(Weak::upgrade))
            .filter(|runtime| runtime.selection == *selection)
            .map(|runtime| runtime.lifecycle_receiver())
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn publish_update_for_test(
        &self,
        selection: &GallerySelection,
        update: WallUpdate,
    ) -> crate::SequencedWallUpdate {
        self.runtime(selection).publish(update).await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn set_interaction_for_test(&self, state: InteractionState) {
        self.scheduler
            .set_interaction_mode(match state {
                InteractionState::Idle => photo_indexer::InteractionMode::Idle,
                InteractionState::Active => photo_indexer::InteractionMode::Active,
            })
            .await;
    }

    #[doc(hidden)]
    pub async fn aggregate_scope_for_test(&self, selection: &GallerySelection) -> GalleryScope {
        self.runtime(selection).aggregate_scope()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn scheduler_permits_for_test(&self) -> usize {
        self.scheduler.available_background_permits()
    }

    #[cfg(test)]
    pub(crate) fn fail_next_start_for_test(&self) {
        self.fail_next_start.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_source_check_for_test(&self) {
        self.fail_next_source_check.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_batch_for_test(&self) {
        self.fail_next_batch.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_join_for_test(&self) {
        self.fail_next_join.store(true, Ordering::Release);
    }

    async fn drain_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
        mut handle: photo_indexer::ScanHandle,
        generation: u64,
        scan_generation: u64,
    ) {
        let mut batch = Vec::new();
        let mut persistence_failed = false;
        let mut stream_open = true;
        while stream_open {
            let Some(first) = handle.events.recv().await else {
                break;
            };
            batch.push(first);
            let deadline = tokio::time::sleep(std::time::Duration::from_millis(50));
            tokio::pin!(deadline);
            while batch.len() < 200 {
                tokio::select! {
                    _ = &mut deadline => break,
                    event = handle.events.recv() => match event {
                        Some(event) => batch.push(event),
                        None => { stream_open = false; break; }
                    }
                }
            }
            if runtime.cancellation_requested() {
                break;
            }
            if self
                .apply_runtime_batch(&runtime, generation, &batch)
                .await
                .is_err()
            {
                persistence_failed = true;
                let _ = handle.cancel();
                break;
            }
            batch.clear();
        }
        let successful = handle
            .join()
            .await
            .map(|summary| !summary.cancelled)
            .unwrap_or(false);
        #[cfg(test)]
        let successful = if self.fail_next_join.swap(false, Ordering::AcqRel) {
            false
        } else {
            successful
        };
        let mut completed = false;
        if successful && !persistence_failed && !runtime.cancellation_requested() {
            let _publication = self.hosted_publication_fence.lock().await;
            completed = if let Ok(mut state) = self.state.lock() {
                state
                    .libraries
                    .catalog_mut()
                    .complete_generation_for_group(
                        runtime.selection.library_id,
                        runtime.selection.group_id,
                        generation,
                    )
                    .is_ok()
            } else {
                false
            };
            if completed {
                runtime
                    .settled
                    .store(true, std::sync::atomic::Ordering::Release);
                let _ = runtime
                    .publish(WallUpdate::MetadataSettled {
                        selection_id: runtime.selection.id().to_owned(),
                        source_id: runtime
                            .selection
                            .library_id
                            .as_uuid()
                            .hyphenated()
                            .to_string(),
                        generation,
                    })
                    .await;
            } else {
                persistence_failed = true;
            }
        }
        let terminal = if runtime.cancellation_requested() {
            crate::hosted_runtime::ScanLifecycle::Cancelled
        } else if persistence_failed {
            crate::hosted_runtime::ScanLifecycle::Failed
        } else if completed {
            crate::hosted_runtime::ScanLifecycle::Completed
        } else {
            crate::hosted_runtime::ScanLifecycle::Failed
        };
        runtime.finish_scan(scan_generation, terminal);
        self.remove_runtime_if_dead(runtime.selection.group_id, &runtime);
    }

    async fn apply_runtime_batch(
        &self,
        runtime: &Arc<SelectionRuntime>,
        generation: u64,
        events: &[IndexEvent],
    ) -> Result<(), AppServiceError> {
        #[cfg(test)]
        if self.fail_next_batch.swap(false, Ordering::AcqRel) {
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test persistence failure".to_owned(),
            )));
        }
        if runtime.cancellation_requested() {
            return Ok(());
        }
        let progress = events
            .iter()
            .filter_map(|e| {
                if let IndexEvent::Progress(progress) = e {
                    Some(*progress)
                } else {
                    None
                }
            })
            .last();
        let shaped = events
            .iter()
            .filter_map(|e| match e {
                IndexEvent::ShapeReady { asset_id, .. }
                | IndexEvent::ShapeFallback { asset_id, .. } => Some(*asset_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let updates = {
            let _publication = self.hosted_publication_fence.lock().await;
            let mut updates = Vec::new();
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let mut writer = photo_indexer::CatalogWriter::new(
                state.libraries.catalog_mut(),
                runtime.selection.library_id,
                generation,
            );
            if let Err(error) = writer.apply_batch(events) {
                return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                    format!("hosted scan batch could not be persisted: {error}"),
                )));
            }
            let progress_update = progress.map(|progress| WallUpdate::Progress {
                selection_id: runtime.selection.id().to_owned(),
                generation,
                progress: crate::scan::progress_dto(progress),
            });
            if let Some(update) = progress_update {
                updates.push(update);
            }
            if let Ok(records) = state.libraries.catalog().wall_records_for_assets_scoped(
                runtime.selection.group_id,
                GalleryScope::IncludeSubfolders,
                &shaped,
            ) {
                if let Ok(assets) = crate::service::wall_assets_with_derivatives(
                    state.libraries.catalog(),
                    &records,
                    OrderState::Provisional,
                ) {
                    if !assets.is_empty() {
                        updates.push(WallUpdate::CatalogBatch {
                            selection_id: runtime.selection.id().to_owned(),
                            assets,
                            order_state: OrderState::Provisional,
                            generation,
                            progress: progress.map(crate::scan::progress_dto).unwrap_or_default(),
                        });
                    }
                }
            }
            updates
        };
        for update in updates {
            if runtime.cancellation_requested() {
                break;
            }
            let _ = runtime.publish(update).await;
        }
        Ok(())
    }
}
