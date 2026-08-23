use std::path::PathBuf;

use photo_domain::{AssetId, FileSignature};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivativeKind {
    WallThumbnail,
    ScreenPreview,
    RawDecode,
    DeepZoomTile,
    PosterFrame,
    VideoProxy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivativeTarget {
    LongEdge(u32),
    Tile {
        level: u16,
        x: u32,
        y: u32,
        size: u16,
    },
    Original,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivativeSpec {
    pub asset_id: AssetId,
    pub signature: FileSignature,
    pub orientation: u16,
    pub kind: DerivativeKind,
    pub decoder_version: String,
    pub colour_space: String,
    pub target: DerivativeTarget,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DerivativeKey(String);

impl DerivativeKey {
    pub fn compute(spec: &DerivativeSpec) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"photo-viewer-derivative-v1\0");
        hasher.update(spec.asset_id.as_uuid().as_bytes());
        hasher.update(&spec.signature.size_bytes.to_le_bytes());
        hasher.update(&spec.signature.modified_unix_ns.to_le_bytes());
        match spec.signature.sidecar_modified_unix_ns {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(&value.to_le_bytes());
            }
            None => {
                hasher.update(&[0]);
            }
        }
        hasher.update(&spec.orientation.to_le_bytes());
        hasher.update(&[kind_tag(spec.kind)]);
        update_string(&mut hasher, &spec.decoder_version);
        update_string(&mut hasher, &spec.colour_space);
        update_target(&mut hasher, spec.target);
        Self(hasher.finalize().to_hex().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn sharded_path(&self, extension: &str) -> PathBuf {
        let extension = extension.strip_prefix('.').unwrap_or(extension);
        PathBuf::from(&self.0[..2])
            .join(&self.0[2..4])
            .join(format!("{}.{}", self.0, extension))
    }
}

fn update_string(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn kind_tag(kind: DerivativeKind) -> u8 {
    match kind {
        DerivativeKind::WallThumbnail => 0,
        DerivativeKind::ScreenPreview => 1,
        DerivativeKind::RawDecode => 2,
        DerivativeKind::DeepZoomTile => 3,
        DerivativeKind::PosterFrame => 4,
        DerivativeKind::VideoProxy => 5,
    }
}

fn update_target(hasher: &mut blake3::Hasher, target: DerivativeTarget) {
    match target {
        DerivativeTarget::LongEdge(edge) => {
            hasher.update(&[0]);
            hasher.update(&edge.to_le_bytes());
        }
        DerivativeTarget::Tile { level, x, y, size } => {
            hasher.update(&[1]);
            hasher.update(&level.to_le_bytes());
            hasher.update(&x.to_le_bytes());
            hasher.update(&y.to_le_bytes());
            hasher.update(&size.to_le_bytes());
        }
        DerivativeTarget::Original => {
            hasher.update(&[2]);
        }
    }
}
