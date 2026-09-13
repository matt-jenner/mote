use std::path::{Path, PathBuf};

use photo_app_service::{
    AppConfig, AppService, AppServiceError, DerivativeClass, DerivativePriority, DerivativeRequest,
    GalleryEngine, GalleryScope, PickReference, SourceAvailability,
};
use photo_catalog::{
    AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, NewDerivative, ShapeStatus,
};
use photo_domain::{AssetId, DerivativeId, FolderGroupId, MediaKind, RelativePathKey};

struct Fixture {
    _temp: tempfile::TempDir,
    config: AppConfig,
    service: AppService,
    roots: Vec<PathBuf>,
    groups: Vec<String>,
    assets: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        let mut roots = Vec::new();
        let mut groups = Vec::new();
        let mut assets = Vec::new();
        for name in ["First", "Second"] {
            let root = temp.path().join(name);
            std::fs::create_dir_all(root.join("child")).unwrap();
            image::ImageBuffer::from_pixel(3, 2, image::Rgb([40_u8, 80, 120]))
                .save(root.join("child/photo.jpg"))
                .unwrap();
            let bootstrap = service.open_recent(&root).unwrap();
            let saved = bootstrap
                .saved_folders
                .entries
                .iter()
                .find(|e| e.name == name)
                .unwrap();
            let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&saved.folder_id).unwrap());
            let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
            let library = catalog.folder_group(group).unwrap().unwrap().library_id;
            let mut asset = NewAsset::minimal(
                library,
                RelativePathKey::from_relative_path(Path::new("child/photo.jpg")).unwrap(),
                "child/photo.jpg",
                MediaKind::Jpeg,
                1,
            );
            asset.folder_group_id = Some(group);
            catalog.upsert_asset(&asset).unwrap();
            catalog
                .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: asset.id,
                    width: 3,
                    height: 2,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: ShapeStatus::Ready,
                })])
                .unwrap();
            roots.push(root);
            groups.push(saved.folder_id.clone());
            assets.push(asset.id.as_uuid().to_string());
        }
        Self {
            _temp: temp,
            config,
            service,
            roots,
            groups,
            assets,
        }
    }

    fn pick(&self, index: usize) {
        self.service
            .add_photo_pick(&self.assets[index], &self.groups[index])
            .unwrap();
    }

    fn reference(&self, index: usize) -> PickReference {
        PickReference {
            asset_id: self.assets[index].clone(),
            source_folder_id: self.groups[index].clone(),
            source_label: "Caller supplied label".into(),
        }
    }

    fn request(&self, ids: Vec<String>) -> DerivativeRequest {
        DerivativeRequest {
            asset_ids: ids,
            kind: DerivativeClass::WallThumbnail,
            priority: DerivativePriority::Visible,
        }
    }

    async fn engine(&self) -> (GalleryEngine, photo_app_service::GallerySelection) {
        let engine = GalleryEngine::open(self.config.clone(), self.roots[0].clone()).unwrap();
        let summary = engine.select_relative(Path::new(".")).await.unwrap();
        let selection = engine.resolve_selection(&summary.id).unwrap();
        (engine, selection)
    }
}

#[test]
fn resolving_picks_preserves_active_wall_order_labels_and_cached_references() {
    let fixture = Fixture::new();
    let saved = fixture
        .service
        .bootstrap()
        .unwrap()
        .saved_folders
        .entries
        .into_iter()
        .find(|e| e.folder_id == fixture.groups[0])
        .unwrap();
    fixture
        .service
        .rename_saved_folder(&saved.id, Some("Family"))
        .unwrap();
    fixture.pick(1);
    fixture.pick(0);
    fixture.pick(1);
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: AssetId::from_uuid(uuid::Uuid::parse_str(&fixture.assets[0]).unwrap()),
            folder_group_id: FolderGroupId::from_uuid(
                uuid::Uuid::parse_str(&fixture.groups[0]).unwrap(),
            ),
            kind: "wall_thumbnail".into(),
            cache_key: "cached-key".into(),
            relative_cache_path: PathBuf::from("cache/photo.jpg"),
            size_bytes: 12,
            durable: true,
            created_at: 1,
        })
        .unwrap();
    let before = fixture.service.bootstrap().unwrap().active_source;
    let picks = fixture.service.list_photo_picks().unwrap();
    assert_eq!(
        picks
            .items
            .iter()
            .map(|p| p.reference.asset_id.as_str())
            .collect::<Vec<_>>(),
        [&fixture.assets[1], &fixture.assets[0]]
    );
    assert_eq!(picks.revision, 2);
    assert_eq!(picks.items[0].reference.source_label, "Second");
    assert_eq!(picks.items[1].reference.source_label, "Family");
    assert_eq!(
        picks.items[1]
            .asset
            .as_ref()
            .unwrap()
            .wall_thumbnail
            .as_ref()
            .unwrap()
            .key,
        "cached-key"
    );
    assert_eq!(fixture.service.bootstrap().unwrap().active_source, before);
    let wire = serde_json::to_value(&picks).unwrap();
    assert_eq!(wire["items"][0]["sourceFolderId"], fixture.groups[1]);
    assert_eq!(wire["items"][0]["assetId"], fixture.assets[1]);
    assert_eq!(wire["items"][0]["sourceLabel"], "Second");
    assert!(
        !wire
            .to_string()
            .contains(fixture.roots[0].to_str().unwrap())
    );
    assert!(!wire.to_string().contains("relativeCachePath"));
}

