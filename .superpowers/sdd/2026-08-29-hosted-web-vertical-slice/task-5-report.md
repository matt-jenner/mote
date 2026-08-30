# Task 5 verification report

Implementation under verification: `dbfdb39` (`feat: serve managed hosted photo derivatives`), compared with `8a3de31`.

## RED / GREEN evidence

The task-5 implementation commit adds the previously absent hosted derivative API and managed-open interfaces. The pre-implementation RED expectation in `task-5-brief.md` was missing-method/route failure; this verification run was against the existing implementation and therefore focused on GREEN evidence. No implementation files were changed during verification.

Focused GREEN results:

- `photo-cache` cache policy: 15 passed; focused `open_checked`: 1 passed.
- `photo-catalog` catalog round trip: 12 passed; selection membership: 4 passed.
- `photo-app-service` hosted selections: 7 passed; source safety: 1 passed.
- `photo-server` derivative API: 1 passed.
- Adjacent progressive/offline/path-safety checks: offline reopen 1 passed; cache reuse after source removal 1 passed; managed-read checks 2 passed; path-free wall DTO 1 passed; open/reopen suite 7 passed.
- Full workspace: all unit/integration tests and doc-tests passed, including progressive wall (57), task-7 source safety (1), derivative API (1), events (10), gallery (9), folders (10), health (10), and all cache/catalog/indexer/metadata suites. No failures or ignored tests were reported.

One standalone `progressive_wall` invocation remained in `invalidation_before_commit_admission_has_no_side_effects` for about eight minutes after reporting the other 56 tests as passing and was interrupted (exit 130). The subsequent required full workspace run completed the same test and all 57 progressive-wall tests in 4.42 seconds; this appears nondeterministic timing/flakiness worth monitoring, not a reproduced functional failure.

## Exact verification commands

All Cargo commands were run serially with offline dependencies, `CARGO_BUILD_JOBS=2`, and `--jobs 2` (except no build-job flag is accepted by `cargo fmt`; the environment was still set).

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy --jobs 2
  PASS: 15 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test cache_policy open_checked --jobs 2
  PASS: 1 passed, 0 failed (14 filtered)
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test catalog_round_trip --jobs 2
  PASS: 12 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-catalog --test selection_membership --jobs 2
  PASS: 4 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections --jobs 2
  PASS: 7 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test task7_source_safety --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test derivative_api --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall --jobs 2
  INTERRUPTED: stalled in invalidation_before_commit_admission_has_no_side_effects; exit 130
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall offline_reopen --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall second_identical_wall_request_reuses_cache_after_the_source_goes_offline --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall read_derivative --jobs 2
  PASS: 2 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test progressive_wall wall_dtos --jobs 2
  PASS: 1 passed, 0 failed
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test open_and_return --jobs 2
  PASS: 7 passed, 0 failed
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace tests and doc-tests; 0 failures
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
  PASS: no formatting differences
git diff --check 8a3de31..dbfdb39
  PASS: no whitespace errors
git status --short
  PASS: clean before this report was created
```

## Diff and safety audit

`dbfdb39` changes 11 files: 698 insertions and 7 deletions. `CacheWriter::open_checked` validates relative paths, rejects traversal and symlink escapes, checks regular-file status, and opens read-only. The derivative route resolves only the opaque cache key through the catalog and never accepts a source path, range header, or original-media fallback. Source snapshot/symlink/missing-file tests pass.

Source-media mutation audit found no source-root `write`, `remove`, `rename`, or `copy` operation in the hosted derivative path. Cache writes and partial-file cleanup are confined to the canonical cache root. DTO and HTTP error paths expose opaque IDs/relative selection breadcrumbs or generic messages; no native source path is serialized. Relevant path-free tests passed, including `wall_dtos_never_serialize_native_paths`, `bootstrap_does_not_expose_a_native_source_path`, and the task-7 source snapshot harness.

## Concern

`crates/server/src/api/derivative.rs` calls `Read::read_to_end` and constructs `Body::from(bytes)`, so delivery is safely cache-scoped but currently buffers the complete derivative rather than streaming the managed file. If “managed streaming” is a hard requirement for large derivatives, this should be addressed in a follow-up.
