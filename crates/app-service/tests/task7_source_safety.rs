use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use photo_app_service::{
    AppConfig, AppService, DerivativeClass, DerivativePriority, DerivativeRequest, SortDirection,
    WallMediaKind, WallQueryRequest, WallUpdate,
};
use photo_catalog::Catalog;
use photo_domain::AssetId;

#[derive(Debug, Eq, PartialEq)]
struct SourceEntry {
    relative_path: PathBuf,
    size: u64,
    modified: (i64, u32),
    digest: [u8; 32],
}

#[derive(Debug, Eq, PartialEq)]
struct SourceSnapshot {
    entries: Vec<SourceEntry>,
    content_digest: [u8; 32],
}

fn demo_source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("app-service has a workspace parent")
        .parent()
        .expect("workspace has a repository parent")
        .join("apps/interface/public/demo-photos")
}

fn source_snapshot(root: &Path) -> SourceSnapshot {
    fn visit(root: &Path, directory: &Path, entries: &mut Vec<SourceEntry>) {
        let mut children = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read source directory {directory:?}: {error}"))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("read source directory {directory:?}: {error}"));
        children.sort_by_key(|entry| entry.path());
        for entry in children {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("stat source entry {path:?}: {error}"));
            if file_type.is_dir() {
                visit(root, &path, entries);
                continue;
            }
            assert!(
                file_type.is_file(),
                "source fixture must contain regular files only: {path:?}"
            );
            let metadata = entry
                .metadata()
                .unwrap_or_else(|error| panic!("stat source file {path:?}: {error}"));
            let modified = metadata
                .modified()
                .unwrap_or_else(|error| panic!("read source mtime {path:?}: {error}"))
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|error| panic!("source mtime predates epoch {path:?}: {error}"));
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("read source fixture {path:?}: {error}"));
            entries.push(SourceEntry {
                relative_path: path
                    .strip_prefix(root)
                    .unwrap_or_else(|error| panic!("source path escaped root: {path:?}: {error}"))
                    .to_owned(),
                size: metadata.len(),
                modified: (
                    i64::try_from(modified.as_secs()).expect("source mtime fits i64"),
                    modified.subsec_nanos(),
                ),
                digest: *blake3::hash(&bytes).as_bytes(),
            });
        }
    }

    assert!(
        root.is_dir(),
        "controlled source fixture is missing: {root:?}"
    );
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    let mut hasher = blake3::Hasher::new();
    for entry in &entries {
        hasher.update(entry.relative_path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(&entry.size.to_be_bytes());
        hasher.update(&entry.digest);
    }
    SourceSnapshot {
        entries,
        content_digest: *hasher.finalize().as_bytes(),
    }
}

fn query(direction: SortDirection, cursor: Option<String>, limit: u32) -> WallQueryRequest {
    WallQueryRequest {
        cursor,
        limit,
        direction,
    }
}

async fn wait_for_metadata_settled(receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if matches!(
                receiver.recv().await,
                Ok(WallUpdate::MetadataSettled { .. })
            ) {
                return;
            }
        }
    })
    .await
    .expect("controlled source scan did not settle");
}

async fn wait_for_derivatives(
    receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
    class: DerivativeClass,
    expected: &HashSet<String>,
) {
    let mut seen = HashSet::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(WallUpdate::DerivativesReady { derivatives, .. }) = receiver.recv().await {
                seen.extend(
                    derivatives
                        .into_iter()
                        .filter(|derivative| derivative.kind == class)
                        .map(|derivative| derivative.asset_id),
                );
                if expected.is_subset(&seen) {
                    return;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for {class:?} derivatives: expected {expected:?}, seen {seen:?}")
    });
}

fn assert_photo_page(page: &photo_app_service::WallPage, expected: usize) {
    assert_eq!(page.items.len(), expected);
    assert!(page.items.iter().all(|asset| {
        asset.media_kind != WallMediaKind::Video && asset.display_name.ends_with(".jpg")
    }));
}

