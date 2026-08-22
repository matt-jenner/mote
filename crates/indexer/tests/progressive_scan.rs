use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use photo_catalog::{Catalog, NewAsset, NewLibrary};
use photo_core::FolderPolicyEngine;
use photo_domain::{MediaKind, RelativePathKey};
use photo_indexer::{CatalogWriter, IndexEvent, Indexer, MetadataReader, ScanRequest};
use photo_metadata::{
    Keyword, MetadataBundle, MetadataReadWarning, ProvenanceRecord, RepresentativeRgb,
    ResolvedMetadata,
};

#[derive(Clone, Default)]
struct NoopMetadataReader;

impl MetadataReader for NoopMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        Ok(MetadataBundle::default())
    }
}

#[derive(Clone)]
struct BlockingMetadataReader {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingMetadataReader {
    fn new() -> (Self, ReleaseMetadata) {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        (Self { gate: gate.clone() }, ReleaseMetadata { gate })
    }
}

impl MetadataReader for BlockingMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        let (lock, changed) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
        Ok(MetadataBundle::default())
    }
}

struct ReleaseMetadata {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl ReleaseMetadata {
    fn release(self) {
        let (lock, changed) = &*self.gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

fn write_png(path: &Path, rgb: [u8; 3]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_raw(1, 1, rgb.to_vec())
        .unwrap()
        .save(path)
        .unwrap();
}

fn empty_policy_engine() -> FolderPolicyEngine {
    FolderPolicyEngine::new(Vec::new()).unwrap()
}

#[tokio::test]
async fn emits_discovery_before_blocked_metadata_reader_finishes() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("a.png"), [255, 0, 0]);
    write_png(&fixture.path().join("b.png"), [0, 0, 255]);
    let (reader, release) = BlockingMetadataReader::new();
    let indexer = Indexer::new(reader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();

    let first = scan.events.recv().await.unwrap();

    assert!(matches!(first, IndexEvent::Discovered { .. }));
    release.release();
    let summary = scan.join().await.unwrap();
    assert_eq!(summary.discovered, 2);
    assert_eq!(summary.failed, 0);
}

#[tokio::test]
async fn corrupt_media_warns_without_preventing_valid_metadata() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("valid.png"), [1, 2, 3]);
    std::fs::write(fixture.path().join("broken.jpg"), b"not a jpeg").unwrap();
    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let mut warned = false;
    let mut metadata_ready = false;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::Warning {
                code: "shape_read_failed",
                ..
            } => warned = true,
            IndexEvent::MetadataReady { .. } => metadata_ready = true,
            IndexEvent::Completed(_) => break,
            _ => {}
        }
    }

    let summary = scan.join().await.unwrap();

    assert!(warned);
    assert!(metadata_ready);
    assert_eq!(summary.discovered, 2);
    assert_eq!(summary.failed, 1);
}

#[tokio::test]
async fn cancellation_stops_a_large_scan_at_a_bounded_partial_result() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..1_000 {
        write_png(&fixture.path().join(format!("{index:04}.png")), [1, 2, 3]);
    }
    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    assert!(matches!(
        scan.events.recv().await,
        Some(IndexEvent::Discovered { .. })
    ));

    scan.cancel().unwrap();
    let summary = scan.join().await.unwrap();

    assert!(summary.cancelled);
    assert!(summary.discovered < 1_000);
}

#[test]
fn catalog_writer_commits_discovery_shape_and_metadata_together() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap();
    let asset = NewAsset::minimal(library.id, relative, "a.jpg", MediaKind::Jpeg, 10);
    let resolved = ResolvedMetadata {
        rating: Some(4),
        keywords: vec![Keyword {
            normalized: "family".to_owned(),
            display_value: "Family".to_owned(),
            hierarchy: None,
        }],
        provenance: vec![ProvenanceRecord {
            field: "rating".to_owned(),
            source: photo_metadata::MetadataSource::SidecarXmp,
            raw_value: "4".to_owned(),
            chosen: true,
        }],
        ..ResolvedMetadata::default()
    };
    let events = vec![
        IndexEvent::Discovered {
            asset: asset.clone(),
        },
        IndexEvent::Shaped {
            asset_id: asset.id,
            width: 100,
            height: 50,
            orientation: Some(6),
            representative_rgb: Some(RepresentativeRgb {
                red: 1,
                green: 2,
                blue: 3,
            }),
        },
        IndexEvent::MetadataReady {
            asset_id: asset.id,
            metadata: resolved,
        },
    ];

    CatalogWriter::new(&mut catalog, library.id)
        .apply_batch(&events)
        .unwrap();

    let stored = catalog.find_asset(asset.id).unwrap().unwrap();
    assert_eq!((stored.width, stored.height), (Some(100), Some(50)));
    assert_eq!(stored.rating, Some(4));
    assert_eq!(catalog.asset_keywords(asset.id).unwrap(), vec!["Family"]);
}

#[test]
fn filename_sidecar_wins_over_stem_sidecar_case_insensitively() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("Photo.JPG");
    std::fs::write(&media, b"data").unwrap();
    std::fs::write(fixture.path().join("PHOTO.JPG.XMP"), b"filename").unwrap();
    std::fs::write(fixture.path().join("photo.xmp"), b"stem").unwrap();

    let found = photo_indexer::find_sidecar(&media).unwrap();

    assert_eq!(found, Some(fixture.path().join("PHOTO.JPG.XMP")));
}
