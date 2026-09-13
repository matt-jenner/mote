use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use photo_app_service::AppService;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WallSubscriptionId(pub String);

#[derive(Clone, Default)]
pub struct WallSubscriptionRegistry {
    next_id: Arc<AtomicU64>,
    cancellations: Arc<Mutex<HashMap<WallSubscriptionId, watch::Sender<bool>>>>,
}

impl WallSubscriptionRegistry {
    pub fn register(&self) -> (WallSubscriptionId, watch::Receiver<bool>) {
        let id = WallSubscriptionId(format!(
            "wall-subscription-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        ));
        let (sender, receiver) = watch::channel(false);
        self.cancellations
            .lock()
            .expect("wall subscription registry lock is not poisoned")
            .insert(id.clone(), sender);
        (id, receiver)
    }

    pub fn cancel(&self, id: &WallSubscriptionId) -> bool {
        let sender = self
            .cancellations
            .lock()
            .expect("wall subscription registry lock is not poisoned")
            .remove(id);
        sender
            .map(|sender| {
                let _ = sender.send(true);
                true
            })
            .unwrap_or(false)
    }

    pub fn finish(&self, id: &WallSubscriptionId) {
        self.cancellations
            .lock()
            .expect("wall subscription registry lock is not poisoned")
            .remove(id);
    }

    #[cfg(test)]
    pub fn active_count(&self) -> usize {
        self.cancellations
            .lock()
            .expect("wall subscription registry lock is not poisoned")
            .len()
    }
}

#[derive(Clone, Default)]
pub struct CopyOperationRegistry {
    next_id: Arc<AtomicU64>,
    active: Arc<Mutex<Option<CopyOperationEntry>>>,
}

struct CopyOperationEntry {
    id: u64,
    cancellation: photo_app_service::CopyCancellation,
    finished: watch::Sender<bool>,
}

pub struct CopyOperation {
    pub id: u64,
    pub cancellation: photo_app_service::CopyCancellation,
    registry: CopyOperationRegistry,
}

impl Drop for CopyOperation {
    fn drop(&mut self) {
        self.registry.finish(self.id);
    }
}

impl CopyOperationRegistry {
    /// The caller retains the exclusive copy guard until this operation drops.
    pub fn begin(&self) -> CopyOperation {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = photo_app_service::CopyCancellation::default();
        let (finished, _) = watch::channel(false);
        let mut active = self
            .active
            .lock()
            .expect("copy registry lock is not poisoned");
        assert!(
            active.is_none(),
            "the copy guard permits only one operation"
        );
        *active = Some(CopyOperationEntry {
            id,
            cancellation: cancellation.clone(),
            finished,
        });
        CopyOperation {
            id,
            cancellation,
            registry: self.clone(),
        }
    }

    /// Capture this generation and its completion signal under one short lock.
    pub fn cancel(&self) -> Option<watch::Receiver<bool>> {
        let active = self
            .active
            .lock()
            .expect("copy registry lock is not poisoned");
        active.as_ref().map(|entry| {
            entry.cancellation.cancel();
            entry.finished.subscribe()
        })
    }

    pub fn finish(&self, id: u64) {
        let mut active = self
            .active
            .lock()
            .expect("copy registry lock is not poisoned");
        if active.as_ref().is_some_and(|entry| entry.id == id) {
            let entry = active.take().expect("matched operation");
            let _ = entry.finished.send(true);
        }
    }
}

pub struct DesktopState {
    pub service: AppService,
    pub wall_subscriptions: WallSubscriptionRegistry,
    pub copy_operation: Arc<tokio::sync::Mutex<()>>,
    pub copy_registry: CopyOperationRegistry,
    pub last_completed_copy_destination: Arc<Mutex<Option<std::path::PathBuf>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copy_cancel_waits_for_finish_and_old_generation_cannot_clear_next() {
        let registry = CopyOperationRegistry::default();
        let first = registry.begin();
        let first_id = first.id;
        let mut cancelled = registry.cancel().unwrap();
        let second_cancel = registry.cancel().unwrap();
        assert!(first.cancellation.is_cancelled());
        assert!(!*cancelled.borrow());
        assert!(!*second_cancel.borrow());
        drop(first);
        cancelled.changed().await.unwrap();
        assert!(*cancelled.borrow());
        let next = registry.begin();
        assert!(next.id > first_id);
        registry.finish(first_id);
        assert!(!next.cancellation.is_cancelled());
        assert!(*second_cancel.borrow());
        drop(next);
        assert!(registry.cancel().is_none());
    }

    #[tokio::test]
    async fn cancellation_registry_owns_and_terminates_forwarders() {
        let registry = WallSubscriptionRegistry::default();
        let (id, mut cancellation) = registry.register();
        assert_eq!(registry.active_count(), 1);
        assert!(registry.cancel(&id));
        assert_eq!(registry.active_count(), 0);
        cancellation.changed().await.unwrap();
        assert!(*cancellation.borrow());
        assert!(!registry.cancel(&id));
    }
}
