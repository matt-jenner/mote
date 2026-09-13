//! Lifecycle adapter for the gallery's existing per-folder runtimes.
//! Work is executed only by SelectionRuntime and the shared IndexScheduler.

use crate::hosted_runtime::SelectionRuntime;
use photo_domain::{FolderGroupId, LibraryId};
use photo_indexer::IndexScheduler;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

pub(crate) type FolderJobKey = (LibraryId, FolderGroupId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FolderJobPriority {
    Foreground,
    Background,
}

pub(crate) struct FolderJobRegistry {
    pub(crate) runtimes: Mutex<HashMap<FolderJobKey, Weak<SelectionRuntime>>>,
    foreground: Mutex<Option<FolderJobKey>>,
    scheduler: Arc<IndexScheduler>,
}

impl FolderJobRegistry {
    pub(crate) fn new(scheduler: Arc<IndexScheduler>) -> Self {
        Self {
            runtimes: Mutex::new(HashMap::new()),
            foreground: Mutex::new(None),
            scheduler,
        }
    }

    pub(crate) fn ensure(
        &self,
        key: FolderJobKey,
        create: impl FnOnce() -> Arc<SelectionRuntime>,
    ) -> Arc<SelectionRuntime> {
        let mut runtimes = self.runtimes.lock().expect("folder jobs poisoned");
        runtimes.retain(|_, runtime| runtime.strong_count() > 0);
        if let Some(runtime) = runtimes.get(&key).and_then(Weak::upgrade) {
            return runtime;
        }
        let runtime = create();
        runtimes.insert(key, Arc::downgrade(&runtime));
        runtime
    }

    pub(crate) fn set_foreground(&self, key: Option<FolderJobKey>) {
        *self.foreground.lock().expect("folder jobs poisoned") = key;
        self.scheduler.set_foreground_folder(key);
    }

    pub(crate) fn priority(&self, key: FolderJobKey) -> FolderJobPriority {
        if *self.foreground.lock().expect("folder jobs poisoned") == Some(key) {
            FolderJobPriority::Foreground
        } else {
            FolderJobPriority::Background
        }
    }

    pub(crate) fn finish(&self, key: FolderJobKey, runtime: &Arc<SelectionRuntime>) {
        let mut runtimes = self.runtimes.lock().expect("folder jobs poisoned");
        if runtimes.get(&key).is_some_and(|weak| {
            weak.upgrade()
                .is_none_or(|current| Arc::ptr_eq(&current, runtime))
        }) && Arc::strong_count(runtime) <= 1
        {
            runtimes.remove(&key);
        }
    }

    pub(crate) fn cancel_removed(&self, key: FolderJobKey) {
        let runtime = self
            .runtimes
            .lock()
            .expect("folder jobs poisoned")
            .remove(&key)
            .and_then(|runtime| runtime.upgrade());
        if let Some(runtime) = runtime {
            runtime.cancel_folder_work();
            let coordinator = runtime.coordinator.clone();
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::spawn(async move {
                    coordinator.close_folder().await;
                });
            }
        }
        drop(self.scheduler.folder_work_scope(key));
        if self.priority(key) == FolderJobPriority::Foreground {
            self.set_foreground(None);
        }
    }

    pub(crate) fn cancel_inaccessible(&self, library: LibraryId, group: Option<FolderGroupId>) {
        let keys = self
            .runtimes
            .lock()
            .expect("folder jobs poisoned")
            .keys()
            .copied()
            .filter(|key| key.0 == library && group.is_none_or(|group| key.1 == group))
            .collect::<Vec<_>>();
        for key in keys {
            self.cancel_removed(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GallerySelection;
    use photo_domain::RelativePathKey;
    use photo_indexer::SchedulerConfig;

    #[tokio::test]
    async fn folder_jobs_ensure_retains_running_folder_when_focus_changes() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let registry = FolderJobRegistry::new(scheduler.clone());
        let a = (LibraryId::new(), FolderGroupId::new());
        let b = (LibraryId::new(), FolderGroupId::new());
        let runtime = registry.ensure(a, || {
            SelectionRuntime::new_at_recovery(
                GallerySelection {
                    id: "runtime-a".into(),
                    library_id: a.0,
                    group_id: a.1,
                    relative_folder: RelativePathKey::from_relative_path(std::path::Path::new(""))
                        .unwrap(),
                    epoch: 1,
                },
                scheduler,
                false,
                0,
            )
        });
        registry.set_foreground(Some(a));
        registry.set_foreground(Some(b));
        let existing = registry.ensure(a, || panic!("must not start a second runtime"));
        assert!(Arc::ptr_eq(&runtime, &existing));
        assert!(!runtime.cancellation_requested());
        assert_eq!(registry.priority(a), FolderJobPriority::Background);
        assert_eq!(registry.priority(b), FolderJobPriority::Foreground);
        registry.cancel_removed(a);
        assert!(runtime.cancellation_requested());
    }
}
