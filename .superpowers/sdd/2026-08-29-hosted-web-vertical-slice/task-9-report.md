# Task 9 implementation report

Status: review fixes complete; final implementation and lifecycle gates pass.

Regression implementation commit: `e316105` (`fix: preserve hosted offline availability`), based on `5e5b5ba`.

Deployment and acceptance commit: `517c679` (`feat: package hosted photo viewer for OCI`), based on `e316105`.

Initial recovery review-fix commit: `9a0cb05` (`fix: reconcile hosted source recovery`), based on report commit `b0616de`.

Persisted recovery review-fix commit: `d9fe53c` (`fix: persist hosted recovery reconciliation`), based on corrected report commit `8fa8af0`.

Terminal replay review-fix commit: `1e3fd6f` (`fix: render terminal hosted cache after replay`), based on `d9fe53c`.

## Delivered deployment contract

- `Containerfile` builds the hosted interface with Node 24, builds `photo-server` with Rust 1.97.1 and two Cargo jobs, and copies both products into a Debian Bookworm runtime image.
- The runtime is UID/GID 10001, exposes port 8080, and owns only `/var/lib/photo-viewer`, `/var/cache/photo-viewer`, and `/app/web`. It does not need write access to `/photos`.
- `.containerignore` excludes local build products, worktree metadata, `.env` files, SQLite state and sidecars, caches, and test artifacts from the image context.
- `deploy/compose.yaml` binds the photo source at `/photos:ro`, retains named data/cache volumes, publishes only `127.0.0.1:8080`, uses the four approved runtime variables, and restarts unless stopped.
- `deploy/nginx.conf.example` forwards the host and standard proxy headers, disables response buffering for SSE, and uses a one-hour read timeout. It introduces no public-hostname variable.
- `docs/deployment/hosted.md` covers Podman, Docker, Compose, read-only source permissions, UID 10001, health, persistence, backup, cache rebuild, reverse proxying, and restart/upgrade operations. All client URLs remain origin-relative, including at `https://photos.docker.jenner.lan`.

## Hosted lifecycle acceptance

`tests/hosted/hosted.spec.ts` and `scripts/hosted-smoke.sh` exercise one retained server installation across three phases:

- `beforeRestart` creates independent browser contexts for folders A and B, restores distinct appearance, sort, and scope preferences, opens each viewer, and records opaque selection IDs, derivative keys, URLs, and ETags.
- A starts at `currentFolder`; B uses `includeSubfolders`. A's nested child is catalogued through the existing recursive scan but is proved to have no wall or screen derivative before source loss.
- `afterRestart` recreates the contexts from saved local storage against the same data/cache volumes. Both selections, preferences, wall state, viewers, derivative IDs, and ETags survive without choosing the folders again.
- `offline` runs while the same restarted container remains alive and the source root is unreadable. Cached parent wall and screen derivatives remain usable for both browsers. Broadening A's scope exposes the known child as exactly `rootOffline` with null derivative references, a visible unavailable cue, and no open action.
- Folder requests reject encoded traversal, absolute, NUL, over-limit, repeated-parameter, file-as-folder, and symlink-escape paths.
- Portable metadata and SHA-256 manifests are checked after startup, browsing and derivative demand, restart, restored reads, offline reads, and all request probes.
- The EXIT trap restores source permissions first and removes only its exact `photo-viewer-smoke-*` container, network, named volumes, state, and record tree.

## Source-availability regressions found by acceptance

The first source-loss simulation showed that the application could no longer list `/photos`, but `/healthz` still counted the source as available. The health route was intentionally path-free and did no fresh source I/O, so the fix records the result of the already-requested empty-path root listing:

- an unreadable/unavailable root listing marks only the canonical matching library root offline;
- a later successful root listing marks that root online again;
- a nested unreadable folder does not mark the whole source offline;
- cached derivative responses remain 200 with the same opaque keys and ETags after the source becomes unavailable.

The offline child UI then exposed two event-order races:

- `wallError`/`sourceUnavailable` used to clear the active page request, causing a successful cached page to be rejected as unowned; only `pageRequestFailed` now clears that ownership;
- a replayed historical `catalogBatch` could overwrite an authoritative page's `rootOffline` availability with stale `available`. Catalog replay may still merge metadata and derivative references, but it cannot upgrade an existing unavailable item. A later authoritative `pageLoaded` result can restore `available` normally. A conservative unavailable replay still downgrades available state.