#[test]
fn unavailable_picks_survive_reopen_and_saved_shortcut_removal() {
    let fixture = Fixture::new();
    fixture.pick(0);
    let saved = fixture
        .service
        .bootstrap()
        .unwrap()
        .saved_folders
        .entries
        .into_iter()
        .find(|e| e.folder_id == fixture.groups[0])
        .unwrap();
    fixture.service.remove_saved_folder(&saved.id).unwrap();
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&fixture.groups[0]).unwrap());
    catalog
        .mark_root_offline(catalog.folder_group(group).unwrap().unwrap().library_id)
        .unwrap();
    let reopened = AppService::open(fixture.config.clone()).unwrap();
    let picks = reopened.list_photo_picks().unwrap();
    assert_eq!(picks.items.len(), 1);
    assert_eq!(picks.items[0].reference.source_label, "First");
    assert_eq!(
        picks.items[0].asset.as_ref().unwrap().availability,
        SourceAvailability::RootOffline
    );
}

#[test]
fn mutations_validate_membership_and_restore_order_without_trusting_labels() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture
            .service
            .add_photo_pick(&fixture.assets[0], &fixture.groups[1]),
        Err(AppServiceError::ForeignAsset)
    ));
    assert!(matches!(
        fixture.service.add_photo_pick("bad", &fixture.groups[0]),
        Err(AppServiceError::InvalidAssetId)
    ));
    assert!(
        fixture
            .service
            .add_photo_pick(&fixture.assets[0], "bad")
            .is_err()
    );
    fixture.pick(0);
    let mut undo = fixture
        .service
        .list_photo_picks()
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.reference)
        .collect::<Vec<_>>();
    undo[0].source_label = "Caller supplied label".into();
    let cleared = fixture.service.clear_photo_picks().unwrap();
    assert!(cleared.items.is_empty());
    assert_eq!(cleared.revision, 2);
    assert_eq!(cleared, fixture.service.list_photo_picks().unwrap());
    fixture.pick(1);
    let restored = fixture.service.restore_photo_picks(&undo).unwrap();
    assert_eq!(
        restored
            .items
            .iter()
            .map(|p| p.reference.asset_id.as_str())
            .collect::<Vec<_>>(),
        [&fixture.assets[0], &fixture.assets[1]]
    );
    assert_eq!(restored.items[0].reference.source_label, "First");
    assert_eq!(restored.revision, 4);
    let mut foreign = fixture.reference(0);
    foreign.source_folder_id = fixture.groups[1].clone();
    assert!(matches!(
        fixture.service.restore_photo_picks(&[foreign]),
        Err(AppServiceError::ForeignAsset)
    ));
    assert!(
        fixture
            .service
            .restore_photo_picks(&vec![fixture.reference(0); 251])
            .is_err()
    );
    let mut malformed = fixture.reference(0);
    malformed.asset_id = "bad".into();
    assert!(matches!(
        fixture.service.restore_photo_picks(&[malformed]),
        Err(AppServiceError::InvalidAssetId)
    ));
    let mut disappeared = fixture.reference(0);
    disappeared.asset_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        fixture.service.restore_photo_picks(&[disappeared]).unwrap(),
        restored
    );
    assert!(fixture.service.remove_photo_pick("bad").is_err());
    let removed = fixture
        .service
        .remove_photo_pick(&fixture.assets[0])
        .unwrap();
    assert_eq!(removed.items.len(), 1);
    assert_eq!(removed.items[0].reference.asset_id, fixture.assets[1]);
}

