#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::dto::{DerivativeClass, DerivativeReference};
use crate::service::SelectionToken;
use photo_catalog::WallCursorKey;
use photo_domain::AssetId;
use photo_indexer::{IndexJob, IndexScheduler, JobPriority};
use tokio::sync::{Mutex, Notify, oneshot};

pub(crate) const RECENT_CAPACITY: usize = 250;
const SCHEDULER_OWNER_PREFIX: &str = "photo-derivative-coordinator:";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum WorkLane {
    IdlePreview,
    IdleWall,
    NearWall,
    ViewerPreview,
    VisibleWall,
}

impl WorkLane {
    fn is_background(self) -> bool {
        matches!(self, Self::IdlePreview | Self::IdleWall)
    }

    fn is_foreground(self) -> bool {
        !self.is_background()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkKey {
    pub(crate) selection: SelectionToken,
    pub(crate) asset_id: AssetId,
    pub(crate) class: DerivativeClass,
    pub(crate) cache_key: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkTicket {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommitPermit {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
    pub(crate) selection: SelectionToken,
}

pub(crate) type WorkResultReceiver = oneshot::Receiver<Option<DerivativeReference>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CollectionPhase {
    Dormant,
    Thumbnails,
    Previews,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CollectionState {
    phase: CollectionPhase,
    cursor: Option<WallCursorKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobStatus {
    Queued,
    Running,
    Committing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TicketStatus {
    Running,
    Committing,
}

struct JobState {
    job_id: u64,
    job_name: String,
    lane: WorkLane,
    status: JobStatus,
    ticket: Option<WorkTicket>,
    background_generation: u64,
    commit_background: bool,
    prerequisite_key: Option<String>,
    foreground_waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
    background_waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
    completion: Arc<Notify>,
}

struct TicketState {
    key: WorkKey,
    lane: WorkLane,
    status: TicketStatus,
}

struct CoordinatorState {
    selection: Option<SelectionToken>,
    background_generation: u64,
    next_job_id: u64,
    next_attempt: u64,
    recent: VecDeque<AssetId>,
    jobs: HashMap<WorkKey, JobState>,
    job_names: HashMap<String, WorkKey>,
    tickets: HashMap<WorkTicket, TicketState>,
    terminal: HashSet<WorkKey>,
    collection: CollectionState,
}

impl Default for CoordinatorState {
    fn default() -> Self {
        Self {
            selection: None,
            background_generation: 0,
            next_job_id: 1,
            next_attempt: 1,
            recent: VecDeque::with_capacity(RECENT_CAPACITY),
            jobs: HashMap::new(),
            job_names: HashMap::new(),
            tickets: HashMap::new(),
            terminal: HashSet::new(),
            collection: CollectionState {
                phase: CollectionPhase::Dormant,
                cursor: None,
            },
        }
    }
}

pub(crate) struct DerivativeCoordinator {
    state: Mutex<CoordinatorState>,
    scheduler: Arc<IndexScheduler>,
    wake: Notify,
}

impl DerivativeCoordinator {
    pub(crate) fn new(scheduler: Arc<IndexScheduler>) -> Self {
        Self {
            state: Mutex::new(CoordinatorState::default()),
            scheduler,
            wake: Notify::new(),
        }
    }

    pub(crate) fn with_selection(
        scheduler: Arc<IndexScheduler>,
        selection: SelectionToken,
    ) -> Self {
        let state = CoordinatorState {
            selection: Some(selection),
            ..CoordinatorState::default()
        };
        Self {
            state: Mutex::new(state),
            scheduler,
            wake: Notify::new(),
        }
    }

    pub(crate) async fn enqueue(&self, key: WorkKey, lane: WorkLane) -> WorkResultReceiver {
        self.enqueue_with_prerequisite(key, lane, None).await
    }

    pub(crate) async fn enqueue_with_prerequisite(
        &self,
        key: WorkKey,
        lane: WorkLane,
        prerequisite_key: Option<String>,
    ) -> WorkResultReceiver {
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        let mut schedule = None;
        let mut immediate_none = false;
        {
            let mut state = self.state.lock().await;
            if let Some(selection) = state.selection {
                if selection != key.selection {
                    immediate_none = true;
                }
            } else {
                state.selection = Some(key.selection);
            }

            if !immediate_none && state.terminal.contains(&key) {
                immediate_none = true;
            }

            if !immediate_none {
                if state.jobs.contains_key(&key) {
                    let mut promote = None;
                    {
                        let work = state.jobs.get_mut(&key).expect("job was present");
                        if let Some(prerequisite_key) = prerequisite_key {
                            if work
                                .prerequisite_key
                                .as_ref()
                                .is_some_and(|existing| existing != &prerequisite_key)
                            {
                                immediate_none = true;
                            } else if work.prerequisite_key.is_none() {
                                work.prerequisite_key = Some(prerequisite_key);
                            }
                        }
                        if !immediate_none {
                            if lane.is_foreground() {
                                work.foreground_waiters
                                    .push(sender.take().expect("waiter was not consumed"));
                            } else {
                                work.background_waiters
                                    .push(sender.take().expect("waiter was not consumed"));
                            }
                            if lane > work.lane {
                                work.lane = lane;
                                promote = Some((
                                    work.job_name.clone(),
                                    work.ticket,
                                    work.status,
                                    work.lane,
                                ));
                            }
                        }
                    }
                    if let Some((job_name, ticket, status, promoted_lane)) = promote {
                        if status == JobStatus::Queued {
                            schedule = Some((job_name, scheduler_priority(promoted_lane)));
                        }
                        if let Some(ticket) = ticket
                            && let Some(ticket_state) = state.tickets.get_mut(&ticket)
                        {
                            ticket_state.lane = promoted_lane;
                        }
                    }
                } else {
                    let job_id = state.next_job_id;
                    state.next_job_id = state.next_job_id.wrapping_add(1).max(1);
                    let job_name = format!("{SCHEDULER_OWNER_PREFIX}{job_id}");
                    let (foreground_waiters, background_waiters) = if lane.is_foreground() {
                        (
                            vec![sender.take().expect("waiter was not consumed")],
                            Vec::new(),
                        )
                    } else {
                        (
                            Vec::new(),
                            vec![sender.take().expect("waiter was not consumed")],
                        )
                    };
                    let background_generation = state.background_generation;
                    state.jobs.insert(
                        key.clone(),
                        JobState {
                            job_id,
                            job_name: job_name.clone(),
                            lane,
                            status: JobStatus::Queued,
                            ticket: None,
                            background_generation,
                            commit_background: false,
                            prerequisite_key,
                            foreground_waiters,
                            background_waiters,
                            completion: Arc::new(Notify::new()),
                        },
                    );
                    state.job_names.insert(job_name.clone(), key);
                    schedule = Some((job_name, scheduler_priority(lane)));
                }
            }
        }
        if immediate_none {
            let _ = sender
                .take()
                .expect("rejected waiter was not consumed")
                .send(None);
        }
        if let Some((job_name, priority)) = schedule {
            self.scheduler
                .enqueue(IndexJob::new(job_name, priority))
                .await;
        }
        self.wake.notify_waiters();
        receiver
    }

    pub(crate) async fn next_work(&self) -> Option<WorkTicket> {
        loop {
            let job = self.scheduler.next_owned(SCHEDULER_OWNER_PREFIX).await?;
            let result = {
                let mut state = self.state.lock().await;
                match state.job_names.get(job.name()).cloned() {
                    None => None,
                    Some(key) => match state.jobs.get(&key) {
                        None => {
                            state.job_names.remove(job.name());
                            None
                        }
                        Some(work) => {
                            if work.status != JobStatus::Queued
                                || work.job_name != job.name()
                                || scheduler_priority(work.lane) != job.priority()
                                || (work.lane.is_background()
                                    && work.background_generation != state.background_generation)
                            {
                                None
                            } else {
                                let lane = work.lane;
                                let job_id = work.job_id;
                                let job_name = work.job_name.clone();
                                let attempt = state.next_attempt;
                                state.next_attempt = state.next_attempt.wrapping_add(1).max(1);
                                let ticket = WorkTicket { job_id, attempt };
                                state.job_names.remove(&job_name);
                                if let Some(work) = state.jobs.get_mut(&key) {
                                    work.status = JobStatus::Running;
                                    work.ticket = Some(ticket);
                                }
                                state.tickets.insert(
                                    ticket,
                                    TicketState {
                                        key,
                                        lane,
                                        status: TicketStatus::Running,
                                    },
                                );
                                Some(ticket)
                            }
                        }
                    },
                }
            };
            if let Some(ticket) = result {
                self.wake.notify_waiters();
                return Some(ticket);
            }
        }
    }

    pub(crate) async fn lane(&self, ticket: WorkTicket) -> WorkLane {
        self.state
            .lock()
            .await
            .tickets
            .get(&ticket)
            .map(|ticket| ticket.lane)
            .unwrap_or(WorkLane::IdlePreview)
    }

    pub(crate) async fn work_key(&self, ticket: WorkTicket) -> Option<WorkKey> {
        self.state
            .lock()
            .await
            .tickets
            .get(&ticket)
            .map(|ticket| ticket.key.clone())
    }

    pub(crate) async fn ticket_for(&self, key: &WorkKey) -> Option<WorkTicket> {
        self.state
            .lock()
            .await
            .jobs
            .get(key)
            .and_then(|work| work.ticket)
    }

    pub(crate) async fn background_generation(&self) -> u64 {
        self.state.lock().await.background_generation
    }

    pub(crate) async fn complete(&self, ticket: WorkTicket, reference: DerivativeReference) {
        let waiters = self.finish_running(ticket).await;
        for waiter in waiters {
            let _ = waiter.send(Some(reference.clone()));
        }
    }

    pub(crate) async fn discard(&self, ticket: WorkTicket) {
        let waiters = self.finish_running(ticket).await;
        for waiter in waiters {
            let _ = waiter.send(None);
        }
    }

    /// Returns a running attempt to the coordinator-owned scheduler after a driver
    /// observes that its lane is paused by interaction policy.
    pub(crate) async fn requeue(&self, ticket: WorkTicket) -> bool {
        let schedule = {
            let mut state = self.state.lock().await;
            let Some(ticket_state) = state.tickets.remove(&ticket) else {
                return false;
            };
            if ticket_state.status != TicketStatus::Running {
                state.tickets.insert(ticket, ticket_state);
                return false;
            }
            let key = ticket_state.key;
            let Some(work) = state.jobs.get_mut(&key) else {
                return false;
            };
            if work.ticket != Some(ticket) || work.status != JobStatus::Running {
                return false;
            }
            work.ticket = None;
            work.status = JobStatus::Queued;
            let job_name = work.job_name.clone();
            let priority = scheduler_priority(work.lane);
            state.job_names.insert(job_name.clone(), key);
            (job_name, priority)
        };
        self.scheduler
            .enqueue(IndexJob::new(schedule.0, schedule.1))
            .await;
        self.wake.notify_waiters();
        true
    }

    async fn finish_running(
        &self,
        ticket: WorkTicket,
    ) -> Vec<oneshot::Sender<Option<DerivativeReference>>> {
        let waiters = {
            let mut state = self.state.lock().await;
            let Some(ticket_state) = state.tickets.get(&ticket) else {
                return Vec::new();
            };
            if ticket_state.status != TicketStatus::Running {
                return Vec::new();
            }
            let key = ticket_state.key.clone();
            let Some(work) = state.jobs.get(&key) else {
                state.tickets.remove(&ticket);
                return Vec::new();
            };
            if work.ticket != Some(ticket) || work.status != JobStatus::Running {
                return Vec::new();
            }
            let work = state.jobs.remove(&key).expect("job was present");
            state.job_names.remove(&work.job_name);
            state.tickets.remove(&ticket);
            work.foreground_waiters
                .into_iter()
                .chain(work.background_waiters)
                .collect()
        };
        self.wake.notify_waiters();
        waiters
    }

    pub(crate) async fn invalidate_background(&self) {
        loop {
            let (waiters, completion_waits) = {
                let mut state = self.state.lock().await;
                state.background_generation = state.background_generation.wrapping_add(1);
                let keys = state
                    .jobs
                    .iter()
                    .filter(|(_, work)| {
                        (work.status == JobStatus::Committing && work.commit_background)
                            || (work.lane.is_background() && work.foreground_waiters.is_empty())
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                let mut waiters = Vec::new();
                let mut completion_waits = Vec::new();
                for key in keys {
                    let Some(work) = state.jobs.get(&key) else {
                        continue;
                    };
                    if work.status == JobStatus::Committing {
                        completion_waits.push(work.completion.clone().notified_owned());
                        continue;
                    }
                    let work = state.jobs.remove(&key).expect("job was present");
                    state.job_names.remove(&work.job_name);
                    if let Some(ticket) = work.ticket {
                        state.tickets.remove(&ticket);
                    }
                    waiters.extend(work.background_waiters);
                    waiters.extend(work.foreground_waiters);
                }
                (waiters, completion_waits)
            };
            for waiter in waiters {
                let _ = waiter.send(None);
            }
            self.wake.notify_waiters();
            if completion_waits.is_empty() {
                return;
            }
            for completion in completion_waits {
                completion.await;
            }
        }
    }

    pub(crate) async fn admit_commit(
        &self,
        ticket: WorkTicket,
        selection: SelectionToken,
        prerequisite_key: Option<&str>,
    ) -> Option<CommitPermit> {
        let mut state = self.state.lock().await;
        if state.selection != Some(selection) {
            return None;
        }
        let ticket_state = state.tickets.get(&ticket)?;
        if ticket_state.status != TicketStatus::Running {
            return None;
        }
        let key = ticket_state.key.clone();
        if key.selection != selection {
            return None;
        }
        let work = state.jobs.get_mut(&key)?;
        if work.status != JobStatus::Running || work.ticket != Some(ticket) {
            return None;
        }
        if work.prerequisite_key.as_deref() != prerequisite_key {
            return None;
        }
        work.commit_background = work.lane.is_background() && work.foreground_waiters.is_empty();
        work.status = JobStatus::Committing;
        if let Some(ticket_state) = state.tickets.get_mut(&ticket) {
            ticket_state.status = TicketStatus::Committing;
        }
        Some(CommitPermit {
            job_id: ticket.job_id,
            attempt: ticket.attempt,
            selection,
        })
    }

    pub(crate) async fn complete_commit(
        &self,
        permit: CommitPermit,
        reference: DerivativeReference,
    ) {
        let waiters = self.finish_commit(permit).await;
        for waiter in waiters {
            let _ = waiter.send(Some(reference.clone()));
        }
    }

    pub(crate) async fn fail_commit(&self, permit: CommitPermit) {
        let waiters = self.finish_commit(permit).await;
        for waiter in waiters {
            let _ = waiter.send(None);
        }
    }

    async fn finish_commit(
        &self,
        permit: CommitPermit,
    ) -> Vec<oneshot::Sender<Option<DerivativeReference>>> {
        let waiters = {
            let mut state = self.state.lock().await;
            let ticket = WorkTicket {
                job_id: permit.job_id,
                attempt: permit.attempt,
            };
            let Some(ticket_state) = state.tickets.get(&ticket) else {
                return Vec::new();
            };
            if ticket_state.status != TicketStatus::Committing
                || ticket_state.key.selection != permit.selection
            {
                return Vec::new();
            }
            let key = ticket_state.key.clone();
            let Some(work) = state.jobs.get(&key) else {
                state.tickets.remove(&ticket);
                return Vec::new();
            };
            if work.ticket != Some(ticket) || work.status != JobStatus::Committing {
                return Vec::new();
            }
            work.completion.notify_waiters();
            let work = state.jobs.remove(&key).expect("job was present");
            state.job_names.remove(&work.job_name);
            state.tickets.remove(&ticket);
            work.foreground_waiters
                .into_iter()
                .chain(work.background_waiters)
                .collect()
        };
        self.wake.notify_waiters();
        waiters
    }

    pub(crate) async fn note_recent(&self, asset_id: AssetId) {
        let mut state = self.state.lock().await;
        if let Some(position) = state
            .recent
            .iter()
            .position(|existing| *existing == asset_id)
        {
            state.recent.remove(position);
        }
        state.recent.push_back(asset_id);
        while state.recent.len() > RECENT_CAPACITY {
            state.recent.pop_front();
        }
    }

    pub(crate) async fn recent_ids(&self) -> Vec<AssetId> {
        self.state.lock().await.recent.iter().copied().collect()
    }

    pub(crate) async fn take_recent(&self) -> Vec<AssetId> {
        self.state.lock().await.recent.drain(..).collect()
    }

    pub(crate) async fn selection(&self) -> Option<SelectionToken> {
        self.state.lock().await.selection
    }

    pub(crate) async fn ensure_selection(&self, selection: SelectionToken) {
        if self.selection().await != Some(selection) {
            self.reset_selection(selection).await;
        }
    }

    pub(crate) async fn reset_selection(&self, selection: SelectionToken) {
        let waiters = {
            let mut state = self.state.lock().await;
            state.selection = Some(selection);
            state.background_generation = state.background_generation.wrapping_add(1);
            state.recent.clear();
            state.terminal.clear();
            state.collection = CollectionState {
                phase: CollectionPhase::Dormant,
                cursor: None,
            };
            let keys = state
                .jobs
                .iter()
                .filter(|(_, work)| work.status != JobStatus::Committing)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            let mut waiters = Vec::new();
            for key in keys {
                let work = state.jobs.remove(&key).expect("job was present");
                state.job_names.remove(&work.job_name);
                if let Some(ticket) = work.ticket {
                    state.tickets.remove(&ticket);
                }
                waiters.extend(work.foreground_waiters);
                waiters.extend(work.background_waiters);
            }
            waiters
        };
        for waiter in waiters {
            let _ = waiter.send(None);
        }
        self.wake.notify_waiters();
    }

    pub(crate) async fn begin_collection(&self, selection: SelectionToken) {
        let mut state = self.state.lock().await;
        if let Some(current) = state.selection {
            if current != selection {
                return;
            }
        } else {
            state.selection = Some(selection);
        }
        state.collection = CollectionState {
            phase: CollectionPhase::Thumbnails,
            cursor: None,
        };
        drop(state);
        self.wake.notify_waiters();
    }

    pub(crate) async fn advance_collection(
        &self,
        selection: SelectionToken,
        cursor: Option<WallCursorKey>,
    ) {
        let mut state = self.state.lock().await;
        if state.selection != Some(selection) {
            return;
        }
        if matches!(
            state.collection.phase,
            CollectionPhase::Thumbnails | CollectionPhase::Previews
        ) {
            state.collection.cursor = cursor;
        }
        drop(state);
        self.wake.notify_waiters();
    }

    pub(crate) async fn finish_thumbnail_phase(&self, selection: SelectionToken) {
        let mut state = self.state.lock().await;
        if state.selection != Some(selection) {
            return;
        }
        if state.collection.phase == CollectionPhase::Thumbnails {
            state.collection.phase = CollectionPhase::Previews;
            state.collection.cursor = None;
        }
        drop(state);
        self.wake.notify_waiters();
    }

    pub(crate) async fn finish_preview_phase(&self, selection: SelectionToken) {
        let mut state = self.state.lock().await;
        if state.selection != Some(selection) {
            return;
        }
        if state.collection.phase == CollectionPhase::Previews {
            state.collection.phase = CollectionPhase::Complete;
            state.collection.cursor = None;
        }
        drop(state);
        self.wake.notify_waiters();
    }

    pub(crate) async fn collection_state(&self) -> (CollectionPhase, Option<WallCursorKey>) {
        let state = self.state.lock().await;
        (state.collection.phase, state.collection.cursor.clone())
    }

    pub(crate) async fn collection_phase(&self) -> CollectionPhase {
        self.state.lock().await.collection.phase
    }

    pub(crate) async fn collection_cursor(&self) -> Option<WallCursorKey> {
        self.state.lock().await.collection.cursor.clone()
    }

    pub(crate) async fn mark_terminal(&self, ticket: WorkTicket) -> bool {
        let waiters = {
            let mut state = self.state.lock().await;
            let Some(ticket_state) = state.tickets.get(&ticket) else {
                return false;
            };
            if ticket_state.status != TicketStatus::Running {
                return false;
            }
            let key = ticket_state.key.clone();
            let Some(work) = state.jobs.get(&key) else {
                return false;
            };
            if work.ticket != Some(ticket) || work.status != JobStatus::Running {
                return false;
            }

            state.terminal.insert(key.clone());
            let work = state.jobs.remove(&key).expect("job was present");
            state.job_names.remove(&work.job_name);
            state.tickets.remove(&ticket);
            work.foreground_waiters
                .into_iter()
                .chain(work.background_waiters)
                .collect::<Vec<_>>()
        };
        for waiter in waiters {
            let _ = waiter.send(None);
        }
        self.wake.notify_waiters();
        true
    }

    pub(crate) async fn clear_terminal(&self, key: &WorkKey) {
        self.state.lock().await.terminal.remove(key);
    }

    pub(crate) async fn pending_job_count(&self) -> usize {
        self.state.lock().await.jobs.len()
    }

    pub(crate) async fn wait_for_change(&self) {
        let notified = self.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.pending_job_count().await == 0 {
            notified.await;
        }
    }

    pub(crate) fn wake(&self) {
        self.wake.notify_waiters();
    }
}

pub(crate) fn scheduler_priority(lane: WorkLane) -> JobPriority {
    match lane {
        WorkLane::IdlePreview => JobPriority::IdleLibrary,
        WorkLane::IdleWall => JobPriority::OpenCollection,
        WorkLane::NearWall => JobPriority::NearViewport,
        WorkLane::ViewerPreview => JobPriority::ViewerPreview,
        WorkLane::VisibleWall => JobPriority::Visible,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use photo_catalog::WallCursorKey;
    use photo_domain::{AssetId, FolderGroupId, LibraryId};
    use photo_indexer::{IndexScheduler, SchedulerConfig};

    use super::{
        CollectionPhase, DerivativeClass, DerivativeCoordinator, DerivativeReference, WorkKey,
        WorkLane,
    };
    use crate::service::SelectionToken;

    struct Fixture {
        coordinator: Arc<DerivativeCoordinator>,
        selection: SelectionToken,
    }

    impl Fixture {
        async fn enqueue(&self, key: WorkKey, lane: WorkLane) -> super::WorkResultReceiver {
            self.coordinator.enqueue(key, lane).await
        }
    }

    fn fixture() -> Fixture {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let selection = selection(1);
        Fixture {
            coordinator: Arc::new(DerivativeCoordinator::new(scheduler)),
            selection,
        }
    }

    fn selection(seed: u128) -> SelectionToken {
        SelectionToken {
            library_id: LibraryId::from_uuid(uuid::Uuid::from_u128(seed)),
            group_id: FolderGroupId::from_uuid(uuid::Uuid::from_u128(seed + 1)),
            epoch: seed as u64,
        }
    }

    fn fixture_id(index: u128) -> AssetId {
        AssetId::from_uuid(uuid::Uuid::from_u128(index + 100))
    }

    fn fixture_ids(range: std::ops::Range<u128>) -> Vec<AssetId> {
        range.map(fixture_id).collect()
    }

    fn screen_key_for(selection: SelectionToken, asset_id: AssetId, cache_key: &str) -> WorkKey {
        WorkKey {
            selection,
            asset_id,
            class: DerivativeClass::ScreenPreview,
            cache_key: cache_key.to_owned(),
        }
    }

    fn screen_key() -> WorkKey {
        screen_key_for(selection(1), fixture_id(1), "screen-v1")
    }

    fn reference() -> DerivativeReference {
        DerivativeReference {
            asset_id: fixture_id(1).as_uuid().hyphenated().to_string(),
            kind: DerivativeClass::ScreenPreview,
            key: "screen-v1".to_owned(),
        }
    }

    #[tokio::test]
    async fn recent_window_is_deduplicated_and_capped_at_250() {
        let fixture = fixture();
        for id in fixture_ids(0..300) {
            fixture.coordinator.note_recent(id).await;
        }
        fixture.coordinator.note_recent(fixture_id(299)).await;

        let recent = fixture.coordinator.recent_ids().await;
        assert_eq!(recent.len(), 250);
        assert_eq!(recent.first(), Some(&fixture_id(50)));
        assert_eq!(recent.last(), Some(&fixture_id(299)));
    }

    #[tokio::test]
    async fn foreground_request_promotes_current_background_job_and_waiter() {
        let fixture = fixture();
        let background = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;

        assert_eq!(
            fixture.coordinator.lane(ticket).await,
            WorkLane::ViewerPreview
        );
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(background.await.unwrap(), Some(reference()));
        assert_eq!(foreground.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn invalidated_background_result_cannot_complete_foreground_replacement() {
        let fixture = fixture();
        let stale = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let stale_ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.invalidate_background().await;
        let foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;

        fixture
            .coordinator
            .complete(stale_ticket, reference())
            .await;
        fixture.coordinator.discard(stale_ticket).await;
        let replacement = fixture.coordinator.next_work().await.unwrap();
        assert_ne!(replacement.attempt, stale_ticket.attempt);
        fixture.coordinator.complete(replacement, reference()).await;
        assert_eq!(stale.await.unwrap(), None);
        assert_eq!(foreground.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn stale_attempt_cannot_mark_foreground_replacement_terminal() {
        let fixture = fixture();
        let stale = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let stale_ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.invalidate_background().await;
        let foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;

        assert!(!fixture.coordinator.mark_terminal(stale_ticket).await);
        let replacement = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.complete(replacement, reference()).await;

        assert_eq!(stale.await.unwrap(), None);
        assert_eq!(foreground.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn wait_for_change_observes_enqueue_before_waiter_registration() {
        let fixture = fixture();
        assert!(fixture.coordinator.next_work().await.is_none());

        let waiter = fixture.coordinator.wait_for_change();
        let _queued = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;

        tokio::time::timeout(Duration::from_millis(50), waiter)
            .await
            .expect("enqueue notification was lost before wait registration");
    }

    #[tokio::test]
    async fn mismatched_attempt_cannot_complete_the_current_job() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::VisibleWall).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let mismatched = super::WorkTicket {
            job_id: ticket.job_id,
            attempt: ticket.attempt.wrapping_add(1),
        };
        let wrong_reference = DerivativeReference {
            asset_id: fixture_id(1).as_uuid().hyphenated().to_string(),
            kind: DerivativeClass::ScreenPreview,
            key: "wrong-attempt".to_owned(),
        };

        fixture
            .coordinator
            .complete(mismatched, wrong_reference)
            .await;
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(waiter.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn selection_reset_discards_old_jobs_and_recent_ids() {
        let fixture = fixture();
        let old = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        fixture.coordinator.note_recent(fixture_id(1)).await;
        fixture.coordinator.reset_selection(selection(2)).await;

        assert_eq!(old.await.unwrap(), None);
        assert!(fixture.coordinator.recent_ids().await.is_empty());
        assert!(fixture.coordinator.next_work().await.is_none());

        let fresh = WorkKey {
            selection: selection(2),
            asset_id: fixture_id(2),
            class: DerivativeClass::WallThumbnail,
            cache_key: "wall-v2".to_owned(),
        };
        let waiter = fixture.enqueue(fresh, WorkLane::VisibleWall).await;
        assert!(fixture.coordinator.next_work().await.is_some());
        assert_eq!(fixture.coordinator.selection().await, Some(selection(2)));
        drop(waiter);
    }

    #[tokio::test]
    async fn duplicate_full_work_key_coalesces_waiters_into_one_attempt() {
        let fixture = fixture();
        let first = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let second = fixture.enqueue(screen_key(), WorkLane::NearWall).await;

        let ticket = fixture.coordinator.next_work().await.unwrap();
        assert!(fixture.coordinator.next_work().await.is_none());
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(first.await.unwrap(), Some(reference()));
        assert_eq!(second.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn changed_cache_fingerprint_does_not_coalesce_work() {
        let fixture = fixture();
        let first = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let second_key = screen_key_for(fixture.selection, fixture_id(1), "screen-v2");
        let second = fixture.enqueue(second_key, WorkLane::IdlePreview).await;

        let first_ticket = fixture.coordinator.next_work().await.unwrap();
        let second_ticket = fixture.coordinator.next_work().await.unwrap();
        assert_ne!(first_ticket.job_id, second_ticket.job_id);
        fixture
            .coordinator
            .complete(first_ticket, reference())
            .await;
        fixture
            .coordinator
            .complete(second_ticket, reference())
            .await;
        assert_eq!(first.await.unwrap(), Some(reference()));
        assert_eq!(second.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn commit_requires_exact_stored_prerequisite() {
        let fixture = fixture();
        let waiter = fixture
            .coordinator
            .enqueue_with_prerequisite(
                screen_key(),
                WorkLane::VisibleWall,
                Some("wall-v1".to_owned()),
            )
            .await;
        let ticket = fixture.coordinator.next_work().await.unwrap();

        assert!(
            fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .is_none()
        );
        assert!(
            fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, Some("wall-v2"))
                .await
                .is_none()
        );
        assert!(
            fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, Some("wall-v1"))
                .await
                .is_some()
        );
        drop(waiter);
    }

    #[tokio::test]
    async fn foreground_commit_does_not_block_background_invalidation() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::VisibleWall).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let _permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();

        let coordinator = fixture.coordinator.clone();
        let invalidator = tokio::spawn(async move { coordinator.invalidate_background().await });
        tokio::time::timeout(Duration::from_millis(50), invalidator)
            .await
            .expect("foreground-only commit blocked background invalidation")
            .expect("background invalidation task panicked");
        drop(waiter);
    }

    #[tokio::test]
    async fn complete_commit_resolves_waiter_without_returned_sender() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::VisibleWall).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();

        fixture
            .coordinator
            .complete_commit(permit, reference())
            .await;

        assert_eq!(waiter.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn fail_commit_resolves_waiter_without_returned_sender() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::VisibleWall).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();

        fixture.coordinator.fail_commit(permit).await;

        assert_eq!(waiter.await.unwrap(), None);
    }

    #[tokio::test]
    async fn terminal_work_is_excluded_until_its_full_key_changes_or_is_cleared() {
        let fixture = fixture();
        let key = screen_key();
        let terminal_waiter = fixture.enqueue(key.clone(), WorkLane::IdlePreview).await;
        let terminal_ticket = fixture.coordinator.next_work().await.unwrap();
        assert!(fixture.coordinator.mark_terminal(terminal_ticket).await);
        assert_eq!(terminal_waiter.await.unwrap(), None);
        let blocked = fixture.enqueue(key.clone(), WorkLane::IdlePreview).await;
        assert!(fixture.coordinator.next_work().await.is_none());
        assert_eq!(blocked.await.unwrap(), None);

        let changed = screen_key_for(fixture.selection, fixture_id(1), "screen-v2");
        let eligible = fixture.enqueue(changed, WorkLane::IdlePreview).await;
        assert!(fixture.coordinator.next_work().await.is_some());
        drop(eligible);

        fixture.coordinator.clear_terminal(&key).await;
        let retry = fixture.enqueue(key, WorkLane::IdlePreview).await;
        assert!(fixture.coordinator.next_work().await.is_some());
        drop(retry);
    }

    #[tokio::test]
    async fn collection_cursor_advances_through_thumbnail_and_preview_phases() {
        let fixture = fixture();
        fixture
            .coordinator
            .begin_collection(fixture.selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, None)
        );

        let thumbnail_cursor = WallCursorKey::Provisional {
            order: 2,
            id: fixture_id(2),
        };
        fixture
            .coordinator
            .advance_collection(fixture.selection, Some(thumbnail_cursor.clone()))
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, Some(thumbnail_cursor))
        );
        fixture
            .coordinator
            .finish_thumbnail_phase(fixture.selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Previews, None)
        );

        let preview_cursor = WallCursorKey::Captured {
            captured_at_utc: "2026-08-28T00:00:00Z".to_owned(),
            display_path: "photo.jpg".to_owned(),
            id: fixture_id(3),
        };
        fixture
            .coordinator
            .advance_collection(fixture.selection, Some(preview_cursor.clone()))
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Previews, Some(preview_cursor))
        );
        fixture
            .coordinator
            .finish_preview_phase(fixture.selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Complete, None)
        );
    }

    #[tokio::test]
    async fn stale_collection_selection_cannot_change_new_selection() {
        let fixture = fixture();
        let old_selection = fixture.selection;
        let new_selection = selection(2);
        fixture.coordinator.begin_collection(old_selection).await;
        fixture.coordinator.reset_selection(new_selection).await;
        fixture.coordinator.begin_collection(old_selection).await;
        assert_eq!(fixture.coordinator.selection().await, Some(new_selection));
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Dormant, None)
        );

        let stale_cursor = WallCursorKey::Provisional {
            order: 4,
            id: fixture_id(4),
        };
        fixture
            .coordinator
            .advance_collection(old_selection, Some(stale_cursor))
            .await;
        fixture
            .coordinator
            .finish_thumbnail_phase(old_selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Dormant, None)
        );

        fixture.coordinator.begin_collection(new_selection).await;
        fixture
            .coordinator
            .finish_thumbnail_phase(old_selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, None)
        );
        fixture
            .coordinator
            .finish_thumbnail_phase(new_selection)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Previews, None)
        );
    }

    #[tokio::test]
    async fn visible_promotion_over_near_work_keeps_one_job() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::NearWall).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let promoted_waiter = fixture.enqueue(screen_key(), WorkLane::VisibleWall).await;

        assert_eq!(
            fixture.coordinator.lane(ticket).await,
            WorkLane::VisibleWall
        );
        assert!(fixture.coordinator.next_work().await.is_none());
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(waiter.await.unwrap(), Some(reference()));
        assert_eq!(promoted_waiter.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn admitted_background_commit_orders_before_invalidation_returns() {
        let fixture = fixture();
        let waiter = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();

        let invalidator = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.invalidate_background().await })
        };
        tokio::task::yield_now().await;
        assert!(!invalidator.is_finished());

        fixture
            .coordinator
            .complete_commit(permit, reference())
            .await;
        invalidator.await.unwrap();
        assert_eq!(waiter.await.unwrap(), Some(reference()));
    }
}
