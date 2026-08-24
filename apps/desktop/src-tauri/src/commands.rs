use photo_app_service::{AppServiceError, BootstrapState};
use photo_core::AddLibraryError;
use photo_domain::Appearance;
use tauri::{AppHandle, State, Theme, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::dto::{ChooseFolderResult, CommandError};
use crate::state::DesktopState;

#[tauri::command]
pub fn get_bootstrap_state(state: State<'_, DesktopState>) -> Result<BootstrapState, CommandError> {
    state
        .service
        .lock()
        .map_err(|_| CommandError::internal())?
        .bootstrap()
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
    let state = state
        .service
        .lock()
        .map_err(|_| CommandError::internal())?
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
        .lock()
        .map_err(|_| CommandError::internal())?
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
}