async fn run_task7_source_safety_sequence() {
    let source = demo_source_root();
    let before = source_snapshot(&source);
    assert_eq!(
        before.entries.len(),
        6,
        "demo source fixture row count changed"
    );

    let temp = tempfile::tempdir().unwrap();
    let config = AppConfig::new(temp.path().join("catalog"), temp.path().join("cache"));
    assert!(config.data_dir().starts_with(temp.path()));
    assert!(config.cache_dir().starts_with(temp.path()));

    let current = {
        let service = AppService::open(config.clone()).unwrap();
        let mut updates = service.subscribe_wall_updates();
        service.start_scan(&source).await.unwrap();
        wait_for_metadata_settled(&mut updates).await;
        assert_eq!(
            service.active_scan_count_test(),
            0,
            "metadata settlement must leave no active scan"
        );

        let first_page = service
            .query_wall(query(SortDirection::OldestFirst, None, 3))
            .await
            .unwrap();
        assert_photo_page(&first_page, 3);
        let second_page = service
            .query_wall(query(
                SortDirection::OldestFirst,
                first_page.next_cursor.clone(),
                3,
            ))
            .await
            .unwrap();
        assert_photo_page(&second_page, 3);
        assert!(
            first_page
                .items
                .iter()
                .map(|asset| &asset.id)
                .collect::<HashSet<_>>()
                .is_disjoint(
                    &second_page
                        .items
                        .iter()
                        .map(|asset| &asset.id)
                        .collect::<HashSet<_>>(),
                )
        );
        let newest_page = service
            .query_wall(query(SortDirection::NewestFirst, None, 100))
            .await
            .unwrap();
        assert_photo_page(&newest_page, 6);
        assert_photo_page(
            &service
                .query_wall(query(SortDirection::OldestFirst, None, 100))
                .await
                .unwrap(),
            6,
        );

        let all_ids = newest_page
            .items
            .iter()
            .map(|asset| asset.id.clone())
            .collect::<Vec<_>>();
        let current = all_ids[0].clone();
        let neighbours = all_ids[1..].to_vec();
        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;

        let current_set = HashSet::from([current.clone()]);
        service
            .request_derivatives(DerivativeRequest::visible(vec![current.clone()]))
            .await
            .unwrap();
        wait_for_derivatives(&mut updates, DerivativeClass::WallThumbnail, &current_set).await;

        let neighbour_set = neighbours.iter().cloned().collect::<HashSet<_>>();
        service
            .request_derivatives(DerivativeRequest {
                asset_ids: neighbours,
                priority: DerivativePriority::NearViewport,
                kind: DerivativeClass::WallThumbnail,
            })
            .await
            .unwrap();
        wait_for_derivatives(&mut updates, DerivativeClass::WallThumbnail, &neighbour_set).await;

        let all_set = all_ids.iter().cloned().collect::<HashSet<_>>();
        service
            .request_derivatives(DerivativeRequest::visible(all_ids.clone()))
            .await
            .unwrap();
        wait_for_derivatives(&mut updates, DerivativeClass::WallThumbnail, &all_set).await;
        service.wait_for_derivative_tasks_quiescent_test().await;
        assert!(
            service.collection_driver_active_count_test() > 0,
            "the active interaction should leave a tracked collection driver awaiting idle"
        );

        service
            .request_derivatives(DerivativeRequest::visible_screen_preview(all_ids.clone()))
            .await
            .unwrap();
        wait_for_derivatives(&mut updates, DerivativeClass::ScreenPreview, &all_set).await;
        service.wait_for_derivative_tasks_quiescent_test().await;

        let current_asset = AssetId::from_uuid(uuid::Uuid::parse_str(&current).unwrap());
        {
            let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
            let asset = catalog.find_asset(current_asset).unwrap().unwrap();
            let wall = catalog
                .derivatives_for_assets(&[current_asset], "wall_thumbnail")
                .unwrap()
                .into_iter()
                .next()
                .expect("full thumbnail phase should persist a wall row");
            assert!(
                catalog
                    .derivatives_for_assets(&[current_asset], "screen_preview")
                    .unwrap()
                    .into_iter()
                    .next()
                    .is_some(),
                "full preview phase should persist a screen row"
            );
            assert_eq!(catalog.delete_derivatives(&[wall.id]).unwrap(), 1);
            catalog.mark_root_offline(asset.library_id).unwrap();
        }

        let mut legacy_updates = service.subscribe_wall_updates();
        service
            .request_derivatives(DerivativeRequest::visible_screen_preview(vec![
                current.clone(),
            ]))
            .await
            .unwrap();
        wait_for_derivatives(
            &mut legacy_updates,
            DerivativeClass::WallThumbnail,
            &current_set,
        )
        .await;
        wait_for_derivatives(
            &mut legacy_updates,
            DerivativeClass::ScreenPreview,
            &current_set,
        )
        .await;
        let repaired = service
            .query_wall(query(SortDirection::OldestFirst, None, 100))
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|asset| asset.id == current)
            .expect("legacy-repaired asset should stay on the wall");
        assert!(repaired.wall_thumbnail.is_some());
        assert!(repaired.screen_preview.is_some());
        service.wait_for_derivative_tasks_quiescent_test().await;

        drop(legacy_updates);
        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        service.wait_for_derivative_tasks_quiescent_test().await;
        service.wait_for_collection_drivers_quiescent_test().await;
        assert_eq!(service.derivative_active_task_count_test(), 0);
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert_eq!(service.active_scan_count_test(), 0);
        drop(updates);
        current
    };

    {
        let reopen_config = config.clone();
        let reopened = std::thread::spawn(move || AppService::open(reopen_config))
            .join()
            .expect("service restart thread panicked")
            .unwrap();
        let offline_page = reopened
            .query_wall(query(SortDirection::NewestFirst, None, 100))
            .await
            .unwrap();
        assert_photo_page(&offline_page, 6);
        assert!(
            offline_page
                .items
                .iter()
                .all(|asset| { asset.wall_thumbnail.is_some() && asset.screen_preview.is_some() })
        );
        reopened
            .request_derivatives(DerivativeRequest::visible_screen_preview(vec![
                current.clone(),
            ]))
            .await
            .unwrap();
        reopened.wait_for_derivative_tasks_quiescent_test().await;
        reopened.wait_for_collection_drivers_quiescent_test().await;
        assert_eq!(reopened.derivative_active_task_count_test(), 0);
        assert_eq!(reopened.collection_driver_active_count_test(), 0);
        assert_eq!(reopened.active_scan_count_test(), 0);
    }

    let after = source_snapshot(&source);
    assert_eq!(
        before, after,
        "service sequence changed the controlled source tree"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task7_source_tree_safety_harness() {
    run_task7_source_safety_sequence().await;
}
