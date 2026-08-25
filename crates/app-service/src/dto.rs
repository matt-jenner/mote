use photo_domain::Appearance;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection {
    OldestFirst,
    NewestFirst,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallQueryRequest {
    pub cursor: Option<String>,
    pub limit: u32,
    pub direction: SortDirection,
}

impl WallQueryRequest {
    pub fn oldest_first() -> Self {
        Self {
            cursor: None,
            limit: 50,
            direction: SortDirection::OldestFirst,
        }
    }
    pub fn newest_first() -> Self {
        Self {
            cursor: None,
            limit: 50,
            direction: SortDirection::NewestFirst,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderState {
    Provisional,
    Settled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DerivativeClass {
    WallThumbnail,
    ScreenPreview,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivativeReference {
    pub asset_id: String,
    pub kind: DerivativeClass,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallAsset {
    pub id: String,
    pub display_name: String,
    pub media_kind: WallMediaKind,
    pub provisional_order: u64,
    pub captured_at_utc: Option<String>,
    pub date_state: OrderState,
    pub width: u32,
    pub height: u32,
    pub representative_rgb: Option<u32>,
    pub shape_state: WallShapeState,
    pub availability: SourceAvailability,
    pub warning: Option<WallWarningState>,
    pub wall_thumbnail: Option<DerivativeReference>,
    pub screen_preview: Option<DerivativeReference>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WallMediaKind {
    Jpeg,
    Png,
    Tiff,
    Heif,
    Webp,
    Avif,
    Raw,
    Video,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WallShapeState {
    Ready,
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallWarningState {
    pub code: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallPage {
    pub items: Vec<WallAsset>,
    pub next_cursor: Option<String>,
    pub order_state: OrderState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DerivativePriority {
    Visible,
    NearViewport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivativeRequest {
    pub asset_ids: Vec<String>,
    pub priority: DerivativePriority,
}

impl DerivativeRequest {
    pub fn visible(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            priority: DerivativePriority::Visible,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionState {
    Idle,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WallUpdate {
    CatalogBatch {
        assets: Vec<WallAsset>,
        order_state: OrderState,
        progress: ScanProgressDto,
    },
    DerivativesReady {
        derivatives: Vec<DerivativeReference>,
    },
    MetadataSettled {
        source_id: String,
    },
    Progress {
        progress: ScanProgressDto,
    },
    SourceUnavailable {
        source_id: String,
    },
    Warning {
        source_id: String,
        asset_id: Option<String>,
        warning: WallWarningState,
    },
    WarningCleared {
        source_id: String,
        asset_id: Option<String>,
        code: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressDto {
    pub discovered: u64,
    pub shaped: u64,
    pub enriched: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub settings: SettingsState,
    pub active_source: Option<SourceSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsState {
    pub appearance: Appearance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: String,
    pub display_name: String,
    pub availability: SourceAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceAvailability {
    Available,
    RootOffline,
    Missing,
    Unreadable,
}