Task-scoped review then found a separate recovery defect: marking a root offline made every retained asset `rootOffline`, but a successful root listing restored only the library record. A settled stable selection skipped scan admission, so those assets stayed offline and a previously uncached derivative remained unavailable until process state changed. The first in-memory recovery fix was not durable for overlapping or empty groups. Migration v10 and the final review fix now:

- persist checked signed-64 `recovery_requested` and `recovery_reconciled` counters per folder group, plus the exact recovery token on each scan generation;
- selectively backfill only groups whose library is root-offline or whose membership contains root-offline assets, leaving clean large catalogs untouched;
- increment every group once for a root outage and only the affected group for a local outage, without wraparound or duplicate increments while recovery is already pending;
- serialize recovery admission in the existing per-selection scan control, reject stale tokens at or below the persisted reconciled value, and let concurrent callers for one token start exactly one scan;
- advance only that generation's captured token in the same successful online catalog-completion transaction, so a newer outage during a scan remains pending and a failed or cancelled scan cannot reconcile it;
- preserve pending recovery across overlapping groups, empty groups, unseen or deleted files, and process restart without inferring state from mutable asset availability;
- keep cancellation terminal but non-quiescent until the same generation's detached worker drain calls completion; retry admission is rejected before that handshake and admitted exactly once afterward;
- restore wall assets to `available` and permit a previously uncached derivative after the same stable selection is reselected;
- leave desktop runtimes, nested-folder isolation, cached offline reads, and path-free health behavior unchanged.

The smoke cleanup trap was also corrected to use EXIT for idempotent cleanup and explicit conventional HUP/INT/TERM exit codes. It still restores source permissions first and removes only exact smoke-owned names.

The browser regressions reproduce both relevant orders. The first covers a restored current-folder page, scope broadening, source-unavailable event, authoritative broad page containing the uncached offline child, and a stale available catalog replay. The closing regression adds synchronous scope reconnect/replay, a resync replacement request, a provisional authoritative page, and the same-selection React reset. Terminal scan knowledge now survives `sourceUnavailable`, resync, and same-selection scope reset while items, cursor, and page exhaustion still reset for the authoritative refetch. A different selection always clears that knowledge.

## RED evidence

The planned acceptance entry point was installed before the server/image existed:

```text
npm run test:hosted
  RED: Playwright could not reach the configured loopback hosted server.
```

The root availability regression was reproduced against the real router by making the source unreadable and refreshing the empty folder path. Before the fix, the failed listing did not change the library's health state. The final focused test proves healthy -> degraded/path-free -> healthy recovery and keeps nested failures local.

The stale replay regression has deterministic unit and browser REDs:

```text
npm test --workspace @photo-viewer/interface -- src/wall/wallReducer.test.ts \
  -t "does not let a replayed catalog batch upgrade offline availability"
  RED: expected rootOffline, received available; new RGB and derivative references had merged.

npm run test:browser --workspace @photo-viewer/interface -- \
  src/components/PhotoWall.browser.test.tsx \
  -t "shows an uncached offline child after broadening the restored scope"
  RED: expected "File unavailable" after the stale batch, received an empty fallback.
```

The settled-source recovery review fix began with three deterministic failures:

```text
photo-app-service restart fallback
  RED: remounted stable selection did not reconcile offline assets

photo-app-service concurrent ensure_running requests
  RED: stable selection did not run a recovery scan

photo-server real outage -> root listing -> same selection
  RED: stable selection did not reconcile after root recovery
```

A scan-control RED additionally proved that a cancelled recovery retained its pending token but could be readmitted before the old detached worker drained. Cancellation now has an explicit quiescence handshake: retry is rejected while the cancelled generation is still active and becomes admissible only after that exact generation completes its drain.

Closing review added deterministic REDs for the durable representation and final UI boundary:

```text
catalog migration/recovery state
  RED: per-group requested/reconciled state and scan-generation token APIs were absent

overlapping group restart
  RED: reconciling one shared group could erase the other group's asset-derived fallback

active cancellation barrier
  RED: retry admission overlapped already-admitted blocking metadata work

terminal offline browser replay
  RED: replacement page state contained two assets, but the uncached child figure was absent
```

