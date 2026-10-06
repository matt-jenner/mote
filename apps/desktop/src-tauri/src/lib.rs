use std::env::VarError;

use photo_app_service::{AppConfig, AppService, AppServiceError};
use photo_domain::Appearance;
use profile::{ProfileError, ProfileRoots};
use protocol::handle_derivative_request;
use state::DesktopState;
use tauri::{Manager, Theme};

mod commands;
mod dto;
mod profile;
mod protocol;
mod state;

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("PHOTO_VIEWER_PROFILE is not valid Unicode")]
    InvalidProfileUnicode,
    #[error("the main application window is unavailable")]
    MissingMainWindow,
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Service(#[from] AppServiceError),
    #[error("could not apply the native window theme")]
    NativeTheme(#[source] tauri::Error),
}

pub fn run() {
    let _ = tracing_subscriber::fmt::try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_opener::init())
        .register_uri_scheme_protocol("photo-derivative", |context, request| {
            let state = context.app_handle().state::<DesktopState>();
            handle_derivative_request(&state.service, request)
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let profile = match std::env::var("PHOTO_VIEWER_PROFILE") {
                Ok(profile) => Some(profile),
                Err(VarError::NotPresent) => None,
                Err(VarError::NotUnicode(_)) => {
                    return Err(StartupError::InvalidProfileUnicode.into());
                }
            };
            let roots = ProfileRoots::from_bases(&data_dir, &cache_dir, profile.as_deref())?;
            let service = AppService::open(AppConfig::new(roots.data_dir, roots.cache_dir))?;
            let appearance = service.bootstrap()?.settings.appearance;
            let window = app
                .get_webview_window("main")
                .ok_or(StartupError::MissingMainWindow)?;
            let theme = match appearance {
                Appearance::System => None,
                Appearance::Light => Some(Theme::Light),
                Appearance::Dark => Some(Theme::Dark),
            };
            window.set_theme(theme).map_err(StartupError::NativeTheme)?;
            app.manage(DesktopState {
                service,
                wall_subscriptions: Default::default(),
                copy_operation: Default::default(),
                copy_registry: Default::default(),
                last_completed_copy_destination: Default::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::list_photo_picks,
            commands::add_photo_pick,
            commands::remove_photo_pick,
            commands::clear_photo_picks,
            commands::restore_photo_picks,
            commands::request_pick_derivatives,
            commands::copy_picked_originals,
            commands::cancel_original_copy,
            commands::show_last_copy_destination,
            commands::choose_folder,
            commands::rename_saved_folder,
            commands::remove_saved_folder,
            commands::clear_active_folder,
            commands::activate_saved_folder,
            commands::check_saved_folders,
            commands::update_appearance,
            commands::update_gallery_scope,
            commands::query_wall,
            commands::request_derivatives,
            commands::set_wall_interaction,
            commands::watch_wall_updates,
            commands::unwatch_wall_updates
        ])
        .build(tauri::generate_context!())
        .expect("error while building Mote")
        .run(|app, event| {
            on_run_event(&event, || {
                if let Some(state) = app.try_state::<DesktopState>() {
                    tauri::async_runtime::block_on(state.service.shutdown());
                }
            });
        });
}

fn on_run_event(event: &tauri::RunEvent, shutdown: impl FnOnce()) {
    if matches!(event, tauri::RunEvent::Exit) {
        shutdown();
    }
}

#[cfg(test)]
mod shutdown_tests {
    #[test]
    fn native_exit_invokes_shutdown_only_for_the_final_exit_event() {
        let mut shutdowns = 0;
        super::on_run_event(&tauri::RunEvent::Ready, || shutdowns += 1);
        assert_eq!(shutdowns, 0);
        super::on_run_event(&tauri::RunEvent::Exit, || shutdowns += 1);
        assert_eq!(shutdowns, 1);
    }
}
