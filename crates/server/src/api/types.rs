use photo_app_service::{InteractionState, SortDirection};
use photo_domain::GalleryScope;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateSelectionRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InteractionRequest {
    pub client_id: String,
    pub scope: GalleryScope,
    pub state: InteractionState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WallParams {
    pub scope: GalleryScope,
    pub direction: SortDirection,
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventsParams {
    pub client_id: String,
    pub scope: GalleryScope,
    pub after_event_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    pub capabilities: Capabilities,
    pub source_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub folder_browser: bool,
    pub video: bool,
}
