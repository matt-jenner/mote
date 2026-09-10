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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallPreviewCounts {
    pub wall_ready: u64,
    pub screen_ready: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallPage {
    pub items: Vec<WallAsset>,
    pub next_cursor: Option<String>,
    pub order_state: OrderState,
    pub source_warnings: Vec<WallWarningState>,
    pub total_count: u64,
    pub preview_counts: WallPreviewCounts,
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
        #[serde(rename = "previewCounts")]
        preview_counts: Option<WallPreviewCounts>,
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
    pub direct_total: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub saved_folders: SavedFolderSnapshot,
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
    use super::{
        ScanProgressDto, WallAsset, WallPage, WallPreviewCounts, WallUpdate, WallWarningState,
    };

    #[test]
    fn wall_page_and_derivative_events_serialize_scope_wide_preview_counts() {
        let counts = WallPreviewCounts {
            wall_ready: 1033,
            screen_ready: 149,
        };
        let page = WallPage {
            items: Vec::new(),
            next_cursor: None,
            order_state: OrderState::Settled,
            source_warnings: Vec::new(),
            total_count: 2092,
            preview_counts: counts,
        };
        let page_value = serde_json::to_value(page).unwrap();
        assert_eq!(page_value["previewCounts"]["wallReady"], 1033);
        assert_eq!(page_value["previewCounts"]["screenReady"], 149);

        let update = WallUpdate::DerivativesReady {
            selection_id: "selection-opaque".to_owned(),
            derivatives: Vec::new(),
            preview_counts: Some(counts),
        };
        let update_value = serde_json::to_value(update).unwrap();
        assert_eq!(update_value["previewCounts"], page_value["previewCounts"]);
    }

    #[test]
    fn wall_updates_serialize_selection_identity_and_generation() {
        let update = WallUpdate::Progress {
            selection_id: "selection-opaque".to_owned(),
            generation: 7,
            progress: ScanProgressDto {
                discovered: 1,
                shaped: 1,
                enriched: 0,
                direct_total: Some(1),
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
            total_count: 0,
            preview_counts: WallPreviewCounts::default(),
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFolder {
    pub id: String,
    pub folder_id: String,
    pub name: String,
    pub display_path: String,
    pub custom_label: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderAccessState {
    Unknown,
    Checking,
    Available,
    Missing,
    Unreadable,
    RootOffline,
    Unverified,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderAccess {
    pub folder_id: String,
    pub state: FolderAccessState,
    pub generation: u64,
    pub retry_after_ms: u64,
}
impl FolderAccess {
    pub fn from_reply(folder_id: String, reply: &crate::AccessReply) -> Self {
        use crate::{AccessReply, FolderProbeOutcome};
        let (state, retry_after_ms) = match reply {
            AccessReply::Checking { .. } => (FolderAccessState::Checking, 1000),
            AccessReply::Complete {
                outcome,
                retry_after_ms,
                ..
            } => (
                match outcome {
                    FolderProbeOutcome::Available(_) => FolderAccessState::Available,
                    FolderProbeOutcome::Missing => FolderAccessState::Missing,
                    FolderProbeOutcome::Unreadable => FolderAccessState::Unreadable,
                    FolderProbeOutcome::RootOffline => FolderAccessState::RootOffline,
                    _ => FolderAccessState::Unverified,
                },
                *retry_after_ms,
            ),
        };
        Self {
            folder_id,
            state,
            generation: reply.generation(),
            retry_after_ms,
        }
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFolderSnapshot {
    pub revision: u64,
    pub entries: Vec<SavedFolder>,
    pub access: std::collections::HashMap<String, FolderAccess>,
    pub active_entry_id: Option<String>,
    pub has_opened_folder: bool,
    pub persistence_error: Option<String>,
}
