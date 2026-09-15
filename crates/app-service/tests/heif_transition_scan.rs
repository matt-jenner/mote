use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use photo_app_service::{
    AppConfig, GalleryEngine, GalleryScope, GallerySelection, MetadataReader, WallUpdate,
};
use photo_indexer::DefaultMetadataReader;
use photo_metadata::{MetadataBundle, MetadataReadWarning};
use rusqlite::Connection;

#[derive(Default)]
struct Reader {
    paths: Mutex<Vec<PathBuf>>,
    gate: Mutex<bool>,
    wake: Condvar,
    entered: tokio::sync::Notify,
}

impl Reader {
    #[cfg(feature = "heic")]
    fn release(&self) {
        *self.gate.lock().unwrap() = false;
        self.wake.notify_all();
    }
}

impl MetadataReader for Reader {
    fn read(
        &self,
        path: &Path,
        sidecar: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.paths.lock().unwrap().push(path.to_owned());
        if path
            .extension()
            .is_some_and(|extension| extension == "heic")
        {
            self.entered.notify_one();
            let guard = self.gate.lock().unwrap();
            drop(
                self.wake
                    .wait_timeout_while(guard, Duration::from_secs(10), |blocked| *blocked)
                    .unwrap(),
            );
        }
        DefaultMetadataReader.read(path, sidecar)
    }
}

async fn selection(engine: &GalleryEngine, path: &str) -> GallerySelection {
    let summary = engine.select_relative(Path::new(path)).await.unwrap();
    engine.resolve_selection(&summary.id).unwrap()
}

