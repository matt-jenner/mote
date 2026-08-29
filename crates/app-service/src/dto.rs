use photo_domain::{Appearance, GalleryScope};
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
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
    pub rating: Option<u8>,
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
    pub source_warnings: Vec<WallWarningState>,
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
    pub kind: DerivativeClass,
}

impl DerivativeRequest {
    pub fn visible(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            priority: DerivativePriority::Visible,
            kind: DerivativeClass::WallThumbnail,
        }
    }

    pub fn visible_screen_preview(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            priority: DerivativePriority::Visible,
            kind: DerivativeClass::ScreenPreview,
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
        #[serde(rename = "selectionId")]
        selection_id: String,
        assets: Vec<WallAsset>,
        #[serde(rename = "orderState")]
        order_state: OrderState,
        #[serde(rename = "generation")]
        generation: u64,
        progress: ScanProgressDto,
    },
    DerivativesReady {
        #[serde(rename = "selectionId")]
        selection_id: String,
        derivatives: Vec<DerivativeReference>,
    },
    MetadataSettled {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "sourceId")]
        source_id: String,
        #[serde(rename = "generation")]
        generation: u64,
    },
    Progress {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "generation")]
        generation: u64,
        progress: ScanProgressDto,
    },
    SourceUnavailable {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "sourceId")]
        source_id: String,
    },
    Warning {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "sourceId")]
        source_id: String,
        #[serde(rename = "assetId")]
        asset_id: Option<String>,
        warning: WallWarningState,
    },
    WarningCleared {
        #[serde(rename = "selectionId")]
        selection_id: String,
        #[serde(rename = "sourceId")]
        source_id: String,
        #[serde(rename = "assetId")]
        asset_id: Option<String>,
        code: String,
    },
    ResyncRequired {
        #[serde(rename = "selectionId")]
        selection_id: String,
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
    pub gallery_scope: GalleryScope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: String,
    pub selection_id: String,
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

#[cfg(test)]
mod tests {
    use super::{OrderState, SourceAvailability, WallMediaKind, WallShapeState};
    use super::{ScanProgressDto, WallAsset, WallPage, WallUpdate, WallWarningState};

    #[test]
    fn wall_updates_serialize_selection_identity_and_generation() {
        let update = WallUpdate::Progress {
            selection_id: "selection-opaque".to_owned(),
            generation: 7,
            progress: ScanProgressDto {
                discovered: 1,
                shaped: 1,
                enriched: 0,
                total: Some(1),
            },
        };
        let value = serde_json::to_value(update).unwrap();
        assert_eq!(value["kind"], "progress");
        assert_eq!(value["selectionId"], "selection-opaque");
        assert_eq!(value["generation"], 7);
    }

    #[test]
    fn wall_page_serializes_path_free_source_warning_snapshot() {
        let page = WallPage {
            items: Vec::new(),
            next_cursor: None,
            order_state: super::OrderState::Provisional,
            source_warnings: vec![WallWarningState {
                code: "screenPreviewCacheUnavailable".to_owned(),
                retryable: true,
            }],
        };
        let value = serde_json::to_value(page).unwrap();
        assert_eq!(
            value["sourceWarnings"][0]["code"],
            "screenPreviewCacheUnavailable"
        );
        assert!(!value.to_string().contains('/'));
    }

    #[test]
    fn wall_asset_serializes_rating_without_source_paths() {
        let value = serde_json::to_value(WallAsset {
            id: "00000000-0000-0000-0000-000000000001".to_owned(),
            display_name: "photo.jpg".to_owned(),
            media_kind: WallMediaKind::Jpeg,
            provisional_order: 1,
            captured_at_utc: Some("2026-08-26T12:00:00Z".to_owned()),
            date_state: OrderState::Settled,
            width: 2048,
            height: 1365,
            representative_rgb: Some(0x334455),
            shape_state: WallShapeState::Ready,
            availability: SourceAvailability::Available,
            warning: None,
            wall_thumbnail: None,
            screen_preview: None,
            rating: Some(4),
        })
        .unwrap();
        assert_eq!(value["rating"], 4);
        assert!(!value.to_string().contains("/photos/"));
    }
}
