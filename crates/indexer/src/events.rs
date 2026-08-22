use photo_catalog::NewAsset;
use photo_domain::AssetId;
use photo_metadata::{RepresentativeRgb, ResolvedMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexEvent {
    Discovered {
        asset: NewAsset,
    },
    Shaped {
        asset_id: AssetId,
        width: u32,
        height: u32,
        orientation: Option<u16>,
        representative_rgb: Option<RepresentativeRgb>,
    },
    MetadataReady {
        asset_id: AssetId,
        metadata: ResolvedMetadata,
    },
    Warning {
        asset_id: Option<AssetId>,
        code: &'static str,
        message: String,
    },
    Completed(ScanSummary),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanSummary {
    pub discovered: u64,
    pub failed: u64,
    pub cancelled: bool,
}
