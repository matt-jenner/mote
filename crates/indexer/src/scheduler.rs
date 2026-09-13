use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
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
    folders: std::sync::Mutex<FolderAdmission>,
}

type FolderKey = (photo_domain::LibraryId, photo_domain::FolderGroupId);

#[derive(Default)]
struct FolderAdmission {
    desktop: bool,
    foreground: Option<FolderKey>,
    waiting: VecDeque<FolderKey>,
    active: HashMap<FolderKey, usize>,
    foreground_burst: usize,
}

pub struct FolderWorkPermit {
    scheduler: Arc<IndexScheduler>,
    folder: FolderKey,
    priority_turn: bool,
}

pub struct FolderWorkScope {
    scheduler: Arc<IndexScheduler>,
    folder: FolderKey,
}

impl Drop for FolderWorkScope {
    fn drop(&mut self) {
        self.scheduler.finish_folder_enrichment(Some(self.folder));
    }
}

impl FolderWorkPermit {
    pub fn is_priority_turn(&self) -> bool {
        self.priority_turn
    }
}

impl Drop for FolderWorkPermit {
    fn drop(&mut self) {
        self.scheduler.release_folder_enrichment(Some(self.folder));
    }
}

#[cfg(test)]
mod folder_admission_tests {
    use super::*;
    use photo_domain::{FolderGroupId, LibraryId};

    fn folder() -> (LibraryId, FolderGroupId) {
        (LibraryId::new(), FolderGroupId::new())
    }

    #[test]
    fn foreground_keeps_responsive_capacity_while_old_folder_is_bounded() {
        let scheduler = IndexScheduler::new(SchedulerConfig {
            idle_workers: 4,
            active_workers: 1,
        });
        let a = folder();
        let b = folder();
        scheduler.set_foreground_folder(Some(a));
        assert!(scheduler.try_admit_folder_enrichment(Some(a)));
        scheduler.set_foreground_folder(Some(b));
        assert!(
            !scheduler.try_admit_folder_enrichment(Some(a)),
            "background has only one admitted file"
        );
        assert!(scheduler.try_admit_folder_enrichment(Some(b)));
        scheduler.release_folder_enrichment(Some(a));
        scheduler.release_folder_enrichment(Some(b));
    }

