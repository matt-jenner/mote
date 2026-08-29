#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering as AtomicOrdering};

use crate::dto::{DerivativeClass, DerivativeReference};
use crate::service::SelectionToken;
use photo_catalog::WallCursorKey;
use photo_domain::{AssetId, Availability};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkKey {
    pub(crate) selection: SelectionToken,
    pub(crate) asset_id: AssetId,
    pub(crate) class: DerivativeClass,
    pub(crate) cache_key: String,
    pub(crate) availability: Availability,
}

impl Hash for WorkKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.selection.hash(state);
        self.asset_id.hash(state);
        self.class.hash(state);
        self.cache_key.hash(state);
        let availability = match self.availability {
            Availability::Available => 0_u8,
            Availability::RootOffline => 1,
            Availability::Missing => 2,
            Availability::Unreadable => 3,
        };
        availability.hash(state);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WorkTicket {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct CommitPermit {
    pub(crate) job_id: u64,
    pub(crate) attempt: u64,
    pub(crate) selection: SelectionToken,
    supervisor_state: Arc<AtomicU8>,
}

impl CommitPermit {
    pub(crate) fn claim_supervisor(&self) -> bool {
        self.supervisor_state
            .compare_exchange(0, 1, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_ok()
    }

    fn claim_abort(&self) -> bool {
        self.supervisor_state
            .compare_exchange(0, 2, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_ok()
    }
}

pub(crate) type WorkResultReceiver = oneshot::Receiver<Option<DerivativeReference>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionPhase {
    Dormant,
    Thumbnails,
    Previews,
    Complete,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoordinatorTestSnapshot {
    pub recent_len: usize,
    pub largest_loaded_page: usize,
    pub queued_jobs: usize,
    pub phase: CollectionPhase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CollectionState {
    phase: CollectionPhase,
    cursor: Option<WallCursorKey>,
}

/// An exact snapshot of the collection driver's position.
///
/// Collection work may spend a long time resolving a page.  The driver must
/// present this complete snapshot when it advances or finishes so a visible
/// request, selection change, or another generation cannot mutate the new
/// collection state with an old cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CollectionProgressToken {
    pub(crate) selection: SelectionToken,
    pub(crate) phase: CollectionPhase,
    pub(crate) cursor: Option<WallCursorKey>,
    pub(crate) background_generation: u64,
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
    commit_finishing: bool,
    commit_result: Option<Option<DerivativeReference>>,
    commit_supervisor_state: Arc<AtomicU8>,
    prerequisite_key: Option<String>,
    foreground_waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
    background_waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
    completion: Arc<Notify>,
}

#[derive(Clone)]
struct CompletedCommit {
    background_generation: u64,
    completion_generation: u64,
    result: Option<DerivativeReference>,
}

struct CommitDelivery {
    permit: CommitPermit,
    key: WorkKey,
    completion: Arc<Notify>,
    result: Option<DerivativeReference>,
    waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
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
    completed: HashMap<WorkKey, VecDeque<CompletedCommit>>,
    completed_order: VecDeque<(WorkKey, u64)>,
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
            completed: HashMap::new(),
            completed_order: VecDeque::new(),
            terminal: HashSet::new(),
            collection: CollectionState {
                phase: CollectionPhase::Dormant,
                cursor: None,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct DerivativeCoordinator {
    state: Arc<Mutex<CoordinatorState>>,
    scheduler: Arc<IndexScheduler>,
    owner_prefix: String,
    wake: Arc<Notify>,
    change_generation: Arc<AtomicU64>,
    commit_completion_generation: Arc<AtomicU64>,
    #[cfg(debug_assertions)]
    largest_loaded_page: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    wait_test_hook: Arc<Mutex<Option<Arc<Notify>>>>,
    #[cfg(any(test, debug_assertions))]
    invalidation_wait_test_hook: Arc<Mutex<Option<Arc<Notify>>>>,
    #[cfg(any(test, debug_assertions))]
    commit_waiter_delivery_test_gate: Arc<Mutex<Option<CommitWaiterDeliveryTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    commit_waiter_snapshot_test_gate: Arc<Mutex<Option<CommitWaiterDeliveryTestGate>>>,
    #[cfg(any(test, debug_assertions))]
    commit_outcome_publication_test_gate: Arc<Mutex<Option<CommitWaiterDeliveryTestGate>>>,
}

#[cfg(any(test, debug_assertions))]
struct CommitWaiterDeliveryTestGate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl DerivativeCoordinator {
    pub(crate) fn new(scheduler: Arc<IndexScheduler>) -> Self {
        Self {
            state: Arc::new(Mutex::new(CoordinatorState::default())),
            scheduler,
            owner_prefix: SCHEDULER_OWNER_PREFIX.to_owned(),
            wake: Arc::new(Notify::new()),
            change_generation: Arc::new(AtomicU64::new(0)),
            commit_completion_generation: Arc::new(AtomicU64::new(0)),
            #[cfg(debug_assertions)]
            largest_loaded_page: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            wait_test_hook: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            invalidation_wait_test_hook: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_waiter_delivery_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_waiter_snapshot_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_outcome_publication_test_gate: Arc::new(Mutex::new(None)),
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
            state: Arc::new(Mutex::new(state)),
            scheduler,
            owner_prefix: format!(
                "{SCHEDULER_OWNER_PREFIX}selection-{}:",
                selection.group_id.as_uuid().hyphenated()
            ),
            wake: Arc::new(Notify::new()),
            change_generation: Arc::new(AtomicU64::new(0)),
            commit_completion_generation: Arc::new(AtomicU64::new(0)),
            #[cfg(debug_assertions)]
            largest_loaded_page: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            wait_test_hook: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            invalidation_wait_test_hook: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_waiter_delivery_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_waiter_snapshot_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(any(test, debug_assertions))]
            commit_outcome_publication_test_gate: Arc::new(Mutex::new(None)),
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
        self.enqueue_with_prerequisite_observed(key, lane, prerequisite_key, None)
            .await
    }

    pub(crate) async fn enqueue_with_prerequisite_observed(
        &self,
        key: WorkKey,
        lane: WorkLane,
        prerequisite_key: Option<String>,
        observed_completion_generation: Option<u64>,
    ) -> WorkResultReceiver {
        self.enqueue_with_prerequisite_observed_guarded(
            key,
            lane,
            prerequisite_key,
            observed_completion_generation,
            None,
        )
        .await
    }

    /// Enqueues collection-owned background work only when the caller still
    /// owns the exact collection position it resolved.  The token check and
    /// job insertion share one coordinator lock, so invalidation cannot slip
    /// between the check and the generation stamp on a queued job.
    pub(crate) async fn enqueue_collection_with_prerequisite_observed(
        &self,
        token: &CollectionProgressToken,
        key: WorkKey,
        lane: WorkLane,
        prerequisite_key: Option<String>,
        observed_completion_generation: Option<u64>,
    ) -> WorkResultReceiver {
        self.enqueue_with_prerequisite_observed_guarded(
            key,
            lane,
            prerequisite_key,
            observed_completion_generation,
            Some(token),
        )
        .await
    }

    async fn enqueue_with_prerequisite_observed_guarded(
        &self,
        key: WorkKey,
        lane: WorkLane,
        prerequisite_key: Option<String>,
        observed_completion_generation: Option<u64>,
        collection_token: Option<&CollectionProgressToken>,
    ) -> WorkResultReceiver {
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        let mut schedule = None;
        let mut immediate_none = false;
        let mut immediate_result = None;
        {
            let mut state = self.state.lock().await;
            if collection_token.is_some_and(|token| {
                token.selection != key.selection || !Self::collection_token_matches(&state, token)
            }) {
                immediate_none = true;
            }
            if !immediate_none {
                if let Some(selection) = state.selection {
                    if selection != key.selection {
                        immediate_none = true;
                    }
                } else {
                    state.selection = Some(key.selection);
                }
            }

            if !immediate_none {
                if let Some(completed) = observed_completion_generation
                    .and_then(|observed| completed_after(&state, &key, observed))
                {
                    immediate_result = Some(completed.result);
                } else if state.terminal.contains(&key) {
                    immediate_none = true;
                } else if state.jobs.contains_key(&key) {
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
                            if work.status == JobStatus::Committing && work.commit_finishing {
                                immediate_result = work.commit_result.clone();
                                if immediate_result.is_none() {
                                    immediate_none = true;
                                }
                            } else {
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
                } else if observed_completion_generation.is_none()
                    && let Some(completed) = current_completed(&state, &key)
                {
                    immediate_result = Some(completed.result);
                } else {
                    queue_new_job(
                        &mut state,
                        key,
                        lane,
                        prerequisite_key,
                        &mut sender,
                        &mut schedule,
                        &self.owner_prefix,
                    );
                }
            }
        }
        if immediate_none {
            let _ = sender
                .take()
                .expect("rejected waiter was not consumed")
                .send(None);
        } else if let Some(result) = immediate_result {
            let _ = sender
                .take()
                .expect("completed waiter was not consumed")
                .send(result);
        }
        if let Some((job_name, priority)) = schedule {
            self.scheduler
                .enqueue(IndexJob::new(job_name, priority))
                .await;
        }
        self.notify_waiters();
        receiver
    }

    pub(crate) async fn next_work(&self) -> Option<WorkTicket> {
        loop {
            let job = self.scheduler.next_owned(&self.owner_prefix).await?;
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
                self.notify_waiters();
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
        self.notify_waiters();
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
        self.notify_waiters();
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
            #[cfg(any(test, debug_assertions))]
            if !completion_waits.is_empty()
                && let Some(hook) = self.invalidation_wait_test_hook.lock().await.take()
            {
                hook.notify_one();
            }
            for waiter in waiters {
                let _ = waiter.send(None);
            }
            self.notify_waiters();
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
        let supervisor_state = work.commit_supervisor_state.clone();
        work.commit_background = work.lane.is_background() && work.foreground_waiters.is_empty();
        work.status = JobStatus::Committing;
        if let Some(ticket_state) = state.tickets.get_mut(&ticket) {
            ticket_state.status = TicketStatus::Committing;
        }
        Some(CommitPermit {
            job_id: ticket.job_id,
            attempt: ticket.attempt,
            selection,
            supervisor_state,
        })
    }

    pub(crate) async fn complete_commit(
        &self,
        permit: CommitPermit,
        reference: DerivativeReference,
    ) {
        let Some(delivery) = self.begin_commit_delivery(permit, Some(reference)).await else {
            return;
        };
        let coordinator = self.clone();
        let delivery_task = tokio::spawn(async move {
            coordinator.deliver_commit(delivery).await;
        });
        let _ = delivery_task.await;
    }

    pub(crate) async fn fail_commit(&self, permit: CommitPermit) {
        let Some(delivery) = self.begin_commit_delivery(permit, None).await else {
            return;
        };
        let coordinator = self.clone();
        let delivery_task = tokio::spawn(async move {
            coordinator.deliver_commit(delivery).await;
        });
        let _ = delivery_task.await;
    }

    /// Finishes a committed attempt as an asset-terminal outcome.  Unlike a failed
    /// commit this also remembers the exact work key, so a duplicate request for
    /// the same source signature is answered without scheduling another attempt.
    pub(crate) async fn terminal_commit(&self, permit: CommitPermit) -> bool {
        let (waiters, completion) = {
            let mut state = self.state.lock().await;
            let ticket = WorkTicket {
                job_id: permit.job_id,
                attempt: permit.attempt,
            };
            let Some(ticket_state) = state.tickets.get(&ticket) else {
                return false;
            };
            if ticket_state.status != TicketStatus::Committing
                || ticket_state.key.selection != permit.selection
            {
                return false;
            }
            let key = ticket_state.key.clone();
            let Some(work) = state.jobs.get(&key) else {
                return false;
            };
            if work.ticket != Some(ticket) || work.status != JobStatus::Committing {
                return false;
            }
            remember_terminal(&mut state, key.clone());
            let work = state.jobs.remove(&key).expect("job was present");
            state.job_names.remove(&work.job_name);
            state.tickets.remove(&ticket);
            let completion = work.completion.clone();
            let waiters = work
                .foreground_waiters
                .into_iter()
                .chain(work.background_waiters)
                .collect::<Vec<_>>();
            (waiters, completion)
        };
        for waiter in waiters {
            let _ = waiter.send(None);
        }
        completion.notify_waiters();
        self.notify_waiters();
        true
    }

    pub(crate) async fn abort_attempt(&self, ticket: WorkTicket) {
        let status = {
            let state = self.state.lock().await;
            state.tickets.get(&ticket).map(|ticket_state| {
                let supervisor_state = state
                    .jobs
                    .get(&ticket_state.key)
                    .map(|work| work.commit_supervisor_state.clone())
                    .unwrap_or_else(|| Arc::new(AtomicU8::new(0)));
                (
                    ticket_state.status,
                    ticket_state.key.selection,
                    supervisor_state,
                )
            })
        };
        match status {
            Some((TicketStatus::Running, _, _)) => {
                self.discard(ticket).await;
            }
            Some((TicketStatus::Committing, selection, supervisor_state)) => {
                let permit = CommitPermit {
                    job_id: ticket.job_id,
                    attempt: ticket.attempt,
                    selection,
                    supervisor_state,
                };
                if permit.claim_abort() {
                    self.fail_commit(permit).await;
                }
            }
            None => {}
        }
    }

    pub(crate) async fn abort_all_attempts(&self) {
        let tickets = self
            .state
            .lock()
            .await
            .tickets
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for ticket in tickets {
            self.abort_attempt(ticket).await;
        }
    }

    async fn begin_commit_delivery(
        &self,
        permit: CommitPermit,
        result: Option<DerivativeReference>,
    ) -> Option<CommitDelivery> {
        let delivery = {
            let mut state = self.state.lock().await;
            let ticket = WorkTicket {
                job_id: permit.job_id,
                attempt: permit.attempt,
            };
            let ticket_state = state.tickets.get(&ticket)?;
            if ticket_state.status != TicketStatus::Committing
                || ticket_state.key.selection != permit.selection
            {
                return None;
            }
            let key = ticket_state.key.clone();
            let work = state.jobs.get_mut(&key)?;
            if work.ticket != Some(ticket)
                || work.status != JobStatus::Committing
                || work.commit_finishing
            {
                return None;
            }
            work.commit_result = Some(result.clone());
            work.commit_finishing = true;
            CommitDelivery {
                permit,
                key,
                completion: work.completion.clone(),
                result,
                waiters: work
                    .foreground_waiters
                    .drain(..)
                    .chain(work.background_waiters.drain(..))
                    .collect(),
            }
        };
        self.notify_waiters();
        Some(delivery)
    }

    async fn deliver_commit(&self, delivery: CommitDelivery) {
        let CommitDelivery {
            permit,
            key,
            completion,
            result,
            waiters,
        } = delivery;
        #[cfg(any(test, debug_assertions))]
        if let Some(gate) = self.commit_waiter_snapshot_test_gate.lock().await.take() {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
        #[cfg(any(test, debug_assertions))]
        if let Some(gate) = self.commit_waiter_delivery_test_gate.lock().await.take() {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
        self.finish_commit(CommitDelivery {
            permit,
            key,
            completion,
            result,
            waiters: Vec::new(),
        })
        .await;
    }

    async fn finish_commit(&self, delivery: CommitDelivery) {
        let finished = {
            let mut state = self.state.lock().await;
            let ticket = WorkTicket {
                job_id: delivery.permit.job_id,
                attempt: delivery.permit.attempt,
            };
            let Some(ticket_state) = state.tickets.get(&ticket) else {
                return;
            };
            if ticket_state.status != TicketStatus::Committing
                || ticket_state.key.selection != delivery.permit.selection
            {
                return;
            }
            let Some(work) = state.jobs.get(&delivery.key) else {
                state.tickets.remove(&ticket);
                return;
            };
            if work.ticket != Some(ticket)
                || work.status != JobStatus::Committing
                || !work.commit_finishing
            {
                return;
            }
            let work = state.jobs.remove(&delivery.key).expect("job was present");
            state.job_names.remove(&work.job_name);
            state.tickets.remove(&ticket);
            let completion_generation = self
                .commit_completion_generation
                .load(AtomicOrdering::Acquire)
                .wrapping_add(1);
            if state.selection == Some(delivery.permit.selection) {
                remember_completed(
                    &mut state,
                    delivery.key.clone(),
                    work.background_generation,
                    completion_generation,
                    delivery.result.clone(),
                );
            }
            #[cfg(any(test, debug_assertions))]
            if let Some(gate) = self
                .commit_outcome_publication_test_gate
                .lock()
                .await
                .take()
            {
                let release = gate.release.notified();
                tokio::pin!(release);
                release.as_mut().enable();
                gate.entered.notify_one();
                release.await;
            }
            self.commit_completion_generation
                .store(completion_generation, AtomicOrdering::Release);
            delivery.completion.notify_waiters();
            true
        };
        if finished {
            self.notify_waiters();
        }
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

    pub(crate) async fn remove_recent(&self, consumed: &HashSet<AssetId>) {
        self.state
            .lock()
            .await
            .recent
            .retain(|asset_id| !consumed.contains(asset_id));
    }

    pub(crate) async fn remove_recent_if_collection_current(
        &self,
        token: &CollectionProgressToken,
        consumed: &HashSet<AssetId>,
    ) -> bool {
        let mut state = self.state.lock().await;
        if !Self::collection_token_matches(&state, token) {
            return false;
        }
        state.recent.retain(|asset_id| !consumed.contains(asset_id));
        true
    }

    pub(crate) async fn selection(&self) -> Option<SelectionToken> {
        self.state.lock().await.selection
    }

    pub(crate) async fn ensure_selection(&self, selection: SelectionToken) {
        self.reset_selection(selection).await;
    }

    pub(crate) async fn reset_selection(&self, selection: SelectionToken) {
        let waiters = {
            let mut state = self.state.lock().await;
            if state
                .selection
                .is_some_and(|current| current.epoch >= selection.epoch)
            {
                return;
            }
            state.selection = Some(selection);
            state.background_generation = state.background_generation.wrapping_add(1);
            state.recent.clear();
            state.completed.clear();
            state.completed_order.clear();
            state.terminal.clear();
            state.collection = CollectionState {
                phase: CollectionPhase::Dormant,
                cursor: None,
            };
            #[cfg(debug_assertions)]
            self.largest_loaded_page.store(0, AtomicOrdering::Release);
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
        self.notify_waiters();
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
        if state.collection.phase == CollectionPhase::Dormant {
            state.collection = CollectionState {
                phase: CollectionPhase::Thumbnails,
                cursor: None,
            };
        }
        drop(state);
        self.notify_waiters();
    }

    pub(crate) async fn rewind_collection_to_thumbnails(&self, selection: SelectionToken) {
        let changed = {
            let mut state = self.state.lock().await;
            if state.selection != Some(selection) {
                false
            } else if state.collection
                != (CollectionState {
                    phase: CollectionPhase::Thumbnails,
                    cursor: None,
                })
            {
                state.collection = CollectionState {
                    phase: CollectionPhase::Thumbnails,
                    cursor: None,
                };
                true
            } else {
                false
            }
        };
        if changed {
            self.notify_waiters();
        }
    }

    pub(crate) async fn reset_collection_for_scope(&self, selection: SelectionToken) {
        let changed = {
            let mut state = self.state.lock().await;
            if state.selection != Some(selection) {
                false
            } else {
                state.recent.clear();
                state.collection = CollectionState {
                    phase: CollectionPhase::Thumbnails,
                    cursor: None,
                };
                true
            }
        };
        if changed {
            self.notify_waiters();
        }
    }

    pub(crate) async fn collection_progress_token(
        &self,
        selection: SelectionToken,
    ) -> Option<CollectionProgressToken> {
        let state = self.state.lock().await;
        (state.selection == Some(selection)).then(|| CollectionProgressToken {
            selection,
            phase: state.collection.phase,
            cursor: state.collection.cursor.clone(),
            background_generation: state.background_generation,
        })
    }

    pub(crate) async fn collection_progress_token_is_current(
        &self,
        token: &CollectionProgressToken,
    ) -> bool {
        let state = self.state.lock().await;
        Self::collection_token_matches(&state, token)
    }

    /// Runs a small synchronous collection side effect under the same
    /// linearization boundary as invalidation and cursor transitions.  Once
    /// admitted, invalidation is ordered after the side effect; if the token
    /// is already stale, the closure is never called.
    pub(crate) async fn collection_side_effect_if_current<T>(
        &self,
        token: &CollectionProgressToken,
        effect: impl FnOnce() -> T,
    ) -> Option<T> {
        let state = self.state.lock().await;
        if !Self::collection_token_matches(&state, token) {
            return None;
        }
        Some(effect())
    }

    fn collection_token_matches(state: &CoordinatorState, token: &CollectionProgressToken) -> bool {
        state.selection == Some(token.selection)
            && state.background_generation == token.background_generation
            && state.collection.phase == token.phase
            && state.collection.cursor == token.cursor
    }

    pub(crate) async fn advance_collection(
        &self,
        token: &CollectionProgressToken,
        cursor: Option<WallCursorKey>,
    ) -> bool {
        let changed = {
            let mut state = self.state.lock().await;
            if !Self::collection_token_matches(&state, token)
                || !matches!(
                    state.collection.phase,
                    CollectionPhase::Thumbnails | CollectionPhase::Previews
                )
            {
                false
            } else {
                state.collection.cursor = cursor;
                true
            }
        };
        if changed {
            self.notify_waiters();
        }
        changed
    }

    pub(crate) async fn finish_thumbnail_phase(&self, token: &CollectionProgressToken) -> bool {
        let changed = {
            let mut state = self.state.lock().await;
            if !Self::collection_token_matches(&state, token)
                || state.collection.phase != CollectionPhase::Thumbnails
            {
                false
            } else {
                // The phase transition and cursor reset are one state change;
                // no preview driver can observe a partially advanced state.
                state.collection = CollectionState {
                    phase: CollectionPhase::Previews,
                    cursor: None,
                };
                true
            }
        };
        if changed {
            self.notify_waiters();
        }
        changed
    }

    pub(crate) async fn finish_preview_phase(&self, token: &CollectionProgressToken) -> bool {
        let changed = {
            let mut state = self.state.lock().await;
            if !Self::collection_token_matches(&state, token)
                || state.collection.phase != CollectionPhase::Previews
            {
                false
            } else {
                // As above, Complete and the empty cursor are committed
                // together so stale preview drivers cannot strand a cursor.
                state.collection = CollectionState {
                    phase: CollectionPhase::Complete,
                    cursor: None,
                };
                true
            }
        };
        if changed {
            self.notify_waiters();
        }
        changed
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

    #[cfg(debug_assertions)]
    pub(crate) fn note_loaded_page(&self, page_size: usize) {
        self.largest_loaded_page
            .fetch_max(page_size, AtomicOrdering::AcqRel);
    }

    #[cfg(debug_assertions)]
    pub(crate) async fn test_snapshot(&self) -> CoordinatorTestSnapshot {
        let state = self.state.lock().await;
        CoordinatorTestSnapshot {
            recent_len: state.recent.len(),
            largest_loaded_page: self.largest_loaded_page.load(AtomicOrdering::Acquire),
            queued_jobs: state.jobs.len(),
            phase: state.collection.phase,
        }
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

            remember_terminal(&mut state, key.clone());
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
        self.notify_waiters();
        true
    }

    pub(crate) async fn clear_terminal(&self, key: &WorkKey) {
        self.state.lock().await.terminal.remove(key);
    }

    pub(crate) async fn pending_job_count(&self) -> usize {
        self.state.lock().await.jobs.len()
    }

    #[cfg(any(test, debug_assertions))]
    async fn completed_outcome_count(&self) -> usize {
        self.state.lock().await.completed_order.len()
    }

    pub(crate) fn commit_completion_generation(&self) -> u64 {
        self.commit_completion_generation
            .load(AtomicOrdering::Acquire)
    }

    pub(crate) async fn wait_for_change(&self) {
        let observed = self.change_generation();
        self.wait_for_change_since(observed).await;
    }

    pub(crate) async fn wait_for_change_since(&self, observed: u64) {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.change_generation() != observed {
                return;
            }
            #[cfg(test)]
            if let Some(hook) = self.wait_test_hook.lock().await.clone() {
                hook.notify_one();
            }
            notified.await;
        }
    }

    pub(crate) fn change_generation(&self) -> u64 {
        self.change_generation.load(AtomicOrdering::Acquire)
    }

    fn notify_waiters(&self) {
        self.change_generation.fetch_add(1, AtomicOrdering::AcqRel);
        self.wake.notify_waiters();
    }

    pub(crate) fn wake(&self) {
        self.notify_waiters();
    }

    #[cfg(test)]
    pub(crate) async fn install_wait_test_hook(&self, entered: Arc<Notify>) {
        *self.wait_test_hook.lock().await = Some(entered);
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) async fn install_invalidation_wait_test_hook(&self, entered: Arc<Notify>) {
        *self.invalidation_wait_test_hook.lock().await = Some(entered);
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) async fn install_commit_waiter_delivery_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.commit_waiter_delivery_test_gate.lock().await =
            Some(CommitWaiterDeliveryTestGate { entered, release });
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) async fn install_commit_waiter_snapshot_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.commit_waiter_snapshot_test_gate.lock().await =
            Some(CommitWaiterDeliveryTestGate { entered, release });
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) async fn install_commit_outcome_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.commit_outcome_publication_test_gate.lock().await =
            Some(CommitWaiterDeliveryTestGate { entered, release });
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

fn completed_after(
    state: &CoordinatorState,
    key: &WorkKey,
    observed_generation: u64,
) -> Option<CompletedCommit> {
    state.completed.get(key).and_then(|history| {
        history
            .iter()
            .find(|completed| completed.completion_generation > observed_generation)
            .cloned()
    })
}

fn remember_terminal(state: &mut CoordinatorState, key: WorkKey) {
    state.terminal.insert(key);
    while state.terminal.len() > RECENT_CAPACITY {
        let Some(oldest) = state.terminal.iter().next().cloned() else {
            break;
        };
        state.terminal.remove(&oldest);
    }
}

fn current_completed(state: &CoordinatorState, key: &WorkKey) -> Option<CompletedCommit> {
    state.completed.get(key).and_then(|history| {
        history
            .iter()
            .rev()
            .find(|completed| completed.background_generation == state.background_generation)
            .cloned()
    })
}

fn remember_completed(
    state: &mut CoordinatorState,
    key: WorkKey,
    background_generation: u64,
    completion_generation: u64,
    result: Option<DerivativeReference>,
) {
    state
        .completed
        .entry(key.clone())
        .or_default()
        .push_back(CompletedCommit {
            background_generation,
            completion_generation,
            result,
        });
    state
        .completed_order
        .push_back((key, completion_generation));
    while state.completed_order.len() > RECENT_CAPACITY {
        let Some((oldest_key, oldest_generation)) = state.completed_order.pop_front() else {
            break;
        };
        if let Some(history) = state.completed.get_mut(&oldest_key) {
            if let Some(position) = history
                .iter()
                .position(|completed| completed.completion_generation == oldest_generation)
            {
                history.remove(position);
            }
            if history.is_empty() {
                state.completed.remove(&oldest_key);
            }
        }
    }
}

fn queue_new_job(
    state: &mut CoordinatorState,
    key: WorkKey,
    lane: WorkLane,
    prerequisite_key: Option<String>,
    sender: &mut Option<oneshot::Sender<Option<DerivativeReference>>>,
    schedule: &mut Option<(String, JobPriority)>,
    owner_prefix: &str,
) {
    let job_id = state.next_job_id;
    state.next_job_id = state.next_job_id.wrapping_add(1).max(1);
    let job_name = format!("{owner_prefix}{job_id}");
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
            commit_finishing: false,
            commit_result: None,
            commit_supervisor_state: Arc::new(AtomicU8::new(0)),
            prerequisite_key,
            foreground_waiters,
            background_waiters,
            completion: Arc::new(Notify::new()),
        },
    );
    state.job_names.insert(job_name.clone(), key);
    *schedule = Some((job_name, scheduler_priority(lane)));
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;

    use photo_catalog::WallCursorKey;
    use photo_domain::{AssetId, Availability, FolderGroupId, LibraryId};
    use photo_indexer::{IndexScheduler, SchedulerConfig};
    use tokio::sync::Notify;

    use super::{
        CollectionPhase, DerivativeClass, DerivativeCoordinator, DerivativeReference, WorkKey,
        WorkLane,
    };
    use crate::service::SelectionToken;

    struct Fixture {
        coordinator: Arc<DerivativeCoordinator>,
        scheduler: Arc<IndexScheduler>,
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
            coordinator: Arc::new(DerivativeCoordinator::new(scheduler.clone())),
            scheduler,
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
            availability: Availability::Available,
        }
    }

    fn screen_key() -> WorkKey {
        screen_key_for(selection(1), fixture_id(1), "screen-v1")
    }

    fn reference() -> DerivativeReference {
        reference_for(fixture_id(1), DerivativeClass::ScreenPreview, "screen-v1")
    }

    fn reference_for(asset_id: AssetId, kind: DerivativeClass, key: &str) -> DerivativeReference {
        DerivativeReference {
            asset_id: asset_id.as_uuid().hyphenated().to_string(),
            kind,
            key: key.to_owned(),
        }
    }

    #[tokio::test]
    async fn recent_window_is_deduplicated_and_capped_at_250() {
        let fixture = fixture();
        for id in fixture_ids(0..10_000) {
            fixture.coordinator.note_recent(id).await;
        }
        fixture.coordinator.note_recent(fixture_id(9_999)).await;

        let recent = fixture.coordinator.recent_ids().await;
        assert_eq!(recent.len(), 250);
        assert_eq!(recent.first(), Some(&fixture_id(9_750)));
        assert_eq!(recent.last(), Some(&fixture_id(9_999)));
    }

    #[tokio::test]
    async fn removing_consumed_recent_ids_preserves_newer_entries() {
        let fixture = fixture();
        let consumed = fixture_id(1);
        let newer = fixture_id(2);
        fixture.coordinator.note_recent(consumed).await;
        fixture.coordinator.note_recent(newer).await;

        fixture
            .coordinator
            .remove_recent(&HashSet::from([consumed]))
            .await;

        assert_eq!(fixture.coordinator.recent_ids().await, vec![newer]);
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
    async fn terminal_commit_releases_invalidation_waiters() {
        let fixture = fixture();
        let waiter = fixture
            .coordinator
            .enqueue(screen_key(), WorkLane::IdlePreview)
            .await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .expect("the background attempt should be admitted");
        let coordinator = fixture.coordinator.clone();
        let invalidation = tokio::spawn(async move {
            coordinator.invalidate_background().await;
        });
        fixture.coordinator.terminal_commit(permit).await;
        tokio::time::timeout(Duration::from_secs(1), invalidation)
            .await
            .expect("terminal commit must release invalidation")
            .unwrap();
        assert_eq!(waiter.await.unwrap(), None);
    }

    #[tokio::test]
    async fn wait_for_change_observes_enqueue_before_waiter_registration() {
        let fixture = fixture();
        assert!(fixture.coordinator.next_work().await.is_none());

        let observed = fixture.coordinator.change_generation();
        let waiter = fixture.coordinator.wait_for_change_since(observed);
        let _queued = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;

        tokio::time::timeout(Duration::from_millis(50), waiter)
            .await
            .expect("enqueue notification was lost before wait registration");
    }

    #[tokio::test]
    async fn wait_for_change_waits_on_a_paused_idle_job_until_interaction_ends() {
        let fixture = fixture();
        fixture
            .scheduler
            .set_interaction_mode(photo_indexer::InteractionMode::Active)
            .await;
        let waiter = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let coordinator = fixture.coordinator.clone();
        let waiting = tokio::spawn(async move {
            coordinator.wait_for_change().await;
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !waiting.is_finished(),
            "paused idle work caused a busy spin"
        );

        fixture
            .scheduler
            .set_interaction_mode(photo_indexer::InteractionMode::Idle)
            .await;
        fixture.coordinator.wake();
        tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .expect("interaction transition did not wake the wait")
            .expect("wait task panicked");

        let ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.discard(ticket).await;
        assert_eq!(waiter.await.unwrap(), None);
    }

    #[tokio::test]
    async fn wait_for_change_wakes_when_paused_idle_work_is_promoted() {
        let fixture = fixture();
        fixture
            .scheduler
            .set_interaction_mode(photo_indexer::InteractionMode::Active)
            .await;
        let waiter = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let coordinator = fixture.coordinator.clone();
        let waiting = tokio::spawn(async move {
            coordinator.wait_for_change().await;
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !waiting.is_finished(),
            "paused idle work caused a busy spin"
        );

        let promoted = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;
        tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .expect("promotion did not wake the wait")
            .expect("wait task panicked");

        let ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(waiter.await.unwrap(), Some(reference()));
        assert_eq!(promoted.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn wait_for_change_consumes_one_event_before_waiting_for_another() {
        let fixture = fixture();
        let _idle = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let observed = fixture.coordinator.change_generation();
        let first = fixture.coordinator.wait_for_change_since(observed);
        let promoted = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;
        tokio::time::timeout(Duration::from_millis(100), first)
            .await
            .expect("promotion did not wake the first wait");

        let observed = fixture.coordinator.change_generation();
        let second = fixture.coordinator.wait_for_change_since(observed);
        assert!(
            tokio::time::timeout(Duration::from_millis(25), second)
                .await
                .is_err(),
            "queued actionable work caused the second wait to spin"
        );

        let observed = fixture.coordinator.change_generation();
        let third = fixture.coordinator.wait_for_change_since(observed);
        fixture.coordinator.wake();
        tokio::time::timeout(Duration::from_millis(100), third)
            .await
            .expect("distinct coordinator event did not wake the wait");

        let ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.complete(ticket, reference()).await;
        assert_eq!(promoted.await.unwrap(), Some(reference()));
    }

    #[tokio::test]
    async fn explicit_wake_is_observed_when_coordinator_snapshot_is_unchanged() {
        let fixture = fixture();
        let before = (
            fixture.coordinator.selection().await,
            fixture.coordinator.background_generation().await,
            fixture.coordinator.pending_job_count().await,
            fixture.scheduler.available_background_permits(),
        );
        let first = fixture
            .coordinator
            .wait_for_change_since(fixture.coordinator.change_generation());
        fixture.coordinator.wake();
        tokio::time::timeout(Duration::from_millis(100), first)
            .await
            .expect("explicit wake did not advance coordinator generation");
        assert_eq!(
            (
                fixture.coordinator.selection().await,
                fixture.coordinator.background_generation().await,
                fixture.coordinator.pending_job_count().await,
                fixture.scheduler.available_background_permits(),
            ),
            before,
            "the explicit wake should not change the coordinator snapshot"
        );

        let second = fixture
            .coordinator
            .wait_for_change_since(fixture.coordinator.change_generation());
        assert!(
            tokio::time::timeout(Duration::from_millis(25), second)
                .await
                .is_err(),
            "unchanged coordinator snapshot caused a stale wake"
        );

        let third = fixture
            .coordinator
            .wait_for_change_since(fixture.coordinator.change_generation());
        fixture.coordinator.wake();
        tokio::time::timeout(Duration::from_millis(100), third)
            .await
            .expect("second explicit wake did not advance coordinator generation");
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
            availability: Availability::Available,
        };
        let waiter = fixture.enqueue(fresh, WorkLane::VisibleWall).await;
        assert!(fixture.coordinator.next_work().await.is_some());
        assert_eq!(fixture.coordinator.selection().await, Some(selection(2)));
        drop(waiter);
    }

    #[tokio::test]
    async fn duplicate_selection_reset_is_idempotent_for_current_foreground_work() {
        let fixture = fixture();
        let current = selection(2);
        fixture.coordinator.reset_selection(current).await;
        let key = screen_key_for(current, fixture_id(2), "screen-v2");
        let waiter = fixture.enqueue(key, WorkLane::ViewerPreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();

        fixture.coordinator.reset_selection(current).await;

        assert_eq!(fixture.coordinator.selection().await, Some(current));
        let expected = reference_for(fixture_id(2), DerivativeClass::ScreenPreview, "screen-v2");
        fixture.coordinator.complete(ticket, expected.clone()).await;
        assert_eq!(waiter.await.unwrap(), Some(expected));
    }

    #[tokio::test]
    async fn out_of_order_selection_reset_cannot_regress_newer_foreground_work() {
        let fixture = fixture();
        let older = selection(2);
        let newer = selection(3);

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let older_barrier = barrier.clone();
        let older_coordinator = fixture.coordinator.clone();
        let older_reset = tokio::spawn(async move {
            older_barrier.wait().await;
            older_coordinator.reset_selection(older).await;
        });
        let newer_barrier = barrier.clone();
        let newer_coordinator = fixture.coordinator.clone();
        let newer_reset = tokio::spawn(async move {
            newer_barrier.wait().await;
            newer_coordinator.reset_selection(newer).await;
        });
        older_reset.await.unwrap();
        newer_reset.await.unwrap();

        let key = screen_key_for(newer, fixture_id(3), "screen-v3");
        let waiter = fixture.enqueue(key, WorkLane::ViewerPreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.reset_selection(older).await;

        assert_eq!(fixture.coordinator.selection().await, Some(newer));
        let expected = reference_for(fixture_id(3), DerivativeClass::ScreenPreview, "screen-v3");
        fixture.coordinator.complete(ticket, expected.clone()).await;
        assert_eq!(waiter.await.unwrap(), Some(expected));
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
    async fn separate_selection_coordinators_have_isolated_scheduler_namespaces() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let left_selection = SelectionToken {
            library_id: LibraryId::new(),
            group_id: FolderGroupId::new(),
            epoch: 0,
        };
        let right_selection = SelectionToken {
            library_id: LibraryId::new(),
            group_id: FolderGroupId::new(),
            epoch: 0,
        };
        let left = DerivativeCoordinator::with_selection(scheduler.clone(), left_selection);
        let right = DerivativeCoordinator::with_selection(scheduler, right_selection);
        let left_key = WorkKey {
            selection: left_selection,
            asset_id: AssetId::from_uuid(uuid::Uuid::new_v4()),
            class: DerivativeClass::WallThumbnail,
            cache_key: "left".into(),
            availability: Availability::Available,
        };
        let right_key = WorkKey {
            selection: right_selection,
            asset_id: AssetId::from_uuid(uuid::Uuid::new_v4()),
            class: DerivativeClass::WallThumbnail,
            cache_key: "right".into(),
            availability: Availability::Available,
        };
        let _left_waiter = left.enqueue(left_key, WorkLane::VisibleWall).await;
        let _right_waiter = right.enqueue(right_key, WorkLane::VisibleWall).await;
        assert!(left.next_work().await.is_some());
        assert!(right.next_work().await.is_some());
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

        let thumbnail_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("thumbnail token");

        let thumbnail_cursor = WallCursorKey::Provisional {
            order: 2,
            id: fixture_id(2),
        };
        fixture
            .coordinator
            .advance_collection(&thumbnail_token, Some(thumbnail_cursor.clone()))
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, Some(thumbnail_cursor))
        );
        let thumbnail_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("advanced thumbnail token");
        fixture
            .coordinator
            .finish_thumbnail_phase(&thumbnail_token)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Previews, None)
        );

        let preview_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("preview token");

        let preview_cursor = WallCursorKey::Captured {
            captured_at_utc: "2026-08-28T00:00:00Z".to_owned(),
            display_path: "photo.jpg".to_owned(),
            id: fixture_id(3),
        };
        fixture
            .coordinator
            .advance_collection(&preview_token, Some(preview_cursor.clone()))
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Previews, Some(preview_cursor))
        );
        let preview_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("advanced preview token");
        fixture
            .coordinator
            .finish_preview_phase(&preview_token)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Complete, None)
        );
    }

    #[tokio::test]
    async fn stale_collection_progress_token_cannot_advance_or_finish_after_rewind() {
        let fixture = fixture();
        fixture
            .coordinator
            .begin_collection(fixture.selection)
            .await;
        let thumbnail_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("collection token");
        let thumbnail_cursor = WallCursorKey::Provisional {
            order: 2,
            id: fixture_id(2),
        };
        assert!(
            fixture
                .coordinator
                .advance_collection(&thumbnail_token, Some(thumbnail_cursor.clone()))
                .await
        );
        let advanced_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("advanced collection token");

        fixture
            .coordinator
            .rewind_collection_to_thumbnails(fixture.selection)
            .await;
        assert!(
            !fixture
                .coordinator
                .advance_collection(&advanced_token, Some(thumbnail_cursor))
                .await
        );
        assert!(
            !fixture
                .coordinator
                .finish_thumbnail_phase(&advanced_token)
                .await
        );
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, None)
        );

        let fresh_thumbnail_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("fresh thumbnail token");
        assert!(
            fixture
                .coordinator
                .finish_thumbnail_phase(&fresh_thumbnail_token)
                .await
        );
        let preview_token = fixture
            .coordinator
            .collection_progress_token(fixture.selection)
            .await
            .expect("preview token");
        fixture.coordinator.invalidate_background().await;
        fixture
            .coordinator
            .rewind_collection_to_thumbnails(fixture.selection)
            .await;
        assert!(
            !fixture
                .coordinator
                .advance_collection(
                    &preview_token,
                    Some(WallCursorKey::Captured {
                        captured_at_utc: "2026-08-28T00:00:00Z".to_owned(),
                        display_path: "photo.jpg".to_owned(),
                        id: fixture_id(3),
                    }),
                )
                .await
        );
        assert!(
            !fixture
                .coordinator
                .finish_preview_phase(&preview_token)
                .await
        );
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, None)
        );
    }

    #[tokio::test]
    async fn stale_collection_selection_cannot_change_new_selection() {
        let fixture = fixture();
        let old_selection = fixture.selection;
        let new_selection = selection(2);
        fixture.coordinator.begin_collection(old_selection).await;
        let stale_token = fixture
            .coordinator
            .collection_progress_token(old_selection)
            .await
            .expect("old selection token");
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
            .advance_collection(&stale_token, Some(stale_cursor))
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Dormant, None)
        );

        fixture.coordinator.begin_collection(new_selection).await;
        let new_token = fixture
            .coordinator
            .collection_progress_token(new_selection)
            .await
            .expect("new selection token");
        fixture
            .coordinator
            .finish_thumbnail_phase(&stale_token)
            .await;
        assert_eq!(
            fixture.coordinator.collection_state().await,
            (CollectionPhase::Thumbnails, None)
        );
        fixture.coordinator.finish_thumbnail_phase(&new_token).await;
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

    #[tokio::test]
    async fn invalidation_waits_until_success_waiters_are_delivered() {
        let fixture = fixture();
        let mut waiter = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();
        let delivery_entered = Arc::new(Notify::new());
        let delivery_release = Arc::new(Notify::new());
        fixture
            .coordinator
            .install_commit_waiter_delivery_test_gate(
                delivery_entered.clone(),
                delivery_release.clone(),
            )
            .await;

        let invalidator = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.invalidate_background().await })
        };
        let completer = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.complete_commit(permit, reference()).await })
        };
        delivery_entered.notified().await;

        assert!(!invalidator.is_finished());
        assert_eq!(waiter.try_recv().unwrap(), Some(reference()));
        delivery_release.notify_waiters();
        completer.await.unwrap();
        invalidator.await.unwrap();
    }

    #[tokio::test]
    async fn invalidation_waits_until_failure_waiters_are_delivered() {
        let fixture = fixture();
        let mut waiter = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();
        let mut foreground = fixture.enqueue(screen_key(), WorkLane::ViewerPreview).await;
        let delivery_entered = Arc::new(Notify::new());
        let delivery_release = Arc::new(Notify::new());
        fixture
            .coordinator
            .install_commit_waiter_delivery_test_gate(
                delivery_entered.clone(),
                delivery_release.clone(),
            )
            .await;

        let invalidator = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.invalidate_background().await })
        };
        let failure = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.fail_commit(permit).await })
        };
        delivery_entered.notified().await;

        assert!(!invalidator.is_finished());
        assert_eq!(waiter.try_recv().unwrap(), None);
        assert_eq!(foreground.try_recv().unwrap(), None);
        assert!(waiter.try_recv().is_err());
        assert!(foreground.try_recv().is_err());
        delivery_release.notify_waiters();
        failure.await.unwrap();
        invalidator.await.unwrap();
    }

    #[tokio::test]
    async fn late_success_waiters_join_the_commit_delivery_window() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let snapshot_entered = Arc::new(Notify::new());
            let snapshot_release = Arc::new(Notify::new());
            fixture
                .coordinator
                .install_commit_waiter_snapshot_test_gate(
                    snapshot_entered.clone(),
                    snapshot_release.clone(),
                )
                .await;
            let completer = {
                let coordinator = fixture.coordinator.clone();
                tokio::spawn(async move { coordinator.complete_commit(permit, reference()).await })
            };
            snapshot_entered.notified().await;

            let foreground = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::ViewerPreview)
                .await;
            let background = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::IdlePreview)
                .await;
            snapshot_release.notify_waiters();

            assert_eq!(initial.await.unwrap(), Some(reference()));
            assert_eq!(foreground.await.unwrap(), Some(reference()));
            assert_eq!(background.await.unwrap(), Some(reference()));
            completer.await.unwrap();
            assert_eq!(fixture.coordinator.pending_job_count().await, 0);
        }
    }

    #[tokio::test]
    async fn late_failure_waiters_join_the_commit_delivery_window() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let snapshot_entered = Arc::new(Notify::new());
            let snapshot_release = Arc::new(Notify::new());
            fixture
                .coordinator
                .install_commit_waiter_snapshot_test_gate(
                    snapshot_entered.clone(),
                    snapshot_release.clone(),
                )
                .await;
            let failure = {
                let coordinator = fixture.coordinator.clone();
                tokio::spawn(async move { coordinator.fail_commit(permit).await })
            };
            snapshot_entered.notified().await;

            let foreground = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::ViewerPreview)
                .await;
            let background = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::IdlePreview)
                .await;
            snapshot_release.notify_waiters();

            assert_eq!(initial.await.unwrap(), None);
            assert_eq!(foreground.await.unwrap(), None);
            assert_eq!(background.await.unwrap(), None);
            failure.await.unwrap();
            assert_eq!(fixture.coordinator.pending_job_count().await, 0);
        }
    }

    #[tokio::test]
    async fn late_success_enqueue_after_commit_removal_reuses_completed_outcome() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let delivery_entered = Arc::new(Notify::new());
            let delivery_release = Arc::new(Notify::new());
            fixture
                .coordinator
                .install_commit_waiter_delivery_test_gate(
                    delivery_entered.clone(),
                    delivery_release.clone(),
                )
                .await;
            let completer = {
                let coordinator = fixture.coordinator.clone();
                tokio::spawn(async move { coordinator.complete_commit(permit, reference()).await })
            };
            delivery_entered.notified().await;
            assert_eq!(initial.await.unwrap(), Some(reference()));
            delivery_release.notify_waiters();
            completer.await.unwrap();

            let late = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::ViewerPreview)
                .await;
            let second_late = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::IdlePreview)
                .await;
            assert_eq!(late.await.unwrap(), Some(reference()));
            assert_eq!(second_late.await.unwrap(), Some(reference()));
            assert_eq!(fixture.coordinator.pending_job_count().await, 0);
        }
    }

    #[tokio::test]
    async fn completion_snapshot_is_not_published_before_its_outcome() {
        for _ in 0..20 {
            let fixture = fixture();
            let receiver = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let entered = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            fixture
                .coordinator
                .install_commit_outcome_publication_test_gate(entered.clone(), release.clone())
                .await;
            let coordinator = fixture.coordinator.clone();
            let finisher = tokio::spawn(async move { coordinator.fail_commit(permit).await });
            entered.notified().await;

            let observed_generation = fixture.coordinator.commit_completion_generation();
            release.notify_one();
            finisher.await.unwrap();

            assert_eq!(receiver.await.unwrap(), None);
            let late = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(observed_generation),
                )
                .await;
            assert!(fixture.coordinator.next_work().await.is_none());
            assert_eq!(late.await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn older_failure_observer_keeps_original_outcome_while_retry_runs() {
        for _ in 0..20 {
            let fixture = fixture();
            let old_generation = fixture.coordinator.commit_completion_generation();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let initial_ticket = fixture.coordinator.next_work().await.unwrap();
            let initial_permit = fixture
                .coordinator
                .admit_commit(initial_ticket, fixture.selection, None)
                .await
                .unwrap();
            fixture.coordinator.fail_commit(initial_permit).await;
            assert_eq!(initial.await.unwrap(), None);

            let retry_generation = fixture.coordinator.commit_completion_generation();
            let retry = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::IdlePreview,
                    None,
                    Some(retry_generation),
                )
                .await;
            let retry_ticket = fixture.coordinator.next_work().await.unwrap();
            let retry_permit = fixture
                .coordinator
                .admit_commit(retry_ticket, fixture.selection, None)
                .await
                .unwrap();

            let mut old_observer = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(old_generation),
                )
                .await;
            assert_eq!(old_observer.try_recv().unwrap(), None);

            fixture
                .coordinator
                .complete_commit(retry_permit, reference())
                .await;
            assert_eq!(retry.await.unwrap(), Some(reference()));

            let fresh_observer = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(retry_generation),
                )
                .await;
            assert_eq!(fresh_observer.await.unwrap(), Some(reference()));
            assert!(fixture.coordinator.next_work().await.is_none());
        }
    }

    #[tokio::test]
    async fn older_success_observer_keeps_original_outcome_while_retry_runs() {
        for index in 0..20 {
            let fixture = fixture();
            let old_generation = fixture.coordinator.commit_completion_generation();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let initial_ticket = fixture.coordinator.next_work().await.unwrap();
            let initial_permit = fixture
                .coordinator
                .admit_commit(initial_ticket, fixture.selection, None)
                .await
                .unwrap();
            let first_reference = reference();
            fixture
                .coordinator
                .complete_commit(initial_permit, first_reference.clone())
                .await;
            assert_eq!(initial.await.unwrap(), Some(first_reference.clone()));

            let retry_generation = fixture.coordinator.commit_completion_generation();
            let retry = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::IdlePreview,
                    None,
                    Some(retry_generation),
                )
                .await;
            let retry_ticket = fixture.coordinator.next_work().await.unwrap();
            let retry_permit = fixture
                .coordinator
                .admit_commit(retry_ticket, fixture.selection, None)
                .await
                .unwrap();

            let mut old_observer = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(old_generation),
                )
                .await;
            assert_eq!(old_observer.try_recv().unwrap(), Some(first_reference));

            let second_reference = reference_for(
                fixture_id(1),
                DerivativeClass::ScreenPreview,
                &format!("screen-v1-retry-{index}"),
            );
            fixture
                .coordinator
                .complete_commit(retry_permit, second_reference.clone())
                .await;
            assert_eq!(retry.await.unwrap(), Some(second_reference.clone()));

            let fresh_observer = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(retry_generation),
                )
                .await;
            assert_eq!(fresh_observer.await.unwrap(), Some(second_reference));
            assert!(fixture.coordinator.next_work().await.is_none());
        }
    }

    #[tokio::test]
    async fn completed_outcome_history_is_bounded_and_current_keyed() {
        let fixture = fixture();
        let mut keys = Vec::new();
        for index in 0..=super::RECENT_CAPACITY {
            let cache_key = format!("history-{index}");
            let key = screen_key_for(
                fixture.selection,
                fixture_id((index + 1) as u128),
                &cache_key,
            );
            let receiver = fixture.enqueue(key.clone(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let result = reference_for(key.asset_id, key.class, &key.cache_key);
            fixture
                .coordinator
                .complete_commit(permit, result.clone())
                .await;
            assert_eq!(receiver.await.unwrap(), Some(result));
            keys.push(key);
        }

        assert_eq!(
            fixture.coordinator.completed_outcome_count().await,
            super::RECENT_CAPACITY
        );

        let evicted_receiver = fixture
            .coordinator
            .enqueue(keys[0].clone(), WorkLane::IdlePreview)
            .await;
        let evicted_ticket = fixture.coordinator.next_work().await.unwrap();
        fixture.coordinator.discard(evicted_ticket).await;
        assert_eq!(evicted_receiver.await.unwrap(), None);

        let latest_key = keys.last().unwrap().clone();
        let mut latest_receiver = fixture
            .coordinator
            .enqueue(latest_key.clone(), WorkLane::IdlePreview)
            .await;
        assert_eq!(
            latest_receiver.try_recv().unwrap(),
            Some(reference_for(
                latest_key.asset_id,
                latest_key.class,
                &latest_key.cache_key
            ))
        );
        assert!(fixture.coordinator.next_work().await.is_none());

        fixture.coordinator.reset_selection(selection(99)).await;
        let mut reset_receiver = fixture
            .coordinator
            .enqueue(latest_key, WorkLane::IdlePreview)
            .await;
        assert_eq!(reset_receiver.try_recv().unwrap(), None);
        assert!(fixture.coordinator.next_work().await.is_none());
    }

    #[tokio::test]
    async fn catalogue_pending_snapshot_reuses_completed_outcome_without_duplicate_work() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let observed = fixture.coordinator.commit_completion_generation();
            fixture
                .coordinator
                .complete_commit(permit, reference())
                .await;
            assert_eq!(initial.await.unwrap(), Some(reference()));

            let late = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(observed),
                )
                .await;
            assert_eq!(late.await.unwrap(), Some(reference()));
            assert!(fixture.coordinator.next_work().await.is_none());

            let retry = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(fixture.coordinator.commit_completion_generation()),
                )
                .await;
            let retry_ticket = fixture.coordinator.next_work().await.unwrap();
            assert_ne!(retry_ticket.attempt, ticket.attempt);
            fixture
                .coordinator
                .complete(retry_ticket, reference())
                .await;
            assert_eq!(retry.await.unwrap(), Some(reference()));
        }
    }

    #[tokio::test]
    async fn catalogue_pending_snapshot_reuses_failed_outcome_without_duplicate_work() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let observed = fixture.coordinator.commit_completion_generation();
            fixture.coordinator.fail_commit(permit).await;
            assert_eq!(initial.await.unwrap(), None);

            let late = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(observed),
                )
                .await;
            assert_eq!(late.await.unwrap(), None);
            assert!(fixture.coordinator.next_work().await.is_none());

            let retry = fixture
                .coordinator
                .enqueue_with_prerequisite_observed(
                    screen_key(),
                    WorkLane::ViewerPreview,
                    None,
                    Some(fixture.coordinator.commit_completion_generation()),
                )
                .await;
            let retry_ticket = fixture.coordinator.next_work().await.unwrap();
            assert_ne!(retry_ticket.attempt, ticket.attempt);
            fixture.coordinator.discard(retry_ticket).await;
            assert_eq!(retry.await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn late_failure_enqueue_after_commit_removal_reuses_completed_outcome() {
        for _ in 0..20 {
            let fixture = fixture();
            let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
            let ticket = fixture.coordinator.next_work().await.unwrap();
            let permit = fixture
                .coordinator
                .admit_commit(ticket, fixture.selection, None)
                .await
                .unwrap();
            let delivery_entered = Arc::new(Notify::new());
            let delivery_release = Arc::new(Notify::new());
            fixture
                .coordinator
                .install_commit_waiter_delivery_test_gate(
                    delivery_entered.clone(),
                    delivery_release.clone(),
                )
                .await;
            let failure = {
                let coordinator = fixture.coordinator.clone();
                tokio::spawn(async move { coordinator.fail_commit(permit).await })
            };
            delivery_entered.notified().await;
            assert_eq!(initial.await.unwrap(), None);
            delivery_release.notify_waiters();
            failure.await.unwrap();

            let late = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::ViewerPreview)
                .await;
            let second_late = fixture
                .coordinator
                .enqueue(screen_key(), WorkLane::IdlePreview)
                .await;
            assert_eq!(late.await.unwrap(), None);
            assert_eq!(second_late.await.unwrap(), None);
            assert_eq!(fixture.coordinator.pending_job_count().await, 0);
        }
    }

    #[tokio::test]
    async fn late_waiter_enqueue_after_snapshot_is_closed_to_new_work() {
        let fixture = fixture();
        let initial = fixture.enqueue(screen_key(), WorkLane::IdlePreview).await;
        let ticket = fixture.coordinator.next_work().await.unwrap();
        let permit = fixture
            .coordinator
            .admit_commit(ticket, fixture.selection, None)
            .await
            .unwrap();
        let snapshot_entered = Arc::new(Notify::new());
        let snapshot_release = Arc::new(Notify::new());
        fixture
            .coordinator
            .install_commit_waiter_snapshot_test_gate(
                snapshot_entered.clone(),
                snapshot_release.clone(),
            )
            .await;
        let completer = {
            let coordinator = fixture.coordinator.clone();
            tokio::spawn(async move { coordinator.complete_commit(permit, reference()).await })
        };
        snapshot_entered.notified().await;
        let late = fixture
            .coordinator
            .enqueue(screen_key(), WorkLane::ViewerPreview)
            .await;
        snapshot_release.notify_waiters();
        assert_eq!(initial.await.unwrap(), Some(reference()));
        assert_eq!(late.await.unwrap(), Some(reference()));
        completer.await.unwrap();
        assert_eq!(fixture.coordinator.pending_job_count().await, 0);
    }
}
