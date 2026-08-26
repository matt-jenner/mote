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

pub struct DesktopState {
    pub service: AppService,
    pub wall_subscriptions: WallSubscriptionRegistry,
}

#[cfg(test)]
mod tests {
    use super::*;

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
