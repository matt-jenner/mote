use std::path::{Path, PathBuf};

use crate::{CacheBudget, ProtectedGroups};
use image::{DynamicImage, ImageReader, codecs::jpeg::JpegEncoder};
use photo_catalog::{Catalog, NewDerivative};
use photo_domain::{AssetId, DerivativeId, FileSignature, FolderGroupId};
use photo_metadata::RepresentativeRgb;

use crate::{
    CacheError, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
};

pub const DECODER_VERSION: &str = "image-0.25-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedDerivative {
    pub key: DerivativeKey,
    pub relative_path: PathBuf,
    pub size_bytes: u64,
    pub durable: bool,
    pub reused: bool,
    pub representative_rgb: RepresentativeRgb,
    pub content_type: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum ImageDerivativeError {
    #[error("source image could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("source image could not be decoded: {0}")]
    Decode(#[from] image::ImageError),
    #[error("cache operation failed: {0}")]
    Cache(#[from] CacheError),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] photo_catalog::CatalogError),
    #[error("derivative target must be a long edge")]
    UnsupportedTarget,
    #[error("screen preview generation requires budget authorization")]
    BudgetAuthorizationRequired,
    #[error("derivative specification is not canonical")]
    InvalidSpecification,
}

#[derive(Clone, Debug)]
pub struct ImageDerivativeGenerator {
    writer: CacheWriter,
}

impl ImageDerivativeGenerator {
    pub fn new(cache_root: &Path) -> Result<Self, ImageDerivativeError> {
        Ok(Self {
            writer: CacheWriter::new(cache_root)?,
        })
    }

    pub fn generate(
        &self,
        source: &Path,
        spec: &DerivativeSpec,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        let _decoder_version = DECODER_VERSION;
        validate_spec(spec)?;
        if spec.kind == DerivativeKind::ScreenPreview {
            return Err(ImageDerivativeError::BudgetAuthorizationRequired);
        }
        self.generate_allowed(source, spec)
    }

    pub fn generate_screen_preview(
        &self,
        source: &Path,
        asset_id: AssetId,
        signature: FileSignature,
        orientation: u16,
        folder_group_id: FolderGroupId,
        catalog: &mut Catalog,
        budget: CacheBudget,
        protected: &ProtectedGroups,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        let spec = DerivativeSpec {
            asset_id,
            signature,
            orientation,
            kind: DerivativeKind::ScreenPreview,
            decoder_version: DECODER_VERSION.into(),
            colour_space: "srgb".into(),
            target: DerivativeTarget::LongEdge(4096),
        };
        validate_spec(&spec)?;
        let _guard = protected.begin_write(folder_group_id)?;
        let (encoded, representative_rgb) = encode_screen(source, &spec)?;
        let estimated = encoded.len() as u64;
        budget.prepare_write(
            catalog,
            self.writer.root(),
            folder_group_id,
            estimated,
            protected,
        )?;
        let key = DerivativeKey::compute(&spec);
        let relative_path = key.sharded_path("jpg");
        let write = self.writer.write_atomic(relative_path.clone(), |file| {
            std::io::Write::write_all(file, &encoded)
        })?;
        let generated = GeneratedDerivative {
            key,
            relative_path,
            size_bytes: write.size_bytes,
            durable: false,
            reused: write.reused,
            representative_rgb,
            content_type: "image/jpeg",
        };
        catalog.upsert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id,
            folder_group_id,
            kind: "screen_preview".into(),
            cache_key: generated.key.as_str().into(),
            relative_cache_path: generated.relative_path.clone(),
            size_bytes: generated.size_bytes,
            durable: false,
            created_at: 0,
        })?;
        Ok(generated)
    }

    fn generate_allowed(
        &self,
        source: &Path,
        spec: &DerivativeSpec,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        let edge = match spec.target {
            DerivativeTarget::LongEdge(edge) if edge > 0 => edge,
            _ => return Err(ImageDerivativeError::UnsupportedTarget),
        };
        let key = DerivativeKey::compute(spec);
        let relative_path = key.sharded_path("jpg");
        let quality = match spec.kind {
            DerivativeKind::WallThumbnail => 82,
            DerivativeKind::ScreenPreview => 90,
            _ => return Err(ImageDerivativeError::UnsupportedTarget),
        };
        let durable = spec.kind == DerivativeKind::WallThumbnail;
        let image = apply_orientation(
            ImageReader::open(source)?.with_guessed_format()?.decode()?,
            spec.orientation,
        );
        let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
        let resized = resize_without_upscale(image, edge).to_rgb8();
        let mut encoded = Vec::new();
        JpegEncoder::new_with_quality(&mut encoded, quality).encode(
            &resized,
            resized.width(),
            resized.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        let write = self.writer.write_atomic(relative_path.clone(), |file| {
            std::io::Write::write_all(file, &encoded)
        })?;
        Ok(GeneratedDerivative {
            key,
            relative_path,
            size_bytes: write.size_bytes,
            durable,
            reused: write.reused,
            representative_rgb,
            content_type: "image/jpeg",
        })
    }
}

fn encode_screen(
    source: &Path,
    spec: &DerivativeSpec,
) -> Result<(Vec<u8>, RepresentativeRgb), ImageDerivativeError> {
    let image = apply_orientation(
        ImageReader::open(source)?.with_guessed_format()?.decode()?,
        spec.orientation,
    );
    let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
    let resized = resize_without_upscale(image, 4096).to_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 90).encode(
        &resized,
        resized.width(),
        resized.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok((bytes, representative_rgb))
}

fn validate_spec(spec: &DerivativeSpec) -> Result<(), ImageDerivativeError> {
    let expected = match spec.kind {
        DerivativeKind::WallThumbnail => 1024,
        DerivativeKind::ScreenPreview => 4096,
        _ => return Err(ImageDerivativeError::InvalidSpecification),
    };
    if spec.target != DerivativeTarget::LongEdge(expected)
        || spec.decoder_version != DECODER_VERSION
        || spec.colour_space != "srgb"
    {
        return Err(ImageDerivativeError::InvalidSpecification);
    }
    if !(1..=8).contains(&spec.orientation) {
        return Err(ImageDerivativeError::InvalidSpecification);
    }
    Ok(())
}

fn resize_without_upscale(image: DynamicImage, edge: u32) -> DynamicImage {
    if image.width().max(image.height()) <= edge {
        image
    } else {
        image.thumbnail(edge, edge)
    }
}

fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.fliph().rotate270(),
        6 => image.rotate90(),
        7 => image.fliph().rotate90(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn average_rgb(image: &image::RgbImage) -> RepresentativeRgb {
    let count = u64::from(image.width()) * u64::from(image.height());
    if count == 0 {
        return RepresentativeRgb {
            red: 0,
            green: 0,
            blue: 0,
        };
    }
    let sums = image.pixels().fold([0_u64; 3], |mut s, p| {
        s[0] += u64::from(p[0]);
        s[1] += u64::from(p[1]);
        s[2] += u64::from(p[2]);
        s
    });
    RepresentativeRgb {
        red: ((sums[0] + count / 2) / count) as u8,
        green: ((sums[1] + count / 2) / count) as u8,
        blue: ((sums[2] + count / 2) / count) as u8,
    }
}
