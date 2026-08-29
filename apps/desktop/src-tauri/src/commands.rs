use photo_app_service::{
    AppServiceError, BootstrapState, DerivativeRequest, InteractionState, WallQueryRequest,
    WallUpdate,
};
use photo_core::AddLibraryError;
use photo_domain::{Appearance, GalleryScope};
use tauri::{AppHandle, State, Theme, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::dto::{ChooseFolderResult, CommandError};
use crate::protocol::forward_wall_updates_for_service;
use crate::state::{DesktopState, WallSubscriptionId};

#[tauri::command]
pub fn get_bootstrap_state(state: State<'_, DesktopState>) -> Result<BootstrapState, CommandError> {
    state.service.bootstrap().map_err(map_service_error)
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
    let state = state
        .service
        .open_recent(&path)
        .map_err(map_service_error)?;
    Ok(ChooseFolderResult::Selected { state })
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
pub fn update_gallery_scope(
    scope: GalleryScope,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .update_gallery_scope(scope)
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
            "That folder overlaps Photo Viewer's local data.",
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
        AppServiceError::LocalState(_) => CommandError::new(
            "localStateUnavailable",
            "Photo Viewer cannot open its local data.",
        ),
        _ => CommandError::internal(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use photo_app_service::AppServiceError;
    use photo_core::AddLibraryError;

    use super::map_service_error;

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
