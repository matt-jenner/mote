use photo_app_service::{DerivativeRequest, InteractionState, SortDirection};
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
pub(crate) struct DerivativeHttpRequest {
    pub scope: GalleryScope,
    pub request: DerivativeRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveAssetsRequest {
    pub asset_ids: Vec<String>,
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
    pub root_id: Option<String>,
    pub capabilities: Capabilities,
    pub source_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub folder_browser: bool,
    pub video: bool,
    pub original_downloads: bool,
}
