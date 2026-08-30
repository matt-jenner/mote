use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

use photo_cache::CacheWriter;
use photo_catalog::{Catalog, CatalogError, NewFolderGroup, WallOrder};
use photo_core::{LibraryService, RealSourceFs};
use photo_domain::{FolderGroupId, GalleryScope, LibraryId, RelativePathKey};
use photo_indexer::{DefaultMetadataReader, IndexEvent, Indexer, MetadataReader, ScanRequest};

use crate::hosted_runtime::{SelectionEventSubscription, SelectionRuntime, SubscriptionOrigin};
use crate::service::{ReaderAdapter, ServiceState};
use crate::{
    AppConfig, AppServiceError, InteractionState, OrderState, SourceAvailability, WallPage,
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
    #[cfg(test)]
    fail_next_start: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_source_check: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_batch: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_join: Arc<AtomicBool>,
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
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn from_shared_state(
        state: Arc<Mutex<ServiceState>>,
        scheduler: Arc<photo_indexer::IndexScheduler>,
        metadata_reader: ReaderAdapter,
        shared_coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    ) -> Self {
        Self {
            state,
            scheduler,
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            metadata_reader,
            hosted_library_id: None,
            shared_coordinator: Some(shared_coordinator),
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
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