#[tokio::test]
async fn hosted_resolution_preserves_duplicates_and_absence_without_scanning() {
    let fixture = std::thread::spawn(Fixture::new).join().unwrap();
    let before = fixture.service.bootstrap().unwrap().active_source;
    let (engine, selection) = fixture.engine().await;
    let runtime_count = engine.runtime_count_for_test();
    let absent = uuid::Uuid::new_v4().to_string();
    let resolved = engine
        .resolve_assets(
            &selection,
            &[fixture.assets[0].clone(), absent, fixture.assets[0].clone()],
        )
        .unwrap();
    assert_eq!(resolved.len(), 3);
    assert_eq!(resolved[0].as_ref().unwrap().id, fixture.assets[0]);
    assert!(resolved[1].is_none());
    assert_eq!(resolved[0], resolved[2]);
    assert_eq!(engine.runtime_count_for_test(), runtime_count);
    assert_eq!(fixture.service.bootstrap().unwrap().active_source, before);
    assert_eq!(
        engine
            .resolve_assets(&selection, &vec![fixture.assets[0].clone(); 250])
            .unwrap()
            .len(),
        250
    );
    assert!(matches!(
        engine.resolve_assets(&selection, &vec![fixture.assets[0].clone(); 251]),
        Err(AppServiceError::InvalidLimit)
    ));
    assert!(matches!(
        engine.resolve_assets(&selection, &["bad".into()]),
        Err(AppServiceError::InvalidAssetId)
    ));
    assert!(matches!(
        engine.resolve_assets(&selection, &[fixture.assets[1].clone()]),
        Err(AppServiceError::ForeignAsset)
    ));
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    catalog
        .remove_asset_membership(
            selection.group_id(),
            AssetId::from_uuid(uuid::Uuid::parse_str(&fixture.assets[0]).unwrap()),
        )
        .unwrap();
    assert!(matches!(
        engine.resolve_assets(&selection, &[fixture.assets[0].clone()]),
        Err(AppServiceError::ForeignAsset)
    ));
}

#[tokio::test]
async fn pick_derivatives_authorize_persisted_groups_with_subfolders() {
    let fixture = std::thread::spawn(Fixture::new).join().unwrap();
    fixture.pick(0);
    fixture.pick(1);
    fixture
        .service
        .update_gallery_scope(GalleryScope::CurrentFolder)
        .await
        .unwrap();
    let before = fixture.service.bootstrap().unwrap().active_source;
    fixture
        .service
        .request_pick_derivatives(fixture.request(fixture.assets.clone()))
        .await
        .unwrap();
    let picks = fixture.service.list_photo_picks().unwrap();
    for pick in picks.items {
        assert!(pick.asset.unwrap().wall_thumbnail.is_some());
    }
    assert_eq!(fixture.service.bootstrap().unwrap().active_source, before);
    fixture
        .service
        .update_gallery_scope(GalleryScope::IncludeSubfolders)
        .await
        .unwrap();
    fixture
        .service
        .request_derivatives(fixture.request(vec![fixture.assets[1].clone()]))
        .await
        .unwrap();
    fixture
        .service
        .remove_photo_pick(&fixture.assets[0])
        .unwrap();
    assert!(matches!(
        fixture
            .service
            .request_pick_derivatives(fixture.request(vec![fixture.assets[0].clone()]))
            .await,
        Err(AppServiceError::ForeignAsset)
    ));
    assert!(matches!(
        fixture
            .service
            .request_pick_derivatives(fixture.request(vec!["bad".into()]))
            .await,
        Err(AppServiceError::InvalidAssetId)
    ));
    assert!(matches!(
        fixture
            .service
            .request_pick_derivatives(fixture.request(vec![fixture.assets[1].clone(); 251]))
            .await,
        Err(AppServiceError::InvalidLimit)
    ));
}

#[tokio::test]
async fn service_shutdown_rejects_new_pick_derivative_work() {
    let fixture = std::thread::spawn(Fixture::new).join().unwrap();
    fixture.pick(0);

    fixture.service.shutdown().await;

    assert!(matches!(
        fixture
            .service
            .request_pick_derivatives(fixture.request(vec![fixture.assets[0].clone()]))
            .await,
        Err(AppServiceError::DerivativeUnavailable)
    ));
}
