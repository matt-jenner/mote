# Final native safety report

## Scope

Branch: `codex/release-final-native-safety`

Implementation commit: `a21b691` (`fix: close pick jobs and harden copy publication`)

Shutdown admission-race correction: `dc5718a` (`fix: fence pick admission against shutdown`)

This correction closes the independent Picks gallery registry during `AppService::shutdown`, rejects derivative requests against a closed registry, replaces hard-link publication with atomic no-replace rename on supported native platforms, and verifies the retained destination identity after publication.

## Root causes

- `AppService` owns separate wall and Picks `FolderJobRegistry` instances, but shutdown retired only the wall registry. A post-shutdown Picks request could therefore create and run new derivative work.
- Original-copy publication used `hard_link` followed by temporary-file removal. That made successful publication depend on hard-link support even though the operation only needs an atomic, collision-safe move.
- The copy path checked the destination immediately before publication but returned success immediately afterward. A delete, rename, or path replacement at that boundary could be reported and remembered as a successful copy.

## TDD evidence

The following tests were added first and observed failing against `50e2629`:

- `service_shutdown_rejects_new_pick_derivative_work`: expected `DerivativeUnavailable`, but the request returned success.
- `publication_consumes_the_private_temporary_name`: expected one final entry at the publication boundary, but the hard-link implementation exposed both the final and temporary names.
- `destination_replacement_after_publication_is_not_reported_or_remembered`: expected `CopyDestinationMissing`, zero progress reports, and no remembered destination, but the copy returned success.

All three passed after the implementation.

Review round 1 added a stronger deterministic race. `shutdown_between_pick_validation_and_runtime_admission_rejects_the_racing_request` pauses a real Picks request after validation, closes the registry, and then resumes it. The test initially returned success against `bcf1999`; after `dc5718a`, it resolves with `DerivativeUnavailable`, starts zero hosted attempts, retains zero Picks runtimes, and cannot strand its receiver.

## Implementation notes

- Shutdown now retires and drains both registries. `GalleryEngine` rejects a derivative request once its registry is closed.
- Runtime creation now finishes with the registry's `admits_binding` check. The registry lock orders this admission against shutdown, so a runtime created after shutdown is cancelled and rejected before enqueue.
- Apple and Linux/Redox builds use `rustix::fs::renameat_with(..., RenameFlags::NOREPLACE)`. On macOS this maps to `renameatx_np(RENAME_EXCL)`; Windows uses no-replace `MoveFileW`.
- Platforms without one of those native operations retain the collision-safe hard-link fallback. Mote's macOS build no longer uses that fallback.
- After publication, the service revalidates the destination and compares the path's filesystem identity with the directory handle retained before copying. A replacement path cannot be counted or persisted as success.

## Verification

Host: Darwin arm64, Rust 1.97.1.

- `cargo test -p photo-app-service --lib original_copy::tests`: 14 passed.
- `cargo test -p photo-app-service --test original_copy`: 17 passed.
- `cargo test -p photo-app-service --test photo_picks`: 7 passed.
- `cargo test -p photo-app-service --lib shutdown_`: 3 passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy -p photo-app-service --all-targets -- -D warnings`: passed.
- `git diff --check`: passed.

## Remaining concern

The no-replace call and post-publication destination check are separate filesystem operations, so no portable API can freeze the destination against a later change after the check completes. The new check closes the reviewed publication-boundary gap and uses retained filesystem identity rather than path spelling, but a change after the final check belongs to normal external mutation after a completed copy.
