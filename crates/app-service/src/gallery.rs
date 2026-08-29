use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

use photo_cache::{CacheBudget, CacheWriter, ProtectedGroups};
use photo_catalog::{Catalog, CatalogError, NewFolderGroup, WallOrder};
use photo_core::{LibraryService, RealSourceFs};
use photo_domain::{FolderGroupId, GalleryScope, LibraryId, RelativePathKey};
use photo_indexer::{DefaultMetadataReader, IndexEvent, Indexer, MetadataReader, ScanRequest};

use crate::hosted_runtime::{SelectionEventSubscription, SelectionRuntime};
use crate::service::ReaderAdapter;
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
            epoch: 0,
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

pub(crate) struct GalleryState {
    pub(crate) libraries: LibraryService<RealSourceFs>,
    pub(crate) hosted_library_id: LibraryId,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct GalleryEngine {
    pub(crate) state: Arc<Mutex<GalleryState>>,
    pub(crate) scheduler: Arc<photo_indexer::IndexScheduler>,
    pub(crate) runtimes: Arc<Mutex<HashMap<FolderGroupId, Weak<SelectionRuntime>>>>,
    pub(crate) catalog_path: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) cache_budget: CacheBudget,
    pub(crate) protected_groups: ProtectedGroups,
    pub(crate) metadata_reader: ReaderAdapter,
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
        let canonical = canonicalize_for_identity(&source_root);
        let hosted_library_id = if let Some(library) = libraries
            .catalog()
            .list_libraries()?
            .into_iter()
            .find(|library| {
                library.canonical_root_key.to_path_buf().ok().as_deref()
                    == Some(canonical.as_path())
                    || Path::new(&library.display_path) == source_root.as_path()
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
            state: Arc::new(Mutex::new(GalleryState {
                libraries,
                hosted_library_id,
            })),
            scheduler: Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            catalog_path: config.catalog_path(),
            cache_root: config.cache_dir().to_owned(),
            cache_budget: CacheBudget::automatic(config.cache_dir())?,
            protected_groups: ProtectedGroups::default(),
            metadata_reader: ReaderAdapter::new(reader),
        })
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
        let library = state
            .libraries
            .catalog()
            .find_library(state.hosted_library_id)?
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
        let hosted_library_id = state.hosted_library_id;
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
            library_id: state.hosted_library_id,
            group_id: group,
            relative_folder: state
                .libraries
                .catalog()
                .folder_group(group)?
                .ok_or(AppServiceError::StatePoisoned)?
                .relative_path,
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
        if record.library_id != state.hosted_library_id {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(GallerySelection {
            id: id.to_owned(),
            library_id: record.library_id,
            group_id: record.id,
            relative_folder: record.relative_path,
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
        if group.library_id != state.hosted_library_id
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
            return runtime;
        }
        let settled = self
            .state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .libraries
                    .catalog()
                    .has_completed_generation_for_group(selection.library_id, selection.group_id)
                    .ok()
            })
            .unwrap_or(false);
        let runtime = SelectionRuntime::new(selection.clone(), self.scheduler.clone(), settled);
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
        let runtime = self.runtime(selection);
        let _publication = runtime
            .publication
            .lock()
            .expect("runtime publication poisoned");
        let receiver = runtime.updates.subscribe();
        let client_token = runtime.register(client_id.clone(), scope);
        let history = runtime
            .history
            .lock()
            .expect("runtime history poisoned")
            .clone();
        let head = runtime.next_event_id.load(Ordering::Acquire);
        let oldest = history.front().map(|event| event.id);
        let (backlog, lagged) = match after_event_id {
            None => (history.into(), false),
            Some(after)
                if after > head || oldest.is_some_and(|first| after.saturating_add(1) < first) =>
            {
                (VecDeque::new(), true)
            }
            Some(after) => (
                history
                    .into_iter()
                    .filter(|event| event.id > after)
                    .collect(),
                false,
            ),
        };
        drop(_publication);
        SelectionEventSubscription {
            backlog,
            receiver,
            runtime,
            client_id,
            client_token,
            scope,
            lagged,
            resume_after: None,
            engine: self.clone(),
        }
    }

    pub async fn update_client_interaction(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        let runtime = self.runtime(selection);
        let mut demand = runtime
            .client_demand
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let token = demand
            .iter()
            .filter(|(_, value)| value.client_id == client_id)
            .map(|(token, _)| *token)
            .max();
        let Some(token) = token else {
            return Ok(false);
        };
        let value = demand.get_mut(&token).expect("demand token was present");
        value.scope = scope;
        value.interaction = interaction;
        value.lease_until = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        Ok(true)
    }

    pub(crate) async fn start_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
    ) -> Result<(), AppServiceError> {
        let (root, selected, generation) = {
            let mut state = self
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
            let selection_root = root.join(&selected);
            // An unavailable source is a normal cached-browsing state. Check
            // lexical existence before canonicalization so a missing root can
            // still publish SourceUnavailable and leave cached wall rows
            // readable.
            if !root.is_dir() || !selection_root.is_dir() {
                if root.is_dir() {
                    state.libraries.catalog_mut().mark_group_offline(
                        runtime.selection.library_id,
                        runtime.selection.group_id,
                    )?;
                } else {
                    state
                        .libraries
                        .catalog_mut()
                        .mark_root_offline(runtime.selection.library_id)?;
                }
                drop(state);
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
                *runtime.scan_cancel.lock().await = None;
                self.remove_runtime_if_dead(runtime.selection.group_id, &runtime);
                return Ok(());
            }
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
            (root, selected, generation)
        };
        let indexer = Indexer::with_scheduler(
            self.metadata_reader.clone(),
            photo_core::FolderPolicyEngine::new(Vec::new())
                .map_err(|e| AppServiceError::LibrarySetup(std::io::Error::other(e.to_string())))?,
            self.scheduler.clone(),
        );
        let selection_root = root.join(&selected);
        let handle = indexer
            .start(
                ScanRequest::new(selection_root.clone())
                    .for_library(runtime.selection.library_id)
                    .roots(root, selection_root)
                    .for_folder_group(runtime.selection.group_id),
            )
            .map_err(|e| AppServiceError::LibrarySetup(std::io::Error::other(e.to_string())))?;
        *runtime.scan_cancel.lock().await = Some(handle.cancellation_sender());
        let engine = self.clone();
        tokio::spawn(async move {
            engine.drain_runtime_scan(runtime, handle, generation).await;
        });
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

    #[doc(hidden)]
    pub async fn aggregate_scope_for_test(&self, selection: &GallerySelection) -> GalleryScope {
        self.runtime(selection).aggregate_scope()
    }

    async fn drain_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
        mut handle: photo_indexer::ScanHandle,
        generation: u64,
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
        if successful && !persistence_failed {
            let completed = if let Ok(mut state) = self.state.lock() {
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
        if persistence_failed {
            let _ = runtime
                .publish(WallUpdate::Warning {
                    selection_id: runtime.selection.id().to_owned(),
                    source_id: runtime
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                    asset_id: None,
                    warning: crate::WallWarningState {
                        code: "catalogUnavailable".to_owned(),
                        retryable: true,
                    },
                })
                .await;
        }
        *runtime.scan_cancel.lock().await = None;
        self.remove_runtime_if_dead(runtime.selection.group_id, &runtime);
    }

    async fn apply_runtime_batch(
        &self,
        runtime: &Arc<SelectionRuntime>,
        generation: u64,
        events: &[IndexEvent],
    ) -> Result<(), AppServiceError> {
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
            let _ = runtime.publish(update).await;
        }
        Ok(())
    }
}
