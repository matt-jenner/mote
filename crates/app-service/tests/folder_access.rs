use photo_app_service::{
    AccessReply, FolderAccessCoordinator, FolderAccessKey, FolderAccessTarget, FolderProbe,
    FolderProbeOutcome,
};
use photo_domain::{LibraryId, RelativePathKey};
use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn target() -> FolderAccessTarget {
    FolderAccessTarget {
        key: FolderAccessKey {
            library_id: LibraryId::new(),
            relative: RelativePathKey::from_relative_path(Path::new("Family")).unwrap(),
        },
        root: Path::new("/unused").to_path_buf(),
    }
}

struct Probe {
    calls: Arc<AtomicUsize>,
    gate: Option<Arc<(Mutex<bool>, Condvar)>>,
}
impl FolderProbe for Probe {
    fn probe(&self, _: &FolderAccessTarget) -> FolderProbeOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            let mut released = gate.0.lock().unwrap();
            while !*released {
                released = gate.1.wait(released).unwrap();
            }
        }
        FolderProbeOutcome::Missing
    }
}

#[tokio::test]
async fn simultaneous_clients_share_a_completed_result_and_cooldown() {
    let calls = Arc::new(AtomicUsize::new(0));
    let coordinator = FolderAccessCoordinator::new(Arc::new(Probe {
        calls: calls.clone(),
        gate: None,
    }));
    let target = target();
    let (first, second) = tokio::join!(
        coordinator.check(target.clone()),
        coordinator.check(target.clone())
    );
    assert!(matches!(
        first,
        AccessReply::Complete {
            outcome: FolderProbeOutcome::Missing,
            ..
        }
    ));
    assert_eq!(first.generation(), second.generation());
    coordinator.check(target.clone()).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::time::resume();
    coordinator.check(target).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn timeout_keeps_the_running_slot_until_filesystem_work_finishes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let coordinator = FolderAccessCoordinator::new(Arc::new(Probe {
        calls: calls.clone(),
        gate: Some(gate.clone()),
    }));
    let target = target();
    let running = tokio::spawn({
        let coordinator = coordinator.clone();
        let target = target.clone();
        async move { coordinator.check(target).await }
    });
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(matches!(
        running.await.unwrap(),
        AccessReply::Checking { .. }
    ));
    let again = coordinator.check(target.clone()).await;
    assert!(matches!(again, AccessReply::Checking { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    tokio::time::resume();
    for _ in 0..100 {
        if matches!(
            coordinator.check(target.clone()).await,
            AccessReply::Complete { .. }
        ) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("completed filesystem check never published its result");
}

#[tokio::test]
async fn separate_folder_keys_can_check_independently() {
    let calls = Arc::new(AtomicUsize::new(0));
    let coordinator = FolderAccessCoordinator::new(Arc::new(Probe {
        calls: calls.clone(),
        gate: None,
    }));
    let (first, second) = tokio::join!(coordinator.check(target()), coordinator.check(target()));
    assert!(matches!(first, AccessReply::Complete { .. }));
    assert!(matches!(second, AccessReply::Complete { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