async fn scan(engine: &GalleryEngine, selection: &GallerySelection) {
    let mut events = engine.subscribe(
        selection,
        "scan".into(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    engine.ensure_running(selection).await.unwrap();
    wait_settled(&mut events).await;
}

async fn wait_settled(events: &mut photo_app_service::SelectionEventSubscription) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if matches!(
                events.recv().await.unwrap().update,
                WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
}

fn copy_heif(directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../codec/tests/fixtures/heif/portrait-rotated.heic"),
        directory.join("portrait.heic"),
    )
    .unwrap();
}

fn reset_capability(config: &AppConfig) {
    let db = Connection::open(config.catalog_path()).unwrap();
    db.execute("UPDATE folder_groups SET heif_metadata_revision = 0", [])
        .unwrap();
}

#[cfg(feature = "heic")]
#[tokio::test]
async fn transition_admission_returns_while_metadata_is_blocked() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    copy_heif(&source.join("heif"));
    std::fs::create_dir_all(source.join("jpeg")).unwrap();
    image::RgbImage::from_pixel(3, 2, image::Rgb([20, 40, 60]))
        .save(source.join("jpeg/photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    scan(&seed, &selection(&seed, "heif").await).await;
    scan(&seed, &selection(&seed, "jpeg").await).await;
    drop(seed);
    reset_capability(&config);
    let reader = Arc::new(Reader::default());
    *reader.gate.lock().unwrap() = true;
    let engine = GalleryEngine::open_with_reader(config, source, reader.clone()).unwrap();
    let heif = selection(&engine, "heif").await;
    let mut events = engine.subscribe(
        &heif,
        "completion".into(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    let result =
        tokio::time::timeout(Duration::from_millis(500), engine.ensure_running(&heif)).await;
    if result.is_err() {
        reader.release();
    }
    assert!(
        result.is_ok(),
        "request admission waited for HEIF metadata work"
    );
    result.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(3), reader.entered.notified())
        .await
        .unwrap();
    let jpeg = selection(&engine, "jpeg").await;
    let unrelated =
        tokio::time::timeout(Duration::from_millis(500), engine.ensure_running(&jpeg)).await;
    reader.release();
    assert!(
        unrelated.is_ok(),
        "HEIF transition blocked an unrelated completed selection"
    );
    unrelated.unwrap().unwrap();
    drop(engine);
    wait_settled(&mut events).await;
    assert_eq!(
        reader.paths.lock().unwrap().len(),
        1,
        "the completed JPEG-only group must not rescan"
    );
}

fn metadata(config: &AppConfig) -> (u32, u32, Option<String>) {
    Connection::open(config.catalog_path()).unwrap().query_row(
        "SELECT width, height, captured_at_utc FROM assets WHERE display_path = 'portrait.heic'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()
}

// Run disabled then enabled with the same MOTE_METADATA_TRANSITION_ROOT to
// test a real cross-binary reopen. Normal enabled runs seed historical values.
#[tokio::test]
async fn persisted_disabled_catalog_transitions_through_one_background_scan() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let shared = std::env::var_os("MOTE_METADATA_TRANSITION_ROOT").map(PathBuf::from);
    let root = shared.as_deref().unwrap_or(temp.path());
    let source = root.join("photos");
    let config = AppConfig::new(root.join("data"), root.join("cache"));
    if !config.catalog_path().exists() {
        copy_heif(&source);
        std::fs::write(source.join("portrait.heic.xmp"), br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreateDate="2028-01-02T03:04:05Z"/>"#).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/truncated.heic"),
            source.join("truncated.heic"),
        )
        .unwrap();
        image::RgbImage::from_pixel(3, 2, image::Rgb([20, 40, 60]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
        scan(&engine, &selection(&engine, ".").await).await;
        drop(engine);
        if cfg!(feature = "heic") {
            reset_capability(&config);
            let db = Connection::open(config.catalog_path()).unwrap();
            db.execute("UPDATE assets SET width = 4, height = 3, captured_at_utc = '2028-01-02T03:04:05+00:00' WHERE display_path = 'portrait.heic'", []).unwrap();
            db.execute("INSERT INTO warnings (library_id, asset_id, code, message, occurred_at) SELECT library_id, id, 'image_decode_failed', 'disabled', 1 FROM assets WHERE display_path = 'portrait.heic'", []).unwrap();
        }
    }
    let before = metadata(&config);
    assert_eq!(before, (4, 3, Some("2028-01-02T03:04:05+00:00".into())));
    let snapshots = [
        "portrait.heic",
        "portrait.heic.xmp",
        "truncated.heic",
        "photo.jpg",
    ]
    .map(|name| {
        let path = source.join(name);
        let stat = path.metadata().unwrap();
        (
            path.clone(),
            Sha256::digest(std::fs::read(path).unwrap()),
            stat.len(),
            stat.modified().unwrap(),
            stat.permissions().readonly(),
        )
    });
    let db = Connection::open(config.catalog_path()).unwrap();
    let initial_generation: i64 = db
        .query_row("SELECT MAX(generation) FROM scan_generations", [], |row| {
            row.get(0)
        })
        .unwrap();
    let schema: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    for restart in 0..2 {
        let reader = Arc::new(Reader::default());
        let engine =
            GalleryEngine::open_with_reader(config.clone(), source.clone(), reader.clone())
                .unwrap();
        let selected = selection(&engine, ".").await;
        if cfg!(feature = "heic") && restart == 0 {
            scan(&engine, &selected).await;
        } else {
            engine.ensure_running(&selected).await.unwrap();
        }
        assert_eq!(
            reader.paths.lock().unwrap().len(),
            if cfg!(feature = "heic") && restart == 0 {
                3
            } else {
                0
            }
        );
        assert_eq!(
            metadata(&config),
            if cfg!(feature = "heic") {
                (100, 28, Some("2024-03-04T05:06:07+00:00".into()))
            } else {
                before.clone()
            }
        );
        assert_eq!(
            db.query_row("SELECT MAX(generation) FROM scan_generations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            initial_generation + i64::from(cfg!(feature = "heic"))
        );
    }
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        schema
    );
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        i64::from(cfg!(feature = "heic"))
    );
    if cfg!(feature = "heic") {
        assert_eq!(db.query_row("SELECT COUNT(*) FROM warnings WHERE asset_id IN (SELECT id FROM assets WHERE display_path = 'portrait.heic') AND code = 'image_decode_failed'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
    for (path, hash, length, modified, readonly) in snapshots {
        let stat = path.metadata().unwrap();
        assert_eq!(Sha256::digest(std::fs::read(&path).unwrap()), hash);
        assert_eq!(
            (
                stat.len(),
                stat.modified().unwrap(),
                stat.permissions().readonly()
            ),
            (length, modified, readonly)
        );
    }
}

#[cfg(feature = "heic")]
struct VanishingReader;

#[cfg(all(feature = "heic", unix))]
#[derive(Debug, Eq, PartialEq)]
struct CachedMetadata {
    shape_and_date: (u32, u32, Option<String>),
    rating: Option<i64>,
    keywords: Vec<(String, String, String)>,
    provenance: Vec<(String, String, String, bool)>,
}

#[cfg(all(feature = "heic", unix))]
fn cached_metadata(config: &AppConfig) -> CachedMetadata {
    let db = Connection::open(config.catalog_path()).unwrap();
    CachedMetadata {
        shape_and_date: metadata(config),
        rating: db.query_row("SELECT rating FROM assets WHERE display_path = 'portrait.heic'", [], |row| row.get(0)).unwrap(),
        keywords: db.prepare("SELECT normalized, display_value, hierarchy FROM asset_keywords ORDER BY normalized, hierarchy").unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap(),
        provenance: db.prepare("SELECT field_name, source_kind, raw_value, chosen FROM metadata_provenance ORDER BY field_name, source_kind, raw_value").unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap().collect::<Result<_, _>>().unwrap(),
    }
}

#[cfg(all(feature = "heic", unix))]
struct LostSidecarReader {
    failure: &'static str,
}

#[cfg(all(feature = "heic", unix))]
impl MetadataReader for LostSidecarReader {
    fn read(
        &self,
        path: &Path,
        sidecar: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        use std::os::unix::fs::PermissionsExt;
        let sidecar = sidecar.unwrap();
        match self.failure {
            "missing" => {
                std::fs::rename(sidecar, sidecar.with_extension("away")).unwrap();
            }
            "unreadable" => {
                std::fs::set_permissions(sidecar, std::fs::Permissions::from_mode(0o000)).unwrap();
            }
            "read_error" => {
                std::fs::rename(sidecar, sidecar.with_extension("away")).unwrap();
                std::fs::create_dir(sidecar).unwrap();
            }
            _ => unreachable!(),
        }
        DefaultMetadataReader.read(path, Some(sidecar))
    }
}

#[cfg(all(feature = "heic", unix))]
#[tokio::test]
async fn failed_heif_sidecar_read_preserves_all_cached_metadata_until_clean_retry() {
    use std::os::unix::fs::PermissionsExt;
    for failure in ["missing", "unreadable", "read_error"] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        copy_heif(&source);
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/iphone-8bit.heic"),
            source.join("portrait.heic"),
        )
        .unwrap();
        let sidecar = source.join("portrait.heic.xmp");
        std::fs::write(&sidecar, br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmp:Rating="4" xmp:CreateDate="2030-01-02T03:04:05Z"><dc:subject><rdf:Bag><rdf:li>Family</rdf:li></rdf:Bag></dc:subject></rdf:Description>"#).unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
        scan(&seed, &selection(&seed, ".").await).await;
        drop(seed);
        let before = cached_metadata(&config);
        assert_eq!(
            before.shape_and_date.2.as_deref(),
            Some("2030-01-02T03:04:05+00:00")
        );
        assert_eq!(before.rating, Some(4));
        assert_eq!(
            before.keywords,
            vec![("family".into(), "Family".into(), "".into())]
        );
        assert!(
            before
                .provenance
                .iter()
                .any(|(field, _, value, chosen)| field == "rating" && value == "4" && *chosen)
        );
        reset_capability(&config);
        let engine = GalleryEngine::open_with_reader(
            config.clone(),
            source.clone(),
            Arc::new(LostSidecarReader { failure }),
        )
        .unwrap();
        scan(&engine, &selection(&engine, ".").await).await;
        drop(engine);
        assert_eq!(
            cached_metadata(&config),
            before,
            "{failure} sidecar destroyed cached HEIF metadata"
        );
        let db = Connection::open(config.catalog_path()).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT heif_metadata_revision FROM folder_groups",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        match failure {
            "unreadable" => {
                std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o600)).unwrap()
            }
            "read_error" => {
                std::fs::remove_dir(&sidecar).unwrap();
                std::fs::rename(sidecar.with_extension("away"), &sidecar).unwrap();
            }
            _ => std::fs::rename(sidecar.with_extension("away"), &sidecar).unwrap(),
        }
        let reader = Arc::new(Reader::default());
        let engine =
            GalleryEngine::open_with_reader(config.clone(), source, reader.clone()).unwrap();
        scan(&engine, &selection(&engine, ".").await).await;
        assert_eq!(reader.paths.lock().unwrap().len(), 1);
        assert_eq!(cached_metadata(&config), before);
        assert_eq!(
            db.query_row(
                "SELECT heif_metadata_revision FROM folder_groups",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}

#[cfg(feature = "heic")]
struct IndeterminateReader;

#[cfg(all(feature = "heic", unix))]
struct ReadTimeIoReader;

#[cfg(all(feature = "heic", unix))]
impl MetadataReader for ReadTimeIoReader {
    fn read(
        &self,
        path: &Path,
        sidecar: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        std::fs::rename(path, path.with_extension("away")).unwrap();
        std::fs::create_dir(path).unwrap();
        // Unix open/stat succeed on this path, but the real codec's read fails.
        std::fs::File::open(path).unwrap().metadata().unwrap();
        DefaultMetadataReader.read(path, sidecar)
    }
}

#[cfg(all(feature = "heic", unix))]
#[tokio::test]
async fn heif_read_time_io_preserves_cached_metadata_when_access_checks_succeed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    copy_heif(&source);
    std::fs::write(source.join("portrait.heic.xmp"), br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmp:Rating="4" xmp:CreateDate="2030-01-02T03:04:05Z"><dc:subject><rdf:Bag><rdf:li>Family</rdf:li></rdf:Bag></dc:subject></rdf:Description>"#).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    scan(&seed, &selection(&seed, ".").await).await;
    drop(seed);
    let before = cached_metadata(&config);
    assert_eq!(
        before.shape_and_date.2.as_deref(),
        Some("2024-03-04T05:06:07+00:00")
    );
    assert_eq!(before.rating, Some(4));
    reset_capability(&config);
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source.clone(), Arc::new(ReadTimeIoReader))
            .unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    drop(engine);
    assert_eq!(
        cached_metadata(&config),
        before,
        "typed codec I/O was swallowed as empty EXIF"
    );
    let db = Connection::open(config.catalog_path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT availability FROM assets", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "available"
    );
    std::fs::remove_dir(source.join("portrait.heic")).unwrap();
    std::fs::rename(source.join("portrait.away"), source.join("portrait.heic")).unwrap();
    let reader = Arc::new(Reader::default());
    let engine = GalleryEngine::open_with_reader(config.clone(), source, reader.clone()).unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    assert_eq!(reader.paths.lock().unwrap().len(), 1);
    assert_eq!(cached_metadata(&config), before);
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[cfg(feature = "heic")]
impl MetadataReader for IndeterminateReader {
    fn read(&self, _: &Path, _: Option<&Path>) -> Result<MetadataBundle, MetadataReadWarning> {
        Err(MetadataReadWarning::new(
            "source_check_failed",
            "temporary device I/O failure",
        ))
    }
}

#[cfg(feature = "heic")]
#[tokio::test]
async fn indeterminate_metadata_failure_stays_pending_without_changing_availability() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    copy_heif(&source);
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    scan(&seed, &selection(&seed, ".").await).await;
    drop(seed);
    let before = metadata(&config);
    reset_capability(&config);
    let engine = GalleryEngine::open_with_reader(
        config.clone(),
        source.clone(),
        Arc::new(IndeterminateReader),
    )
    .unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    drop(engine);
    assert_eq!(metadata(&config), before);
    let db = Connection::open(config.catalog_path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT availability FROM assets WHERE display_path = 'portrait.heic'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "available"
    );
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "indeterminate metadata failure was certified"
    );
    assert_eq!(db.query_row("SELECT heif_metadata_retry_required FROM scan_generations ORDER BY generation DESC LIMIT 1", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    let reader = Arc::new(Reader::default());
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source.clone(), reader.clone()).unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    assert_eq!(reader.paths.lock().unwrap().len(), 1);
    assert_eq!(metadata(&config), before);
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(engine);
    let reader = Arc::new(Reader::default());
    let engine = GalleryEngine::open_with_reader(config.clone(), source, reader.clone()).unwrap();
    assert_eq!(db.query_row("SELECT heif_metadata_retry_required FROM scan_generations ORDER BY generation DESC LIMIT 1", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    engine
        .ensure_running(&selection(&engine, ".").await)
        .await
        .unwrap();
    assert!(reader.paths.lock().unwrap().is_empty());
}

#[cfg(feature = "heic")]
impl MetadataReader for VanishingReader {
    fn read(
        &self,
        path: &Path,
        sidecar: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        std::fs::rename(path, path.with_extension("away")).unwrap();
        DefaultMetadataReader.read(path, sidecar)
    }
}

#[cfg(feature = "heic")]
#[tokio::test]
async fn transient_missing_file_preserves_metadata_and_retries_next_enabled_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    copy_heif(&source);
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    scan(&seed, &selection(&seed, ".").await).await;
    drop(seed);
    let before = metadata(&config);
    reset_capability(&config);
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source.clone(), Arc::new(VanishingReader))
            .unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    drop(engine);
    assert_eq!(
        metadata(&config),
        before,
        "transient file loss destroyed known metadata"
    );
    let db = Connection::open(config.catalog_path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    std::fs::rename(source.join("portrait.away"), source.join("portrait.heic")).unwrap();
    let reader = Arc::new(Reader::default());
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source.clone(), reader.clone()).unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    assert_eq!(reader.paths.lock().unwrap().len(), 1);
    assert_eq!(metadata(&config), before);
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[cfg(feature = "heic")]
#[tokio::test]
async fn offline_transition_retains_cached_metadata_and_retries_when_root_returns() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    copy_heif(&source);
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let seed = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let selected = selection(&seed, ".").await;
    scan(&seed, &selected).await;
    let id = selected.id().to_owned();
    drop(seed);
    let before = metadata(&config);
    reset_capability(&config);
    let offline = temp.path().join("offline");
    std::fs::rename(&source, &offline).unwrap();
    let reader = Arc::new(Reader::default());
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source.clone(), reader.clone()).unwrap();
    let selected = engine.resolve_selection(&id).unwrap();
    engine.ensure_running(&selected).await.unwrap();
    assert!(reader.paths.lock().unwrap().is_empty());
    assert_eq!(metadata(&config), before);
    let db = Connection::open(config.catalog_path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    std::fs::rename(&offline, &source).unwrap();
    // The source-availability coordinator reports the remount before the
    // gallery admits recovery, as in the existing hosted recovery tests.
    photo_catalog::Catalog::open(&config.catalog_path())
        .unwrap()
        .set_library_availability(selected.library_id(), photo_domain::Availability::Available)
        .unwrap();
    drop(engine);
    let engine = GalleryEngine::open_with_reader(config.clone(), source, reader.clone()).unwrap();
    scan(&engine, &selection(&engine, ".").await).await;
    assert_eq!(reader.paths.lock().unwrap().len(), 1);
    assert_eq!(metadata(&config), before);
    assert_eq!(
        db.query_row(
            "SELECT heif_metadata_revision FROM folder_groups",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