The browser RED traced the final failure past the HTTP and reducer boundaries: the replacement page was accepted (`Preparing previews · 1 of 2`), the child remained `rootOffline`, and the stale replay could not upgrade it. The figure was omitted because provisional layout withholds the final row until scan completion, while source-unavailable completion was lost across resync and the same-selection scope reset.

## Focused GREEN evidence

```text
focused offline photo-server health and derivative API tests (two Cargo jobs)
  PASS: root health recovery and offline cached derivative cases

npm test --workspace @photo-viewer/interface -- src/wall/wallReducer.test.ts
  PASS: 43 tests

npm run test:browser --workspace @photo-viewer/interface -- \
  src/components/PhotoWall.browser.test.tsx \
  -t "keeps an uncached offline child visible when scope replay ends during a provisional page"
  PASS: 1 test, 53 skipped
```

The final reducer assertions cover metadata/reference merging under the replay fence, conservative unavailable replay, absent-item insertion, authoritative recovery, source-unavailable terminal completion without invented exhaustion, resync preservation, and same-selection-only reset preservation.

The recovery review fix passed the following focused evidence:

```text
photo-catalog recovery_state
  PASS: selective migration, empty/new offline group, idempotence, exact token, newer outage, and overflow rollback

photo-app-service scan-control recovery unit
  PASS: failure retains retry; cancellation requires quiescence; only completion reconciles the exact token

photo-app-service hosted_runtime
  PASS

photo-app-service hosted selections and runtime
  PASS: concurrent exact-one, overlapping-group restart, empty-group recovery, retry, and active-worker barrier

photo-server derivative/folder/health focused suites
  PASS
```

## Pre-commit gates

All commands ran serially. Cargo used offline mode and at most two jobs.

```text
cargo fmt --all -- --check
  PASS

cargo test --offline --workspace --all-targets --all-features --jobs 2
  PASS: all workspace, target, and feature tests

cargo clippy --offline --workspace --all-targets --all-features --jobs 2 -- -D warnings
  PASS

cargo check --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --jobs 2
  PASS

npm run check
  PASS: Biome checked 75 files

npm run typecheck
  PASS

npm test
  PASS: 18 files, 160 tests

npm run test:browser
  PASS: 4 files, 168 tests

podman build -t localhost/photo-viewer:dev -f Containerfile .
  PASS

git diff --check
  PASS
```

The persisted recovery review fix was followed by the complete Rust gate from the updated source. After the final frontend-only reducer change, Biome, typecheck, all 160 unit tests, and all 168 browser tests passed again. The first sandboxed browser attempt could not bind its loopback test server; the identical approved local-listener run passed. The browser suite emitted only the branch's existing non-fatal `ViewerStage` `act(...)` warning.

## Final lifecycle run

The accepted run was:

```text
./scripts/hosted-smoke.sh
  PASS
```

The explicit image build rebuilt the updated Rust product once with two Cargo jobs. The final smoke build reused every Rust layer and rebuilt only the hosted web stage after the reducer fix. The resulting accepted image was `a52039cd7f66937eae5ab2fe341e6c6f10f3136ee1a693bd60716af840cbbd95`.

Observed phase results:

```text
beforeRestart: PASS (1 test)
afterRestart:  PASS (1 test)
offline:       PASS (1 test)

encoded traversal:       HTTP 400
absolute folder:         HTTP 400
NUL folder:              HTTP 400
over-limit folder:       HTTP 400
repeated folder:         HTTP 400
file as folder:          HTTP 404
symlink escape:          HTTP 400
```

The run reported unchanged source metadata and hashes after every required checkpoint. It also proved independent browsers, retained-volume restart, stable opaque IDs and ETags, degraded path-free health, cached offline viewing, and the exact uncached-child unavailable UI.

## Cleanup and limits

- After the run, read-only Podman listings contained no `photo-viewer-smoke-*` container, network, or volume, and no matching temporary root remained. The foreground smoke session exited 0.
- The only removed test artifacts were the exact RED screenshot and Vitest attachment, followed by their empty generated directories.
- No source fixture was modified. No unrelated container, volume, or image was removed by the Task 9 implementation.
- The nested desktop lockfile's generated dependency-list churn was removed before commit.
- Host disk had 13 GiB free at the closing review-fix commit checkpoint.
- The image and lifecycle were exercised through Podman on macOS. This report does not claim a Windows container runtime or Windows source-loss execution.
- No merge or push occurred.
