use photo_indexer::{IndexJob, IndexScheduler, InteractionMode, JobPriority, SchedulerConfig};

fn job(name: &str, priority: JobPriority) -> IndexJob {
    IndexJob::new(name, priority)
}

async fn dequeue_names<const N: usize>(
    scheduler: &IndexScheduler,
    jobs: [IndexJob; N],
) -> Vec<String> {
    for queued in jobs {
        scheduler.enqueue(queued).await;
    }

    let mut names = Vec::new();
    while let Some(queued) = scheduler.next().await {
        names.push(queued.name().to_owned());
    }
    names
}

#[tokio::test(start_paused = true)]
async fn derivative_lanes_dequeue_in_product_priority_order() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());

    assert_eq!(
        dequeue_names(
            &scheduler,
            [
                job("idle-preview", JobPriority::IdleLibrary),
                job("idle-wall", JobPriority::OpenCollection),
                job("near-wall", JobPriority::NearViewport),
                job("viewer", JobPriority::ViewerPreview),
                job("visible-wall", JobPriority::Visible),
            ]
        )
        .await,
        [
            "visible-wall",
            "viewer",
            "near-wall",
            "idle-wall",
            "idle-preview"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn visible_work_preempts_queued_idle_work() {
    let scheduler = IndexScheduler::new(SchedulerConfig {
        idle_workers: 4,
        active_workers: 1,
    });
    scheduler
        .enqueue(job("idle-a", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("idle-b", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("visible", JobPriority::Visible))
        .await;

    assert_eq!(scheduler.next().await.unwrap().name(), "visible");
}

#[tokio::test(start_paused = true)]
async fn owned_dequeue_does_not_remove_foreign_higher_priority_work() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("foreign-visible", JobPriority::Visible))
        .await;
    scheduler
        .enqueue(job(
            "photo-derivative-coordinator:idle",
            JobPriority::IdleLibrary,
        ))
        .await;

    assert!(
        scheduler
            .next_owned("photo-derivative-coordinator:")
            .await
            .is_none()
    );
    assert_eq!(scheduler.next().await.unwrap().name(), "foreign-visible");
    assert_eq!(
        scheduler
            .next_owned("photo-derivative-coordinator:")
            .await
            .unwrap()
            .name(),
        "photo-derivative-coordinator:idle"
    );
}

#[tokio::test(start_paused = true)]
async fn owned_dequeue_discards_stale_foreign_entries_before_owner_barrier() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("foreign", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("foreign", JobPriority::Visible))
        .await;
    assert_eq!(scheduler.next().await.unwrap().name(), "foreign");

    scheduler
        .enqueue(job(
            "photo-derivative-coordinator:idle",
            JobPriority::IdleLibrary,
        ))
        .await;

    assert_eq!(
        scheduler
            .next_owned("photo-derivative-coordinator:")
            .await
            .unwrap()
            .name(),
        "photo-derivative-coordinator:idle"
    );
}

#[tokio::test(start_paused = true)]
async fn equal_priority_jobs_remain_fifo() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("first", JobPriority::NearViewport))
        .await;
    scheduler
        .enqueue(job("second", JobPriority::NearViewport))
        .await;

    assert_eq!(scheduler.next().await.unwrap().name(), "first");
    assert_eq!(scheduler.next().await.unwrap().name(), "second");
}

#[tokio::test(start_paused = true)]
async fn interaction_reduces_new_background_concurrency_to_one() {
    let scheduler = IndexScheduler::new(SchedulerConfig {
        idle_workers: 4,
        active_workers: 1,
    });

    scheduler
        .set_interaction_mode(InteractionMode::Active)
        .await;

    assert_eq!(scheduler.available_background_permits(), 1);
    scheduler.set_interaction_mode(InteractionMode::Idle).await;
    assert_eq!(scheduler.available_background_permits(), 4);
}

#[tokio::test(start_paused = true)]
async fn idle_work_runs_after_a_finite_visible_queue() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("idle", JobPriority::IdleLibrary))
        .await;
    for index in 0..20 {
        scheduler
            .enqueue(job(&format!("visible-{index}"), JobPriority::Visible))
            .await;
    }

    for _ in 0..20 {
        assert_eq!(
            scheduler.next().await.unwrap().priority(),
            JobPriority::Visible
        );
    }
    assert_eq!(scheduler.next().await.unwrap().name(), "idle");
}

#[tokio::test(start_paused = true)]
async fn enqueuing_the_same_asset_at_higher_priority_promotes_without_duplication() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("asset-a", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("asset-a", JobPriority::Visible))
        .await;

    let promoted = scheduler.next().await.unwrap();

    assert_eq!(promoted.name(), "asset-a");
    assert_eq!(promoted.priority(), JobPriority::Visible);
    assert!(scheduler.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn stale_promotion_entry_does_not_hide_other_queued_work() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    scheduler
        .enqueue(job("asset-a", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("asset-b", JobPriority::IdleLibrary))
        .await;
    scheduler
        .enqueue(job("asset-a", JobPriority::Visible))
        .await;

    assert_eq!(scheduler.next().await.unwrap().name(), "asset-a");
    assert_eq!(scheduler.next().await.unwrap().name(), "asset-b");
}

#[tokio::test(start_paused = true)]
async fn cancelling_low_value_queued_work_sets_its_cooperative_token() {
    let scheduler = IndexScheduler::new(SchedulerConfig::default());
    let idle = job("idle", JobPriority::IdleLibrary);
    let token = idle.cancellation_token();
    scheduler.enqueue(idle).await;

    scheduler.cancel_below(JobPriority::OpenCollection).await;

    assert!(token.is_cancelled());
    assert!(scheduler.next().await.is_none());
}
