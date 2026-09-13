use photo_app_service::{
    AppServiceError, BootstrapState, DerivativeRequest, InteractionState, WallQueryRequest,
    WallUpdate,
};
use photo_core::AddLibraryError;
use photo_domain::{Appearance, GalleryScope};
use std::{collections::HashSet, future::Future, path::PathBuf};
use tauri::{AppHandle, State, Theme, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::dto::{ChooseFolderResult, CommandError, CopyProgress, CopyResult};
use crate::protocol::forward_wall_updates_for_service;
use crate::state::{DesktopState, WallSubscriptionId};

#[tauri::command]
pub async fn copy_picked_originals(
    app: AppHandle,
    asset_ids: Option<Vec<String>>,
    on_event: tauri::ipc::Channel<CopyProgress>,
    state: State<'_, DesktopState>,
) -> Result<CopyResult, CommandError> {
    run_original_copy(
        &state,
        asset_ids,
        |initial| async move {
            let mut dialog = app.dialog().file();
            if let Some(initial) = initial {
                dialog = dialog.set_directory(initial);
            }
            let (sender, receiver) = tokio::sync::oneshot::channel();
            dialog.pick_folder(move |selected| {
                let _ = sender.send(selected);
            });
            receiver
                .await
                .map_err(|_| CommandError::internal())?
                .map(|selected| selected.into_path().map_err(|_| CommandError::internal()))
                .transpose()
        },
        move |event| {
            let _ = on_event.send(event);
        },
    )
    .await
}

async fn run_original_copy<F, P>(
    state: &DesktopState,
    asset_ids: Option<Vec<String>>,
    pick_destination: F,
    mut on_event: impl FnMut(CopyProgress) + Send + 'static,
) -> Result<CopyResult, CommandError>
where
    F: FnOnce(Option<PathBuf>) -> P,
    P: Future<Output = Result<Option<PathBuf>, CommandError>>,
{
    let guard = state.copy_operation.clone().try_lock_owned().map_err(|_| {
        CommandError::new("copyInProgress", "An original copy is already in progress.")
    })?;
    let service = state.service.clone();
    // Prepare every bounded batch before showing the dialog. Later pick changes
    // cannot affect even the final chunk of a large operation.
    let (batches, total, initial) = tauri::async_runtime::spawn_blocking(move || {
        let ids = match asset_ids {
            Some(ids) => ids,
            None => service
                .list_photo_picks()
                .map_err(map_service_error)?
                .items
                .into_iter()
                .map(|item| item.reference.asset_id)
                .collect(),
        };
        let mut seen = HashSet::new();
        let ids = ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(CommandError::new(
                "copyPreparationFailed",
                "Mote could not prepare these originals.",
            ));
        }
        let total = u32::try_from(ids.len()).map_err(|_| CommandError::internal())?;
        let batches = ids
            .chunks(250)
            .map(|ids| {
                service
                    .prepare_original_copy(ids)
                    .map(|batch| (batch, ids.to_vec()))
                    .map_err(map_service_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let initial = service
            .last_copy_destination()
            .map_err(map_service_error)?
            .filter(|path| path.is_dir());
        Ok((batches, total, initial))
    })
    .await
    .map_err(|_| CommandError::internal())??;
    let Some(destination) = pick_destination(initial).await? else {
        return Ok(CopyResult::Cancelled);
    };
    let service = state.service.clone();
    let last_completed_destination = state.last_completed_copy_destination.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // The worker owns the guard even if its awaiting command is dropped.
        let _guard = guard;
        let destination = std::fs::canonicalize(destination)
            .map_err(|_| map_service_error(AppServiceError::CopyDestinationUnavailable))?;
        let mut aggregate = photo_app_service::OriginalCopyResult {
            items: vec![],
            copied_count: 0,
            failed_count: 0,
            warning_code: None,
        };
        let mut completed = 0;
        on_event(CopyProgress {
            completed,
            total,
            item: None,
        });
        for (batch, ids) in batches {
            let result = service.copy_originals_with_progress(batch, &destination, |item| {
                completed += 1;
                on_event(CopyProgress {
                    completed,
                    total,
                    item: Some(item.clone()),
                });
            });
            let result = match result {
                Ok(result) => result,
                Err(error) if aggregate.items.is_empty() => return Err(map_service_error(error)),
                Err(error) => {
                    // A destination lost between chunks must not erase earlier successes.
                    let code = match error {
                        AppServiceError::CopyDestinationIsSource => "destination_is_source",
                        _ => "destination_unavailable",
                    };
                    let items = ids
                        .into_iter()
                        .map(|asset_id| photo_app_service::CopyItemResult {
                            asset_id,
                            status: photo_app_service::CopyItemStatus::Failed,
                            destination_name: None,
                            error_code: Some(code.into()),
                        })
                        .collect::<Vec<_>>();
                    for item in &items {
                        completed += 1;
                        on_event(CopyProgress {
                            completed,
                            total,
                            item: Some(item.clone()),
                        });
                    }
                    photo_app_service::OriginalCopyResult {
                        failed_count: items.len() as u32,
                        items,
                        copied_count: 0,
                        warning_code: None,
                    }
                }
            };
            aggregate.copied_count += result.copied_count;
            aggregate.failed_count += result.failed_count;
            aggregate.items.extend(result.items);
            if result.warning_code.is_some() {
                aggregate.warning_code = result.warning_code;
            }
        }
        if aggregate.copied_count > 0 {
            *last_completed_destination
                .lock()
                .map_err(|_| CommandError::internal())? = Some(destination);
        }
        Ok(CopyResult::Complete { result: aggregate })
    })
    .await
    .map_err(|_| CommandError::internal())?
}

#[tauri::command]
pub fn show_last_copy_destination(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    show_copy_destination(&state, |path| {
        app.opener()
            .open_path(
                path.to_str().ok_or_else(CommandError::internal)?,
                None::<&str>,
            )
            .map_err(|_| CommandError::internal())
    })
}

fn show_copy_destination(
    state: &DesktopState,
    open: impl FnOnce(PathBuf) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let session_destination = state
        .last_completed_copy_destination
        .lock()
        .map_err(|_| CommandError::internal())?
        .clone();
    let destination = match session_destination {
        Some(path) => Some(path),
        None => state
            .service
            .last_copy_destination()
            .map_err(map_service_error)?,
    }
    .filter(|path| path.is_dir())
    .ok_or(CommandError::new(
        "copyDestinationUnavailable",
        "The copy destination is unavailable.",
    ))?;
    open(destination)
}

#[tauri::command]
pub async fn get_bootstrap_state(
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .checked_bootstrap()
        .await
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn choose_folder(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<ChooseFolderResult, CommandError> {
    let selected = app.dialog().file().blocking_pick_folder();
    let Some(selected) = selected else {
        return Ok(ChooseFolderResult::Cancelled);
    };
    let path = selected.into_path().map_err(|error| {
        tracing::error!(%error, "native dialog returned an unusable path");
        CommandError::internal()
    })?;
    Ok(
        match state
            .service
            .open_folder(path)
            .await
            .map_err(map_service_error)?
        {
            Some(state) => ChooseFolderResult::Selected { state },
            None => ChooseFolderResult::Cancelled,
        },
    )
}

#[tauri::command]
pub fn update_appearance(
    appearance: Appearance,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    let bootstrap = state
        .service
        .update_appearance(appearance)
        .map_err(map_service_error)?;
    let theme = match appearance {
        Appearance::System => None,
        Appearance::Light => Some(Theme::Light),
        Appearance::Dark => Some(Theme::Dark),
    };
    window.set_theme(theme).map_err(|error| {
        tracing::error!(%error, "could not apply the native window theme");
        CommandError::internal()
    })?;
    Ok(bootstrap)
}

#[tauri::command]
pub async fn update_gallery_scope(
    scope: GalleryScope,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .update_gallery_scope(scope)
        .await
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn query_wall(
    request: WallQueryRequest,
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::WallPage, CommandError> {
    state
        .service
        .query_wall(request)
        .await
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn request_derivatives(
    request: DerivativeRequest,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state
        .service
        .request_derivatives(request)
        .await
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn set_wall_interaction(
    active: bool,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state
        .service
        .set_interaction(if active {
            InteractionState::Active
        } else {
            InteractionState::Idle
        })
        .await;
    Ok(())
}

#[tauri::command]
pub fn watch_wall_updates(
    on_event: tauri::ipc::Channel<WallUpdate>,
    state: State<'_, DesktopState>,
) -> Result<WallSubscriptionId, CommandError> {
    let receiver = state.service.subscribe_wall_updates();
    let service = state.service.clone();
    let (subscription_id, cancellation) = state.wall_subscriptions.register();
    let registry = state.wall_subscriptions.clone();
    let registered_id = subscription_id.clone();
    tauri::async_runtime::spawn(async move {
        forward_wall_updates_for_service(receiver, on_event, service, cancellation).await;
        registry.finish(&registered_id);
    });
    Ok(subscription_id)
}

#[tauri::command]
pub fn unwatch_wall_updates(
    subscription_id: WallSubscriptionId,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state.wall_subscriptions.cancel(&subscription_id);
    Ok(())
}

fn map_service_error(error: AppServiceError) -> CommandError {
    tracing::error!(%error, "desktop command failed");
    match error {
        AppServiceError::CopyPreparationFailed => CommandError::new(
            "copyPreparationFailed",
            "Mote could not prepare these originals.",
        ),
        AppServiceError::CopyDestinationUnavailable => CommandError::new(
            "copyDestinationUnavailable",
            "The copy destination is unavailable.",
        ),
        AppServiceError::CopyDestinationIsSource => CommandError::new(
            "copyDestinationIsSource",
            "Choose a destination outside your source folders.",
        ),
        AppServiceError::OpenRecent(AddLibraryError::Io(_)) => {
            CommandError::new("folderUnavailable", "The selected folder is unavailable.")
        }
        AppServiceError::OpenRecent(AddLibraryError::NotDirectory(_)) => {
            CommandError::new("folderNotDirectory", "Choose a folder, not a file.")
        }
        AppServiceError::OpenRecent(AddLibraryError::Overlaps { .. }) => CommandError::new(
            "folderOverlapsSource",
            "That folder overlaps an existing source.",
        ),
        AppServiceError::OpenRecent(AddLibraryError::OverlapsLocalState) => CommandError::new(
            "folderOverlapsLocalState",
            "That folder overlaps Mote's local data.",
        ),
        AppServiceError::InvalidLimit => {
            CommandError::new("invalidLimit", "The requested wall page is not valid.")
        }
        AppServiceError::DerivativeUnavailable => CommandError::new(
            "derivativeUnavailable",
            "Some requested previews could not be generated.",
        ),
        AppServiceError::InvalidAssetId
        | AppServiceError::ForeignAsset
        | AppServiceError::UnknownAsset => {
            CommandError::new("assetNotFound", "That photo is no longer available.")
        }
        AppServiceError::LocalState(_) => {
            CommandError::new("localStateUnavailable", "Mote cannot open its local data.")
        }
        _ => CommandError::internal(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use photo_app_service::AppServiceError;
    use photo_core::AddLibraryError;

    use super::map_service_error;

    struct CopyFixture {
        _temp: tempfile::TempDir,
        state: crate::state::DesktopState,
        destination: PathBuf,
        source: PathBuf,
        ids: Vec<String>,
    }

    impl CopyFixture {
        fn new() -> Self {
            // open_recent starts background reconciliation when called in a
            // Tokio context. Keep this catalog fixture independent of scanning.
            Self::with_count(2)
        }

        fn with_count(count: usize) -> Self {
            std::thread::spawn(move || Self::build(count))
                .join()
                .unwrap()
        }

        fn build(count: usize) -> Self {
            use photo_app_service::{AppConfig, AppService};
            use photo_catalog::{
                AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, ShapeStatus,
            };
            use photo_domain::{FolderGroupId, MediaKind, RelativePathKey};
            let temp = tempfile::tempdir().unwrap();
            let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
            let service = AppService::open(config.clone()).unwrap();
            let source = temp.path().join("photos");
            let destination = temp.path().join("exports");
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&destination).unwrap();
            let bootstrap = service.open_recent(&source).unwrap();
            let entry = &bootstrap.saved_folders.entries[0];
            let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&entry.folder_id).unwrap());
            let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
            let library = catalog.folder_group(group).unwrap().unwrap().library_id;
            let mut ids = vec![];
            for index in 0..count {
                let name = match index {
                    0 => "one.jpg".to_owned(),
                    1 => "two.jpg".to_owned(),
                    _ => format!("photo-{index}.jpg"),
                };
                let name = name.as_str();
                std::fs::write(source.join(name), name.as_bytes()).unwrap();
                let mut asset = NewAsset::minimal(
                    library,
                    RelativePathKey::from_relative_path(std::path::Path::new(name)).unwrap(),
                    name,
                    MediaKind::Jpeg,
                    1,
                );
                asset.folder_group_id = Some(group);
                catalog.upsert_asset(&asset).unwrap();
                catalog
                    .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: asset.id,
                        width: 3,
                        height: 2,
                        orientation: None,
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    })])
                    .unwrap();
                let id = asset.id.as_uuid().to_string();
                service.add_photo_pick(&id, &entry.folder_id).unwrap();
                ids.push(id);
            }
            Self {
                _temp: temp,
                state: crate::state::DesktopState {
                    service,
                    wall_subscriptions: Default::default(),
                    copy_operation: Default::default(),
                    last_completed_copy_destination: Default::default(),
                },
                destination,
                source,
                ids,
            }
        }
    }

    // Catches a lost preference incorrectly sending Show folder to an older export.
    #[tokio::test]
    async fn show_folder_uses_completed_operation_when_persisted_preference_is_unavailable() {
        let fixture = CopyFixture::new();
        super::run_original_copy(
            &fixture.state,
            None,
            |_| async { Ok(Some(fixture.destination.clone())) },
            |_| {},
        )
        .await
        .unwrap();
        // Simulate preference loss independently of the native operation state.
        let config = photo_app_service::AppConfig::new(
            fixture._temp.path().join("data"),
            fixture._temp.path().join("cache"),
        );
        let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
        catalog
            .set_last_copy_destination(&photo_domain::NativePathKey::from_path(&fixture.source))
            .unwrap();
        super::show_copy_destination(&fixture.state, |path| {
            assert_eq!(path, std::fs::canonicalize(&fixture.destination).unwrap());
            Ok(())
        })
        .unwrap();
    }

    // Catches chunking after the picker, per-chunk dialogs, and resetting progress at 250.
    #[tokio::test]
    async fn large_copy_prepares_every_chunk_before_picker_and_keeps_one_total() {
        let fixture = CopyFixture::with_count(251);
        let events = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let received = events.clone();
        let result = super::run_original_copy(
            &fixture.state,
            None,
            |_| async {
                fixture.state.service.clear_photo_picks().unwrap();
                Ok(Some(fixture.destination.clone()))
            },
            move |event| {
                received
                    .lock()
                    .unwrap()
                    .push((event.completed, event.total))
            },
        )
        .await
        .unwrap();
        let crate::dto::CopyResult::Complete { result } = result else {
            panic!("expected result")
        };
        assert_eq!((result.copied_count, result.failed_count), (251, 0));
        assert_eq!(
            std::fs::read_dir(&fixture.destination).unwrap().count(),
            251
        );
        assert_eq!(events.lock().unwrap().last(), Some(&(251, 251)));
        assert_eq!(result.items.last().unwrap().asset_id, fixture.ids[250]);
    }

    #[tokio::test]
    async fn invalid_later_chunk_aborts_before_picker_or_copying() {
        let fixture = CopyFixture::with_count(250);
        let mut ids = fixture.ids.clone();
        ids.push("not-an-asset-id".into());
        let error = super::run_original_copy(
            &fixture.state,
            Some(ids),
            |_| async { panic!("invalid later chunk must abort before picker") },
            |_| panic!("must not start copying"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "assetNotFound");
        assert_eq!(std::fs::read_dir(&fixture.destination).unwrap().count(), 0);
    }

    // Catches acquiring the lock after opening the picker, and cancellation saving a destination.
    #[tokio::test]
    async fn copy_cancel_releases_operation_without_events_or_preference() {
        let fixture = CopyFixture::new();
        let result = super::run_original_copy(
            &fixture.state,
            None,
            |initial| async move {
                assert_eq!(initial, None);
                Ok(None)
            },
            |_| panic!("cancel must not report copying"),
        )
        .await
        .unwrap();
        assert!(matches!(result, crate::dto::CopyResult::Cancelled));
        assert!(
            fixture
                .state
                .copy_operation
                .clone()
                .try_lock_owned()
                .is_ok()
        );
        assert_eq!(fixture.state.service.last_copy_destination().unwrap(), None);
        let guard = fixture
            .state
            .copy_operation
            .clone()
            .try_lock_owned()
            .unwrap();
        let error = super::run_original_copy(
            &fixture.state,
            None,
            |_| async { panic!("busy operation must not open another picker") },
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "copyInProgress");
        drop(guard);
    }

    // Catches resolving source membership after picker/mutation and hiding determinate progress.
    #[tokio::test]
    async fn copy_snapshots_before_picker_and_survives_remove_and_clear() {
        let fixture = CopyFixture::new();
        let events = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let received = events.clone();
        let result = super::run_original_copy(
            &fixture.state,
            None,
            |_| async {
                fixture
                    .state
                    .service
                    .remove_photo_pick(&fixture.ids[0])
                    .unwrap();
                fixture.state.service.clear_photo_picks().unwrap();
                Ok(Some(fixture.destination.clone()))
            },
            move |event| received.lock().unwrap().push(event),
        )
        .await
        .unwrap();
        let crate::dto::CopyResult::Complete { result } = result else {
            panic!("copy should complete")
        };
        assert_eq!((result.copied_count, result.failed_count), (2, 0));
        assert_eq!(
            std::fs::read(fixture.destination.join("one.jpg")).unwrap(),
            b"one.jpg"
        );
        assert!(
            fixture
                .state
                .service
                .list_photo_picks()
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .map(|event| (event.completed, event.total))
                .collect::<Vec<_>>(),
            [(0, 2), (1, 2), (2, 2)]
        );
        assert_eq!(
            events.lock().unwrap()[1].item.as_ref().unwrap().asset_id,
            fixture.ids[0]
        );
        assert_eq!(
            fixture.state.service.last_copy_destination().unwrap(),
            Some(std::fs::canonicalize(&fixture.destination).unwrap())
        );
    }

    // Catches retry including already-copied IDs, stale picker defaults, and showing a nonexistent folder.
    #[tokio::test]
    async fn partial_copy_retry_reopens_picker_with_existing_remembered_destination() {
        let fixture = CopyFixture::new();
        std::fs::remove_file(fixture.source.join("two.jpg")).unwrap();
        let first = super::run_original_copy(
            &fixture.state,
            None,
            |_| async { Ok(Some(fixture.destination.clone())) },
            |_| {},
        )
        .await
        .unwrap();
        let crate::dto::CopyResult::Complete { result } = first else {
            panic!("expected result")
        };
        assert_eq!((result.copied_count, result.failed_count), (1, 1));
        assert_eq!(
            fixture
                .state
                .service
                .list_photo_picks()
                .unwrap()
                .items
                .len(),
            2
        );
        let failed = result
            .items
            .into_iter()
            .filter(|item| item.status == photo_app_service::CopyItemStatus::Failed)
            .map(|item| item.asset_id)
            .collect();
        std::fs::write(fixture.source.join("two.jpg"), b"two.jpg").unwrap();
        let retried = super::run_original_copy(
            &fixture.state,
            Some(failed),
            |initial| {
                assert_eq!(
                    initial,
                    Some(std::fs::canonicalize(&fixture.destination).unwrap())
                );
                std::future::ready(Ok(Some(fixture.destination.clone())))
            },
            |_| {},
        )
        .await
        .unwrap();
        let crate::dto::CopyResult::Complete { result } = retried else {
            panic!("expected result")
        };
        assert_eq!(
            (result.copied_count, result.failed_count),
            (1, 0),
            "retry result: {result:?}"
        );
        assert_eq!(result.items[0].asset_id, fixture.ids[1]);
        assert_eq!(std::fs::read_dir(&fixture.destination).unwrap().count(), 2);
        super::show_copy_destination(&fixture.state, |path| {
            assert_eq!(path, std::fs::canonicalize(&fixture.destination).unwrap());
            Ok(())
        })
        .unwrap();
        let remembered = fixture.state.service.last_copy_destination().unwrap();
        super::run_original_copy(&fixture.state, None, |_| async { Ok(None) }, |_| {})
            .await
            .unwrap();
        assert_eq!(
            fixture.state.service.last_copy_destination().unwrap(),
            remembered
        );
        std::fs::rename(
            &fixture.destination,
            fixture.destination.with_extension("moved"),
        )
        .unwrap();
        super::run_original_copy(
            &fixture.state,
            None,
            |initial| async move {
                assert_eq!(initial, None);
                Ok(None)
            },
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(
            super::show_copy_destination(&fixture.state, |_| panic!(
                "missing folder must not open"
            ))
            .unwrap_err()
            .code,
            "copyDestinationUnavailable"
        );
    }

    #[test]
    fn copy_failures_are_bounded() {
        for (error, code) in [
            (
                AppServiceError::CopyPreparationFailed,
                "copyPreparationFailed",
            ),
            (
                AppServiceError::CopyDestinationUnavailable,
                "copyDestinationUnavailable",
            ),
            (
                AppServiceError::CopyDestinationIsSource,
                "copyDestinationIsSource",
            ),
        ] {
            assert_eq!(map_service_error(error).code, code);
        }
    }

    #[test]
    fn non_directory_selection_has_a_bounded_error() {
        let error = map_service_error(AppServiceError::OpenRecent(AddLibraryError::NotDirectory(
            PathBuf::from("/private/source-name"),
        )));
        assert_eq!(error.code, "folderNotDirectory");
        assert_eq!(error.message, "Choose a folder, not a file.");
        assert!(!error.message.contains("source-name"));
    }

    #[test]
    fn derivative_unavailable_has_a_bounded_error() {
        let error = map_service_error(AppServiceError::DerivativeUnavailable);
        assert_eq!(error.code, "derivativeUnavailable");
        assert_eq!(
            error.message,
            "Some requested previews could not be generated."
        );
    }
}

#[tauri::command]
pub fn rename_saved_folder(
    id: String,
    label: Option<String>,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .rename_saved_folder(&id, label.as_deref())
        .map_err(map_service_error)
}
#[tauri::command]
pub fn remove_saved_folder(
    id: String,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .remove_saved_folder(&id)
        .map_err(map_service_error)
}
#[tauri::command]
pub fn clear_active_folder(state: State<'_, DesktopState>) -> Result<BootstrapState, CommandError> {
    state
        .service
        .clear_active_folder()
        .map_err(map_service_error)
}
#[tauri::command]
pub async fn activate_saved_folder(
    id: String,
    state: State<'_, DesktopState>,
) -> Result<ChooseFolderResult, CommandError> {
    Ok(
        match state
            .service
            .activate_saved_folder(&id)
            .await
            .map_err(map_service_error)?
        {
            Some(state) => ChooseFolderResult::Selected { state },
            None => ChooseFolderResult::Cancelled,
        },
    )
}
#[tauri::command]
pub async fn check_saved_folders(
    ids: Vec<String>,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .check_saved_folders(&ids)
        .await
        .map_err(map_service_error)?;
    state.service.bootstrap().map_err(map_service_error)
}

#[tauri::command]
pub fn list_photo_picks(
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::PickListSnapshot, CommandError> {
    state.service.list_photo_picks().map_err(map_service_error)
}

#[tauri::command]
pub fn add_photo_pick(
    asset_id: String,
    source_folder_id: String,
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::PickListSnapshot, CommandError> {
    state
        .service
        .add_photo_pick(&asset_id, &source_folder_id)
        .map_err(map_service_error)
}

#[tauri::command]
pub fn remove_photo_pick(
    asset_id: String,
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::PickListSnapshot, CommandError> {
    state
        .service
        .remove_photo_pick(&asset_id)
        .map_err(map_service_error)
}

#[tauri::command]
pub fn clear_photo_picks(
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::PickListSnapshot, CommandError> {
    state.service.clear_photo_picks().map_err(map_service_error)
}

#[tauri::command]
pub fn restore_photo_picks(
    references: Vec<photo_app_service::PickReference>,
    state: State<'_, DesktopState>,
) -> Result<photo_app_service::PickListSnapshot, CommandError> {
    state
        .service
        .restore_photo_picks(&references)
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn request_pick_derivatives(
    request: DerivativeRequest,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    state
        .service
        .request_pick_derivatives(request)
        .await
        .map_err(map_service_error)
}
