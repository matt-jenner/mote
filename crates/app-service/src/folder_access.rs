//! Shares bounded filesystem access probes across selections and clients.

use photo_domain::{LibraryId, RelativePathKey};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, watch},
    time::Instant,
};

const COOLDOWN: Duration = Duration::from_secs(5);
const WAIT_LIMIT: Duration = Duration::from_secs(5);
const MAX_KEYS: usize = 256;
const MAX_WAITERS: usize = 64;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FolderAccessKey {
    pub library_id: LibraryId,
    pub relative: RelativePathKey,
}

#[derive(Clone, Debug)]
pub struct FolderAccessTarget {
    pub key: FolderAccessKey,
    pub root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ValidatedFolder {
    key: FolderAccessKey,
    canonical_path: PathBuf,
}

impl ValidatedFolder {
    pub fn key(&self) -> &FolderAccessKey {
        &self.key
    }
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

#[derive(Clone, Debug)]
pub enum FolderProbeOutcome {
    Available(ValidatedFolder),
    Missing,
    Unreadable,
    RootOffline,
    Invalid,
    Failed,
}

#[derive(Clone, Debug)]
pub enum AccessReply {
    Complete {
        generation: u64,
        outcome: FolderProbeOutcome,
        retry_after_ms: u64,
    },
    Checking {
        generation: u64,
    },
}

impl AccessReply {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Complete { generation, .. } | Self::Checking { generation } => *generation,
        }
    }
}

pub trait FolderProbe: Send + Sync + 'static {
    fn probe(&self, target: &FolderAccessTarget) -> FolderProbeOutcome;
}

struct DirectoryProbe;
impl FolderProbe for DirectoryProbe {
    fn probe(&self, target: &FolderAccessTarget) -> FolderProbeOutcome {
        let Ok(relative) = target.key.relative.to_path_buf() else {
            return FolderProbeOutcome::Invalid;
        };
        let path = target.root.join(relative);
        let canonical = match path.canonicalize() {
            Ok(path) => path,
            Err(error) => return io_outcome(error),
        };
        if !canonical.starts_with(&target.root) {
            return FolderProbeOutcome::Invalid;
        }
        match std::fs::read_dir(&canonical) {
            Ok(_) => {
                let Ok(relative) = canonical.strip_prefix(&target.root) else {
                    return FolderProbeOutcome::Invalid;
                };
                let Ok(relative) = RelativePathKey::from_relative_path(relative) else {
                    return FolderProbeOutcome::Invalid;
                };
                FolderProbeOutcome::Available(ValidatedFolder {
                    key: FolderAccessKey {
                        library_id: target.key.library_id,
                        relative,
                    },
                    canonical_path: canonical,
                })
            }
            Err(error) => io_outcome(error),
        }
    }
}

fn io_outcome(error: std::io::Error) -> FolderProbeOutcome {
    match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            FolderProbeOutcome::Missing
        }
        std::io::ErrorKind::PermissionDenied => FolderProbeOutcome::Unreadable,
        _ => FolderProbeOutcome::Failed,
    }
}

struct Slot {
    generation: u64,
    started: Instant,
    completed: Option<(Instant, FolderProbeOutcome)>,
    sender: watch::Sender<Option<FolderProbeOutcome>>,
}

enum Admission {
    Ready(AccessReply),
    Wait {
        generation: u64,
        receiver: watch::Receiver<Option<FolderProbeOutcome>>,
        start: bool,
    },
}

struct Inner {
    slots: Mutex<HashMap<FolderAccessKey, Slot>>,
    sequence: AtomicU64,
    permits: Arc<Semaphore>,
    probe: Arc<dyn FolderProbe>,
}

#[derive(Clone)]
pub struct FolderAccessCoordinator {
    inner: Arc<Inner>,
}

impl Default for FolderAccessCoordinator {
    fn default() -> Self {
        Self::new(Arc::new(DirectoryProbe))
    }
}

impl FolderAccessCoordinator {
    pub fn new(probe: Arc<dyn FolderProbe>) -> Self {
        Self {
            inner: Arc::new(Inner {
                slots: Mutex::new(HashMap::new()),
                sequence: AtomicU64::new(0),
                permits: Arc::new(Semaphore::new(4)),
                probe,
            }),
        }
    }

