use photo_catalog::NewAsset;
use photo_domain::AssetId;
use photo_metadata::{RepresentativeRgb, ResolvedMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexEvent {
    Discovered {
        asset: NewAsset,
    },
    ShapeReady {
        asset_id: AssetId,
        width: u32,
        height: u32,
        orientation: u16,
    },
    ShapeFallback {
        asset_id: AssetId,
        width: u32,
        height: u32,
        code: &'static str,
        message: String,
    },
    ColourReady {
        asset_id: AssetId,
        representative_rgb: RepresentativeRgb,
    },
    MetadataReady {
        asset_id: AssetId,
        metadata: ResolvedMetadata,
    },
    Progress(ScanProgress),
    Warning {
        asset_id: Option<AssetId>,
        code: &'static str,
        message: String,
    },
    Completed(ScanSummary),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanStage {
    Discovering,
    Shaping,
    Enriching,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanProgress {
    pub stage: ScanStage,
    pub discovered: u64,
    pub shaped: u64,
    pub enriched: u64,
    pub direct_total: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanSummary {
    pub discovered: u64,
    pub failed: u64,
    pub cancelled: bool,
}
