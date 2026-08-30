use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};

use tokio::sync::{Mutex, Notify};

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
    change_generation: AtomicUsize,
    wake: Notify,
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
            change_generation: AtomicUsize::new(0),
            wake: Notify::new(),
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
                self.notify_change();
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
        self.notify_change();
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
            let result = state.queued.remove(&entry.name).map(|queued| queued.job);
            if result.is_some() {
                self.notify_change();
            }
            return result;
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
        let result = state.queued.remove(&entry.name).map(|queued| queued.job);
        if result.is_some() {
            self.notify_change();
        }
        result
    }

    /// Dequeue an owner job only when it is the highest-priority job in the
    /// supplied family.  This is used by hosted derivative consumers: each
    /// selection has its own driver, but the family must still have one
    /// priority order across all selections.  The check and removal happen
    /// under the scheduler lock, so two drivers cannot reserve work out of
    /// order.
    pub async fn next_owned_in_family(
        &self,
        owner_prefix: &str,
        family_prefix: &str,
    ) -> Option<IndexJob> {
        let mut state = self.state.lock().await;
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
            if entry.name.starts_with(family_prefix)
                && selected
                    .as_ref()
                    .is_none_or(|best: &QueueEntry| entry > best)
            {
                selected = Some(entry.clone());
            }
            index += 1;
        }
        let Some(entry) = selected else {
            state.heap = BinaryHeap::from(heap);
            return None;
        };
        if !entry.name.starts_with(owner_prefix) {
            state.heap = BinaryHeap::from(heap);
            return None;
        }
        heap.retain(|queued| queued != &entry);
        state.heap = BinaryHeap::from(heap);
        let result = state.queued.remove(&entry.name).map(|queued| queued.job);
        if result.is_some() {
            self.notify_change();
        }
        result
    }

    /// Returns the highest-priority valid queued job without removing it.
    /// Consumers with per-owner queues use this as a global admission hint so
    /// a lower-priority owner cannot reserve capacity ahead of visible work.
    pub async fn highest_priority(&self) -> Option<JobPriority> {
        let state = self.state.lock().await;
        state
            .heap
            .iter()
            .filter_map(|entry| {
                state.queued.get(&entry.name).and_then(|queued| {
                    (queued.sequence == entry.sequence && queued.job.priority == entry.priority)
                        .then_some(entry.priority)
                })
            })
            .max()
    }

    /// Returns the highest-priority valid queued job in one consumer family.
    /// Shared schedulers also carry unrelated index jobs; those jobs must not
    /// block or reorder the hosted derivative family.
    pub async fn highest_priority_in_family(&self, family_prefix: &str) -> Option<JobPriority> {
        let state = self.state.lock().await;
        state
            .heap
            .iter()
            .filter_map(|entry| {
                (entry.name.starts_with(family_prefix)).then(|| {
                    state.queued.get(&entry.name).and_then(|queued| {
                        (queued.sequence == entry.sequence && queued.job.priority == entry.priority)
                            .then_some(entry.priority)
                    })
                })
            })
            .flatten()
            .max()
    }

    pub async fn set_interaction_mode(&self, mode: InteractionMode) {
        let permits = match mode {
            InteractionMode::Idle => self.config.idle_workers,
            InteractionMode::Active => self.config.active_workers,
        };
        self.available_background_permits
            .store(permits, AtomicOrdering::Release);
        self.notify_change();
    }

    pub fn change_generation(&self) -> u64 {
        self.change_generation.load(AtomicOrdering::Acquire) as u64
    }

    pub async fn wait_for_change_since(&self, observed: u64) {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.change_generation() != observed {
                return;
            }
            notified.await;
        }
    }

    pub async fn wait_for_change(&self) {
        self.wait_for_change_since(self.change_generation()).await;
    }

    fn notify_change(&self) {
        self.change_generation.fetch_add(1, AtomicOrdering::AcqRel);
        self.wake.notify_waiters();
    }

    pub fn available_background_permits(&self) -> usize {
        self.available_background_permits
            .load(AtomicOrdering::Acquire)
    }

    pub(crate) fn try_admit_enrichment(&self) -> bool {
        // This is shared by every scanner using the scheduler.  The scanner
        // still limits its own worker count, while this admission count keeps
        // combined shape and metadata work within the configured global
        // interaction budget.
        let limit = self.available_background_permits().max(1);
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
        let changed = !cancelled.is_empty();
        for (name, token) in cancelled {
            token.cancel();
            state.queued.remove(&name);
        }
        if changed {
            self.notify_change();
        }
    }
}
