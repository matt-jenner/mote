use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::{CacheBudget, ProtectedGroups};
use image::{DynamicImage, ImageReader, codecs::jpeg::JpegEncoder};
use photo_catalog::{Catalog, NewDerivative};
use photo_domain::{AssetId, DerivativeId, FileSignature, FolderGroupId};
use photo_metadata::RepresentativeRgb;

use crate::{
    CacheError, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
};

pub const DECODER_VERSION: &str = "image-0.25-v1";

fn record_timing_stage(stage: &'static str, started: Instant) {
    tracing::debug!(
        target: "photo_viewer::timing",
        stage,
        elapsed_nanos = started.elapsed().as_nanos() as u64,
        "derivative_stage"
    );
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedScreenPreview {
    pub bytes: Vec<u8>,
    pub representative_rgb: RepresentativeRgb,
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
    screen_transaction_lock: Arc<Mutex<()>>,
}

static SCREEN_TRANSACTION_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    OnceLock::new();

fn transaction_lock(root: &Path) -> Arc<Mutex<()>> {
    let locks = SCREEN_TRANSACTION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .expect("screen transaction lock registry must not be poisoned");
    locks
        .entry(root.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

impl ImageDerivativeGenerator {
    pub fn new(cache_root: &Path) -> Result<Self, ImageDerivativeError> {
        let writer = CacheWriter::new(cache_root)?;
        let screen_transaction_lock = transaction_lock(writer.root());
        Ok(Self {
            writer,
            screen_transaction_lock,
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

    /// Rebuilds the durable wall thumbnail from an already cached screen preview.
    ///
    /// Screen previews have already had source orientation applied, so this path only
    /// downscales and re-encodes the cached pixels. The source media is never opened.
    pub fn generate_wall_thumbnail_from_cached_preview(
        &self,
        cached_relative_path: &Path,
        spec: &DerivativeSpec,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        validate_spec(spec)?;
        if spec.kind != DerivativeKind::WallThumbnail {
            return Err(ImageDerivativeError::UnsupportedTarget);
        }
        let edge = match spec.target {
            DerivativeTarget::LongEdge(edge) if edge > 0 => edge,
            _ => return Err(ImageDerivativeError::UnsupportedTarget),
        };
        let read_started = Instant::now();
        let bytes = self.writer.read_checked(cached_relative_path)?;
        let image = image::load_from_memory(&bytes)?;
        record_timing_stage("cached_preview_read_decode", read_started);
        let transform_started = Instant::now();
        let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
        let resized = resize_without_upscale(image, edge).to_rgb8();
        let mut encoded = Vec::new();
        JpegEncoder::new_with_quality(&mut encoded, 82).encode(
            &resized,
            resized.width(),
            resized.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        record_timing_stage("cached_preview_transform_encode", transform_started);

        let key = DerivativeKey::compute(spec);
        let relative_path = key.sharded_path("jpg");
        let write_started = Instant::now();
        let write_result = self.writer.write_atomic(relative_path.clone(), |file| {
            std::io::Write::write_all(file, &encoded)
        });
        record_timing_stage("managed_cache_write", write_started);
        let write = write_result?;
        Ok(GeneratedDerivative {
            key,
            relative_path,
            size_bytes: write.size_bytes,
            durable: true,
            reused: write.reused,
            representative_rgb,
            content_type: "image/jpeg",
        })
    }

    #[allow(clippy::too_many_arguments)]
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
        let encoded = self.encode_screen_preview(source, &spec)?;
        self.commit_screen_preview(encoded, &spec, folder_group_id, catalog, budget, protected)
    }

    pub fn encode_screen_preview(
        &self,
        source: &Path,
        spec: &DerivativeSpec,
    ) -> Result<EncodedScreenPreview, ImageDerivativeError> {
        validate_spec(spec)?;
        if spec.kind != DerivativeKind::ScreenPreview {
            return Err(ImageDerivativeError::UnsupportedTarget);
        }
        let (bytes, representative_rgb) = encode_screen(source, spec)?;
        Ok(EncodedScreenPreview {
            bytes,
            representative_rgb,
        })
    }

    /// Encodes a wall thumbnail without writing it to the cache. Callers that
    /// need to fence source identity before publication use this paired with
    /// `commit_wall_thumbnail`.
    pub fn encode_wall_thumbnail(
        &self,
        source: &Path,
        spec: &DerivativeSpec,
    ) -> Result<EncodedScreenPreview, ImageDerivativeError> {
        validate_spec(spec)?;
        if spec.kind != DerivativeKind::WallThumbnail {
            return Err(ImageDerivativeError::UnsupportedTarget);
        }
        let image = ImageReader::open(source)?.with_guessed_format()?.decode()?;
        let image = apply_orientation(image, spec.orientation);
        let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
        let resized = resize_without_upscale(image, 1024).to_rgb8();
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 82).encode(
            &resized,
            resized.width(),
            resized.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        Ok(EncodedScreenPreview {
            bytes,
            representative_rgb,
        })
    }

    /// Writes a previously encoded wall thumbnail under its immutable key.
    /// The caller is responsible for checking source identity immediately
    /// before this operation and for catalog publication afterward.
    pub fn commit_wall_thumbnail(
        &self,
        encoded: EncodedScreenPreview,
        spec: &DerivativeSpec,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        validate_spec(spec)?;
        if spec.kind != DerivativeKind::WallThumbnail {
            return Err(ImageDerivativeError::UnsupportedTarget);
        }
        let key = DerivativeKey::compute(spec);
        let relative_path = key.sharded_path("jpg");
        let write = self.writer.replace_atomic(relative_path.clone(), |file| {
            std::io::Write::write_all(file, &encoded.bytes)
        })?;
        Ok(GeneratedDerivative {
            key,
            relative_path,
            size_bytes: write.size_bytes,
            durable: true,
            reused: write.reused,
            representative_rgb: encoded.representative_rgb,
            content_type: "image/jpeg",
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_screen_preview(
        &self,
        encoded: EncodedScreenPreview,
        spec: &DerivativeSpec,
        folder_group_id: FolderGroupId,
        catalog: &mut Catalog,
        budget: CacheBudget,
        protected: &ProtectedGroups,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        self.commit_screen_preview_inner(
            encoded,
            spec,
            folder_group_id,
            catalog,
            budget,
            protected,
            false,
        )
    }

    /// Commits a screen preview while replacing bytes under an already-known
    /// immutable key. Hosted repair uses this path after validating that the
    /// existing row's bytes are corrupt; ordinary callers retain the normal
    /// immutable reuse behavior of [`Self::commit_screen_preview`].
    #[allow(clippy::too_many_arguments)]
    pub fn commit_screen_preview_repairing(
        &self,
        encoded: EncodedScreenPreview,
        spec: &DerivativeSpec,
        folder_group_id: FolderGroupId,
        catalog: &mut Catalog,
        budget: CacheBudget,
        protected: &ProtectedGroups,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        self.commit_screen_preview_inner(
            encoded,
            spec,
            folder_group_id,
            catalog,
            budget,
            protected,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_screen_preview_inner(
        &self,
        encoded: EncodedScreenPreview,
        spec: &DerivativeSpec,
        folder_group_id: FolderGroupId,
        catalog: &mut Catalog,
        budget: CacheBudget,
        protected: &ProtectedGroups,
        replace_existing: bool,
    ) -> Result<GeneratedDerivative, ImageDerivativeError> {
        validate_spec(spec)?;
        if spec.kind != DerivativeKind::ScreenPreview {
            return Err(ImageDerivativeError::UnsupportedTarget);
        }
        let _transaction = self
            .screen_transaction_lock
            .lock()
            .map_err(|_| ImageDerivativeError::Cache(CacheError::TransactionUnavailable))?;
        let _guard = protected.begin_write(folder_group_id)?;
        let estimated = u64::try_from(encoded.bytes.len())
            .map_err(|_| ImageDerivativeError::Cache(CacheError::SizeOutOfRange))?;
        budget.prepare_write(
            catalog,
            self.writer.root(),
            folder_group_id,
            estimated,
            protected,
        )?;
        let key = DerivativeKey::compute(spec);
        let relative_path = key.sharded_path("jpg");
        let write_started = Instant::now();
        let write_result = if replace_existing {
            self.writer.replace_atomic(relative_path.clone(), |file| {
                std::io::Write::write_all(file, &encoded.bytes)
            })
        } else {
            self.writer.write_atomic(relative_path.clone(), |file| {
                std::io::Write::write_all(file, &encoded.bytes)
            })
        };
        record_timing_stage("managed_cache_write", write_started);
        let write = write_result?;
        let generated = GeneratedDerivative {
            key,
            relative_path,
            size_bytes: write.size_bytes,
            durable: false,
            reused: write.reused,
            representative_rgb: encoded.representative_rgb,
            content_type: "image/jpeg",
        };
        let catalog_started = Instant::now();
        let catalog_result = catalog.upsert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: spec.asset_id,
            folder_group_id,
            kind: "screen_preview".into(),
            cache_key: generated.key.as_str().into(),
            relative_cache_path: generated.relative_path.clone(),
            size_bytes: generated.size_bytes,
            durable: false,
            created_at: 0,
        });
        record_timing_stage("catalog_commit", catalog_started);
        catalog_result?;
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
        let decode_started = Instant::now();
        let image = (|| {
            Ok::<_, ImageDerivativeError>(
                ImageReader::open(source)?.with_guessed_format()?.decode()?,
            )
        })();
        record_timing_stage("source_read_decode", decode_started);
        let image = image?;
        let transform_started = Instant::now();
        let transformed = (|| {
            let image = apply_orientation(image, spec.orientation);
            let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
            let resized = resize_without_upscale(image, edge).to_rgb8();
            let mut encoded = Vec::new();
            JpegEncoder::new_with_quality(&mut encoded, quality).encode(
                &resized,
                resized.width(),
                resized.height(),
                image::ExtendedColorType::Rgb8,
            )?;
            Ok::<_, ImageDerivativeError>((encoded, representative_rgb))
        })();
        record_timing_stage("transform_encode", transform_started);
        let (encoded, representative_rgb) = transformed?;
        let write_started = Instant::now();
        let write_result = self.writer.write_atomic(relative_path.clone(), |file| {
            std::io::Write::write_all(file, &encoded)
        });
        record_timing_stage("managed_cache_write", write_started);
        let write = write_result?;
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
    let decode_started = Instant::now();
    let image = (|| {
        Ok::<_, ImageDerivativeError>(ImageReader::open(source)?.with_guessed_format()?.decode()?)
    })();
    record_timing_stage("source_read_decode", decode_started);
    let image = image?;
    let transform_started = Instant::now();
    let transformed = (|| {
        let image = apply_orientation(image, spec.orientation);
        let representative_rgb = average_rgb(&image.thumbnail(32, 32).to_rgb8());
        let resized = resize_without_upscale(image, 4096).to_rgb8();
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 90).encode(
            &resized,
            resized.width(),
            resized.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        Ok::<_, ImageDerivativeError>((bytes, representative_rgb))
    })();
    record_timing_stage("transform_encode", transform_started);
    transformed
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
