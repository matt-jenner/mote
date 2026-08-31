# Task 9 implementation report

Status: implementation complete, awaiting task-scoped Sol review.

Regression implementation commit: `e316105` (`fix: preserve hosted offline availability`), based on `5e5b5ba`.

Deployment and acceptance commit: `517c679` (`feat: package hosted photo viewer for OCI`), based on `e316105`.

## Delivered deployment contract

- `Containerfile` builds the hosted interface with Node 24, builds `photo-server` with Rust 1.97.1 and two Cargo jobs, and copies both products into a Debian Bookworm runtime image.
- The runtime is UID/GID 10001, exposes port 8080, and owns only `/var/lib/photo-viewer`, `/var/cache/photo-viewer`, and `/app/web`. It does not need write access to `/photos`.
- `.containerignore` excludes local build products, worktree metadata, state databases, caches, and test artifacts from the image context.
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

The browser regression reproduces the exact order: restored current-folder page, scope broadening, source-unavailable event, authoritative broad page containing the uncached offline child, and a stale available catalog replay. It also proves later authoritative recovery.

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

## Focused GREEN evidence

```text
focused offline photo-server health and derivative API tests (two Cargo jobs)
  PASS: root health recovery and offline cached derivative cases

npm test --workspace @photo-viewer/interface -- src/wall/wallReducer.test.ts
  PASS: 40 tests

npm run test:browser --workspace @photo-viewer/interface -- \
  src/components/PhotoWall.browser.test.tsx \
  -t "shows an uncached offline child after broadening the restored scope"
  PASS: 1 test, 52 skipped
```

The final reducer assertions cover metadata/reference merging under the replay fence, conservative unavailable replay, absent-item insertion through existing behavior, and authoritative recovery.

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
  PASS: 18 files, 157 tests

npm run test:browser
  PASS: 4 files, 167 tests

podman build -t localhost/photo-viewer:dev -f Containerfile .
  PASS

git diff --check
  PASS
```

The final frontend-only replay fix was followed by fresh Biome, typecheck, unit, and browser gates. It did not alter Rust or container Rust inputs. A fresh formatting check also passed immediately before commit. The browser suite emitted only the branch's existing non-fatal `ViewerStage` `act(...)` warning.

## Final lifecycle run

The accepted run was:

```text
./scripts/hosted-smoke.sh
  PASS
```

The Node dependency layers were cached, the hosted web layer rebuilt, and the Rust source copy plus release build were cache hits; no Rust compilation occurred. The resulting accepted image was `5ddcc94a2a0c4d57803d19e4bd60b38ac3b6264f01ac6a4391b00a50ecfed9aa`.

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

- After the run, read-only Podman listings contained no `photo-viewer-smoke-*` container or volume. The smoke session exited 0; no background process was launched.
- The only removed test artifacts were the exact RED screenshot and its hash-identical Vitest attachment, followed by their empty generated directories.
- No source fixture was modified. No unrelated container, volume, or image was removed by the Task 9 implementation.
- The nested desktop lockfile's generated dependency-list churn was removed before commit.
- Host disk had 20 GiB free at the commit checkpoint.
- The image and lifecycle were exercised through Podman on macOS. This report does not claim a Windows container runtime or Windows source-loss execution.
- No merge or push occurred.
