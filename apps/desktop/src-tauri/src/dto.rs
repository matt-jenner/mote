use serde::Serialize;

use photo_app_service::BootstrapState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ChooseFolderResult {
    Cancelled,
    Selected { state: BootstrapState },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: &'static str,
    pub message: &'static str,
}

impl CommandError {
    pub const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    pub const fn internal() -> Self {
        Self::new("internal", "Photo Viewer could not complete that request.")
    }
}