    /// Resolves picker aliases using native paths. This shares the worker bound;
    /// access permission and cooldown are still decided by the folder probe.
    pub async fn canonical_identity(&self, path: PathBuf) -> Option<PathBuf> {
        let permit = self.inner.permits.clone().try_acquire_owned().ok()?;
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            path.canonicalize().ok()
        });
        tokio::time::timeout(WAIT_LIMIT, worker).await.ok()?.ok()?
    }

    /// Reads runtime status without probing or treating an old result as fresh.
    pub fn peek(&self, key: &FolderAccessKey) -> Option<AccessReply> {
        let slots = self.inner.slots.lock().ok()?;
        let slot = slots.get(key)?;
        Some(reply(slot, Instant::now()))
    }

    /// Production callers share a root prerequisite before child probes.
    pub async fn check_with_root(&self, target: FolderAccessTarget) -> AccessReply {
        let key = target.key.clone();
        match tokio::time::timeout(WAIT_LIMIT, self.check_with_root_inner(target)).await {
            Ok(reply) => reply,
            Err(_) => AccessReply::Checking {
                generation: self.peek(&key).map(|r| r.generation()).unwrap_or(0),
            },
        }
    }

    async fn check_with_root_inner(&self, target: FolderAccessTarget) -> AccessReply {
        if !target
            .key
            .relative
            .to_path_buf()
            .is_ok_and(|p| p.as_os_str().is_empty())
        {
            let root = FolderAccessTarget {
                root: target.root.clone(),
                key: FolderAccessKey {
                    library_id: target.key.library_id,
                    relative: RelativePathKey::from_relative_path(Path::new(""))
                        .expect("empty relative path"),
                },
            };
            match self.check(root).await {
                AccessReply::Complete {
                    outcome: FolderProbeOutcome::Available(_),
                    ..
                } => {}
                AccessReply::Complete {
                    generation,
                    outcome,
                    retry_after_ms,
                } => {
                    let outcome = match outcome {
                        FolderProbeOutcome::Missing | FolderProbeOutcome::Unreadable => {
                            FolderProbeOutcome::RootOffline
                        }
                        other => other,
                    };
                    return AccessReply::Complete {
                        generation,
                        outcome,
                        retry_after_ms,
                    };
                }
                checking => return checking,
            }
        }
        self.check(target).await
    }

    fn admit(&self, key: &FolderAccessKey) -> Admission {
        let now = Instant::now();
        let Ok(mut slots) = self.inner.slots.lock() else {
            return Admission::Ready(AccessReply::Complete {
                generation: 0,
                outcome: FolderProbeOutcome::Failed,
                retry_after_ms: 0,
            });
        };
        if let Some(slot) = slots.get(key) {
            if let Some((completed, _)) = slot.completed.as_ref() {
                if now.saturating_duration_since(*completed) < COOLDOWN {
                    return Admission::Ready(reply(slot, now));
                }
            } else {
                if now.saturating_duration_since(slot.started) >= WAIT_LIMIT
                    || slot.sender.receiver_count() >= MAX_WAITERS
                {
                    return Admission::Ready(AccessReply::Checking {
                        generation: slot.generation,
                    });
                }
                return Admission::Wait {
                    generation: slot.generation,
                    receiver: slot.sender.subscribe(),
                    start: false,
                };
            }
        }
        slots.retain(|_, slot| {
            slot.completed
                .as_ref()
                .is_none_or(|(completed, _)| now.saturating_duration_since(*completed) < COOLDOWN)
        });
        if slots.len() >= MAX_KEYS {
            return Admission::Ready(AccessReply::Checking {
                generation: self.inner.sequence.load(Ordering::Relaxed),
            });
        }
        let generation = self.inner.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = watch::channel(None);
        slots.insert(
            key.clone(),
            Slot {
                generation,
                started: now,
                completed: None,
                sender,
            },
        );
        Admission::Wait {
            generation,
            receiver,
            start: true,
        }
    }

    pub async fn check(&self, target: FolderAccessTarget) -> AccessReply {
        let (generation, receiver, start) = match self.admit(&target.key) {
            Admission::Ready(reply) => return reply,
            Admission::Wait {
                generation,
                receiver,
                start,
            } => (generation, receiver, start),
        };
        if start {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                let permits = inner.permits.clone();
                let probe = inner.probe.clone();
                let key = target.key.clone();
                let outcome = match permits.acquire_owned().await {
                    Ok(permit) => tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        probe.probe(&target)
                    })
                    .await
                    .unwrap_or(FolderProbeOutcome::Failed),
                    Err(_) => FolderProbeOutcome::Failed,
                };
                if let Ok(mut slots) = inner.slots.lock()
                    && let Some(slot) = slots.get_mut(&key)
                    && slot.generation == generation
                {
                    slot.completed = Some((Instant::now(), outcome.clone()));
                    slot.sender.send_replace(Some(outcome));
                }
            });
        }
        self.wait(generation, receiver).await
    }

    async fn wait(
        &self,
        generation: u64,
        mut receiver: watch::Receiver<Option<FolderProbeOutcome>>,
    ) -> AccessReply {
        let waiting = async {
            loop {
                if let Some(outcome) = receiver.borrow_and_update().clone() {
                    return Some(outcome);
                }
                if receiver.changed().await.is_err() {
                    return None;
                }
            }
        };
        match tokio::time::timeout(WAIT_LIMIT, waiting).await {
            Ok(Some(outcome)) => AccessReply::Complete {
                generation,
                outcome,
                retry_after_ms: COOLDOWN.as_millis() as u64,
            },
            Ok(None) => AccessReply::Complete {
                generation,
                outcome: FolderProbeOutcome::Failed,
                retry_after_ms: 0,
            },
            Err(_) => AccessReply::Checking { generation },
        }
    }
}