    #[test]
    fn derivative_folder_permit_releases_capacity_on_drop() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig {
            idle_workers: 1,
            active_workers: 1,
        }));
        let a = folder();
        scheduler.set_foreground_folder(Some(a));
        let permit = scheduler.try_admit_folder_work(a).unwrap();
        assert!(scheduler.try_admit_folder_work(a).is_none());
        drop(permit);
        assert!(scheduler.try_admit_folder_work(a).is_some());
    }

    #[test]
    fn cleared_desktop_focus_keeps_all_folders_in_the_bounded_background_lane() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        scheduler.set_foreground_folder(None);
        let permit = scheduler.try_admit_folder_work(folder()).unwrap();
        assert!(scheduler.try_admit_folder_work(folder()).is_none());
        drop(permit);
    }

    #[tokio::test]
    async fn three_background_folders_take_round_robin_turns_without_starvation() {
        let scheduler = IndexScheduler::new(SchedulerConfig {
            idle_workers: 4,
            active_workers: 1,
        });
        scheduler
            .set_interaction_mode(InteractionMode::Active)
            .await;
        let foreground = folder();
        let backgrounds = [folder(), folder(), folder()];
        scheduler.set_foreground_folder(Some(foreground));
        assert!(scheduler.try_admit_folder_enrichment(Some(foreground)));
        for background in backgrounds {
            assert!(!scheduler.try_admit_folder_enrichment(Some(background)));
        }
        scheduler.release_folder_enrichment(Some(foreground));
        for expected in backgrounds {
            // Foreground wins at most three admissions before a waiting
            // background folder gets its next turn, even with one permit.
            for _ in 0..2 {
                assert!(scheduler.try_admit_folder_enrichment(Some(foreground)));
                scheduler.release_folder_enrichment(Some(foreground));
            }
            assert!(!scheduler.try_admit_folder_enrichment(Some(foreground)));
            for other in backgrounds.into_iter().filter(|folder| *folder != expected) {
                assert!(!scheduler.try_admit_folder_enrichment(Some(other)));
            }
            assert!(scheduler.try_admit_folder_enrichment(Some(expected)));
            scheduler.release_folder_enrichment(Some(expected));
            assert!(scheduler.try_admit_folder_enrichment(Some(foreground)));
            scheduler.release_folder_enrichment(Some(foreground));
        }
    }
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
            folders: std::sync::Mutex::new(FolderAdmission::default()),
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
            .filter(|entry| entry.name.starts_with(family_prefix))
            .filter_map(|entry| {
                state.queued.get(&entry.name).and_then(|queued| {
                    (queued.sequence == entry.sequence && queued.job.priority == entry.priority)
                        .then_some(entry.priority)
                })
            })
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

    /// Desktop focus affects admission, never ownership of an existing scan.
    /// Hosted callers leave focus unset and retain the shared global budget.
    pub fn set_foreground_folder(&self, folder: Option<FolderKey>) {
        let mut folders = self.folders.lock().expect("folder admission poisoned");
        folders.desktop = true;
        if folders.foreground != folder {
            folders.foreground = folder;
            folders.foreground_burst = 0;
        }
        self.notify_change();
    }

    pub(crate) fn try_admit_folder_enrichment(&self, folder: Option<FolderKey>) -> bool {
        let Some(folder) = folder else {
            return self.try_admit_enrichment();
        };
        let mut folders = self.folders.lock().expect("folder admission poisoned");
        if !folders.waiting.contains(&folder) {
            folders.waiting.push_back(folder);
        }
        if let Some(foreground) = folders.foreground {
            let next_background = folders
                .waiting
                .iter()
                .find(|key| **key != foreground)
                .copied();
            if folder == foreground {
                if next_background.is_some() && folders.foreground_burst >= 3 {
                    return false;
                }
            } else {
                let active_background: usize = folders
                    .active
                    .iter()
                    .filter(|(key, _)| **key != foreground)
                    .map(|(_, count)| *count)
                    .sum();
                if active_background >= 1 || next_background != Some(folder) {
                    return false;
                }
                if folders.waiting.contains(&foreground)
                    && folders.foreground_burst < 3
                    && (self.available_background_permits() <= 1
                        || !folders.active.contains_key(&foreground))
                {
                    return false;
                }
            }
        } else if folders.desktop
            && (folders.active.values().sum::<usize>() >= 1
                || folders.waiting.front() != Some(&folder))
        {
            return false;
        }
        if !self.try_admit_enrichment() {
            return false;
        }
        folders.waiting.retain(|key| *key != folder);
        *folders.active.entry(folder).or_default() += 1;
        if folders.foreground == Some(folder) {
            folders.foreground_burst += 1;
        } else {
            folders.foreground_burst = 0;
        }
        true
    }

    pub(crate) fn release_folder_enrichment(&self, folder: Option<FolderKey>) {
        if let Some(folder) = folder {
            let mut folders = self.folders.lock().expect("folder admission poisoned");
            if let Some(count) = folders.active.get_mut(&folder) {
                *count -= 1;
                if *count == 0 {
                    folders.active.remove(&folder);
                }
            }
        }
        self.release_enrichment();
        self.notify_change();
    }

    pub fn try_admit_folder_work(self: &Arc<Self>, folder: FolderKey) -> Option<FolderWorkPermit> {
        let priority_turn = {
            let folders = self.folders.lock().expect("folder admission poisoned");
            // Desktop folder admission already arbitrates foreground priority
            // and bounded background turns. A second family-wide FIFO barrier
            // would contradict that admission and can strand another folder.
            folders.desktop
        };
        self.try_admit_folder_enrichment(Some(folder))
            .then(|| FolderWorkPermit {
                scheduler: self.clone(),
                folder,
                priority_turn,
            })
    }

    pub fn folder_work_scope(self: &Arc<Self>, folder: FolderKey) -> FolderWorkScope {
        FolderWorkScope {
            scheduler: self.clone(),
            folder,
        }
    }

    pub(crate) fn finish_folder_enrichment(&self, folder: Option<FolderKey>) {
        if let Some(folder) = folder {
            self.folders
                .lock()
                .expect("folder admission poisoned")
                .waiting
                .retain(|key| *key != folder);
        }
        self.notify_change();
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
