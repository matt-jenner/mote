use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};

use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum JobPriority {
    IdleLibrary = 0,
    OpenCollection = 1,
    NearViewport = 2,
    ViewerPreview = 3,
    Visible = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionMode {
    Idle,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    pub idle_workers: usize,
    pub active_workers: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            idle_workers: 4,
            active_workers: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrdering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Release);
    }
}

#[derive(Clone, Debug)]
pub struct IndexJob {
    name: String,
    priority: JobPriority,
    cancellation: CancellationToken,
}

impl IndexJob {
    pub fn new(name: impl Into<String>, priority: JobPriority) -> Self {
        Self {
            name: name.into(),
            priority,
            cancellation: CancellationToken {
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn priority(&self) -> JobPriority {
        self.priority
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }
}

pub struct IndexScheduler {
    state: Mutex<SchedulerState>,
    config: SchedulerConfig,
    available_background_permits: AtomicUsize,
    active_enrichment: AtomicUsize,
}

#[derive(Default)]
struct SchedulerState {
    next_sequence: u64,
    heap: BinaryHeap<QueueEntry>,
    queued: HashMap<String, QueuedJob>,
}

struct QueuedJob {
    sequence: u64,
    job: IndexJob,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueueEntry {
    name: String,
    priority: JobPriority,
    sequence: u64,
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| self.name.cmp(&other.name))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl IndexScheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            state: Mutex::new(SchedulerState::default()),
            available_background_permits: AtomicUsize::new(config.idle_workers),
            active_enrichment: AtomicUsize::new(0),
            config,
        }
    }

    pub async fn enqueue(&self, job: IndexJob) {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.queued.get_mut(job.name()) {
            if job.priority > existing.job.priority {
                existing.job = existing.job.clone().with_priority(job.priority);
                let entry = QueueEntry {
                    name: existing.job.name.clone(),
                    priority: existing.job.priority,
                    sequence: existing.sequence,
                };
                state.heap.push(entry);
            }
            return;
        }

        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        let entry = QueueEntry {
            name: job.name.clone(),
            priority: job.priority,
            sequence,
        };
        state
            .queued
            .insert(job.name.clone(), QueuedJob { sequence, job });
        state.heap.push(entry);
    }

    pub async fn next(&self) -> Option<IndexJob> {
        let mut state = self.state.lock().await;
        while let Some(entry) = state.heap.pop() {
            let Some(current) = state.queued.get(&entry.name) else {
                continue;
            };
            if current.sequence != entry.sequence || current.job.priority != entry.priority {
                continue;
            }
            return state.queued.remove(&entry.name).map(|queued| queued.job);
        }
        None
    }

    /// Dequeue the highest-priority job only when it belongs to `owner_prefix`.
    ///
    /// A shared scheduler can have several consumers.  Looking at the heap
    /// before removing anything keeps a consumer from temporarily taking a
    /// higher-priority job owned by another consumer.
    pub async fn next_owned(&self, owner_prefix: &str) -> Option<IndexJob> {
        let mut state = self.state.lock().await;
        // A foreign job may be globally highest priority. Find the best valid
        // job for this owner without removing or reordering any foreign work.
        let mut heap = std::mem::take(&mut state.heap).into_vec();
        let mut selected = None;
        let mut index = 0;
        while index < heap.len() {
            let entry = &heap[index];
            let valid = state.queued.get(&entry.name).is_some_and(|current| {
                current.sequence == entry.sequence && current.job.priority == entry.priority
            });
            if !valid {
                heap.swap_remove(index);
                continue;
            }
            if entry.name.starts_with(owner_prefix)
                && selected
                    .as_ref()
                    .is_none_or(|best: &QueueEntry| entry > best)
            {
                selected = Some(entry.clone());
            }
            index += 1;
        }
        state.heap = BinaryHeap::from(heap);
        let entry = selected?;
        state.queued.remove(&entry.name).map(|queued| queued.job)
    }

    pub async fn set_interaction_mode(&self, mode: InteractionMode) {
        let permits = match mode {
            InteractionMode::Idle => self.config.idle_workers,
            InteractionMode::Active => self.config.active_workers,
        };
        self.available_background_permits
            .store(permits, AtomicOrdering::Release);
    }

    pub fn available_background_permits(&self) -> usize {
        self.available_background_permits
            .load(AtomicOrdering::Acquire)
    }

    pub(crate) fn try_admit_enrichment(&self) -> bool {
        let limit = self.available_background_permits().clamp(1, 2);
        let mut current = self.active_enrichment.load(AtomicOrdering::Acquire);
        loop {
            if current >= limit {
                return false;
            }
            match self.active_enrichment.compare_exchange_weak(
                current,
                current + 1,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }

    pub(crate) fn release_enrichment(&self) {
        self.active_enrichment.fetch_sub(1, AtomicOrdering::AcqRel);
    }

    pub async fn cancel_below(&self, minimum: JobPriority) {
        let mut state = self.state.lock().await;
        let cancelled = state
            .queued
            .iter()
            .filter(|(_, queued)| queued.job.priority < minimum)
            .map(|(name, queued)| (name.clone(), queued.job.cancellation.clone()))
            .collect::<Vec<_>>();
        for (name, token) in cancelled {
            token.cancel();
            state.queued.remove(&name);
        }
    }
}