fn reply(slot: &Slot, now: Instant) -> AccessReply {
    match &slot.completed {
        Some((completed, outcome)) => AccessReply::Complete {
            generation: slot.generation,
            outcome: outcome.clone(),
            retry_after_ms: COOLDOWN
                .saturating_sub(now.saturating_duration_since(*completed))
                .as_millis() as u64,
        },
        None => AccessReply::Checking {
            generation: slot.generation,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Condvar, atomic::AtomicUsize};

    #[test]
    fn admission_bounds_distinct_keys_and_waiters_without_releasing_running_slots() {
        let coordinator = FolderAccessCoordinator::default();
        let library_id = LibraryId::new();
        let mut receivers = Vec::new();
        for index in 0..MAX_KEYS {
            let key = FolderAccessKey {
                library_id,
                relative: RelativePathKey::from_relative_path(Path::new(&format!(
                    "folder-{index}"
                )))
                .unwrap(),
            };
            let Admission::Wait {
                receiver, start, ..
            } = coordinator.admit(&key)
            else {
                panic!("bounded capacity was not admitted");
            };
            assert!(start);
            receivers.push(receiver);
        }
        let extra = FolderAccessKey {
            library_id,
            relative: RelativePathKey::from_relative_path(Path::new("extra")).unwrap(),
        };
        assert!(matches!(
            coordinator.admit(&extra),
            Admission::Ready(AccessReply::Checking { .. })
        ));
        let first = FolderAccessKey {
            library_id,
            relative: RelativePathKey::from_relative_path(Path::new("folder-0")).unwrap(),
        };
        for _ in 1..MAX_WAITERS {
            let Admission::Wait {
                receiver, start, ..
            } = coordinator.admit(&first)
            else {
                panic!("bounded waiter was not admitted");
            };
            assert!(!start);
            receivers.push(receiver);
        }
        assert!(matches!(
            coordinator.admit(&first),
            Admission::Ready(AccessReply::Checking { .. })
        ));
    }

    struct Gates {
        entered: AtomicUsize,
        released: Mutex<usize>,
        changed: Condvar,
    }
    impl FolderProbe for Gates {
        fn probe(&self, target: &FolderAccessTarget) -> FolderProbeOutcome {
            let stage = self.entered.fetch_add(1, Ordering::SeqCst) + 1;
            let mut released = self.released.lock().unwrap();
            while *released < stage {
                released = self.changed.wait(released).unwrap();
            }
            FolderProbeOutcome::Available(ValidatedFolder {
                key: target.key.clone(),
                canonical_path: target.root.clone(),
            })
        }
    }
    struct ReleaseOnDrop(Arc<Gates>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            *self.0.released.lock().unwrap() = usize::MAX;
            self.0.changed.notify_all();
        }
    }

    #[tokio::test]
    async fn root_and_child_share_one_caller_deadline() {
        let gates = Arc::new(Gates {
            entered: AtomicUsize::new(0),
            released: Mutex::new(0),
            changed: Condvar::new(),
        });
        let _release = ReleaseOnDrop(gates.clone());
        let coordinator = FolderAccessCoordinator::new(gates.clone());
        let target = FolderAccessTarget {
            root: PathBuf::from("/test"),
            key: FolderAccessKey {
                library_id: LibraryId::new(),
                relative: RelativePathKey::from_relative_path(Path::new("child")).unwrap(),
            },
        };
        let running = tokio::spawn({
            let coordinator = coordinator.clone();
            let target = target.clone();
            async move { coordinator.check_with_root(target).await }
        });
        while gates.entered.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(4)).await;
        tokio::time::resume();
        *gates.released.lock().unwrap() = 1;
        gates.changed.notify_all();
        while gates.entered.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(2)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            running.is_finished(),
            "child probe incorrectly received a fresh five-second wait"
        );
        assert!(matches!(
            running.await.unwrap(),
            AccessReply::Checking { .. }
        ));
        assert!(matches!(
            coordinator.peek(&target.key),
            Some(AccessReply::Checking { .. })
        ));
        tokio::time::resume();
    }
}
