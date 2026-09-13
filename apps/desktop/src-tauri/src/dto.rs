use serde::Serialize;

use photo_app_service::BootstrapState;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyProgress {
    pub completed: u32,
    pub total: u32,
    pub item: Option<photo_app_service::CopyItemResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CopyResult {
    SelectionCancelled,
    CopyCancelled,
    Complete {
        #[serde(flatten)]
        result: photo_app_service::OriginalCopyResult,
    },
}

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
        Self::new("internal", "Mote could not complete that request.")
    }
}
