use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notify::event::{DataChange, EventAttributes, ModifyKind};
use notify::{Event, EventKind};
use photo_catalog::{Catalog, NewLibrary};
use photo_domain::{Availability, LibraryId};
use photo_indexer::{ChangeHint, RealReconcileSource, ReconcileOutcome, Reconciler, WatchService};

struct Harness {
    _temp: tempfile::TempDir,
    root: PathBuf,
    offline_root: PathBuf,
    library_id: LibraryId,
    reconciler: Reconciler<RealReconcileSource>,
}

fn harness(files: &[&str]) -> Harness {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Photos");
    let offline_root = temp.path().join("Photos-offline");
    std::fs::create_dir_all(&root).unwrap();
    for file in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"media").unwrap();
    }
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", &root))
        .unwrap();
    let reconciler = Reconciler::new(catalog, library.id, root.clone(), RealReconcileSource);
    Harness {
        _temp: temp,
        root,
        offline_root,
        library_id: library.id,
        reconciler,
    }
}

#[tokio::test]
async fn offline_root_marks_assets_unavailable_without_deleting_them() {
    let mut harness = harness(&["a.jpg", "b.jpg"]);
    harness.reconciler.run().await.unwrap();
    std::fs::rename(&harness.root, &harness.offline_root).unwrap();

    let outcome = harness.reconciler.run().await.unwrap();

    assert_eq!(
        outcome,
        ReconcileOutcome::RootOffline { retained_assets: 2 }
    );
    assert_eq!(
        harness
            .reconciler
            .catalog()
            .asset_count(harness.library_id)
            .unwrap(),
        2
    );
    assert!(
        harness
            .reconciler
            .catalog()
            .assets(harness.library_id)
            .unwrap()
            .iter()
            .all(|asset| asset.availability == Availability::RootOffline)
    );
}

#[tokio::test]
async fn readable_root_marks_only_unseen_assets_missing() {
    let mut harness = harness(&["a.jpg", "b.jpg"]);
    harness.reconciler.run().await.unwrap();
    std::fs::remove_file(harness.root.join("b.jpg")).unwrap();

    let outcome = harness.reconciler.run().await.unwrap();

    assert!(matches!(
        outcome,
        ReconcileOutcome::Completed {
            observed: 1,
            marked_missing: 1,
            ..
        }
    ));
    let assets = harness
        .reconciler
        .catalog()
        .assets(harness.library_id)
        .unwrap();
    assert_eq!(assets.len(), 2);
    assert_eq!(
        assets
            .iter()
            .find(|asset| asset.display_path == "b.jpg")
            .unwrap()
            .availability,
        Availability::Missing
    );
}

#[tokio::test]
async fn periodic_reconciliation_finds_a_file_without_a_watcher_event() {
    let mut harness = harness(&["a.jpg"]);
    harness.reconciler.run().await.unwrap();
    std::fs::write(harness.root.join("new.jpg"), b"new").unwrap();

    let outcome = harness.reconciler.run().await.unwrap();

    assert!(matches!(
        outcome,
        ReconcileOutcome::Completed { observed: 2, .. }
    ));
    assert_eq!(
        harness
            .reconciler
            .catalog()
            .asset_count(harness.library_id)
            .unwrap(),
        2
    );
}

#[test]
fn notify_events_map_to_hints_and_errors_request_a_root_rescan() {
    let root = Path::new("/Photos");
    let changed = root.join("a.jpg");
    let event = Event {
        kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
        paths: vec![changed.clone()],
        attrs: EventAttributes::default(),
    };

    assert_eq!(
        WatchService::map_notify(Ok(event), root),
        vec![ChangeHint::PathChanged(changed)]
    );
    assert_eq!(
        WatchService::map_notify(Err(notify::Error::generic("lost events")), root),
        vec![ChangeHint::RescanRoot(root.to_path_buf())]
    );
}

#[test]
fn repeated_path_hints_are_coalesced_for_250_milliseconds() {
    let root = PathBuf::from("/Photos");
    let path = root.join("a.jpg");
    let mut service = WatchService::new(root);
    let start = Instant::now();
    service.record(ChangeHint::PathChanged(path.clone()), start);
    service.record(
        ChangeHint::PathChanged(path.clone()),
        start + Duration::from_millis(100),
    );

    assert!(
        service
            .drain_ready(start + Duration::from_millis(349))
            .is_empty()
    );
    assert_eq!(
        service.drain_ready(start + Duration::from_millis(350)),
        vec![ChangeHint::PathChanged(path)]
    );
}
