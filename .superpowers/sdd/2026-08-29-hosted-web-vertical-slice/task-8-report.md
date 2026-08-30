# Task 8 implementation report

Status: fix round 1 complete, awaiting task-scoped re-review.

Initial implementation commit: `e5c96b9` (`feat: serve the hosted photo viewer securely`), based on `bce40c4`.

Fix round 1 implementation commit: `90d48d1` (`fix: contain hosted static file access`), based on `edfd339`.

## Delivered behavior

- `photo-server` now composes the existing API, SSE, derivative, and `/healthz` routes ahead of one static fallback rooted at the validated `PHOTO_VIEWER_WEB_ROOT`.
- The static host uses `tower-http` `ServeDir` over a descriptor-relative backend, including ETags, conditional requests, ranges, HEAD parity, and precompressed Brotli/gzip variants when present. Hashed `/assets/*` responses use `public, max-age=31536000, immutable`; interface HTML uses `no-cache`.
- `/` and extensionless non-API GET/HEAD routes serve `index.html` without redirects. File-like misses remain 404, and unknown `/api/v1/*` and `/healthz/*` routes return fixed, path-free JSON 404 responses rather than the interface shell.
- Interface responses carry the planned CSP, `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`. The CSP remains:

  ```text
  default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'
  ```

- `ServerConfig` now requires an existing canonical web directory and rejects source/web or private-state/web overlap before startup. The existing source-root containment, read-only scan/derivative behavior, request-target limits, 64 KiB JSON limit, opaque errors, SSE streaming, and bounded derivative streaming remain unchanged.
- Production startup passes the validated web root to `build_router(state, web_root)` and supplies peer `ConnectInfo`. Host and forwarded headers are recorded only in trace fields named as untrusted; they never participate in URL, path, authorization, or containment decisions.
- Added root `web:build` and `web:dev` scripts for Vite hosted mode. `apps/interface/package.json` needed no edit because its existing build and dev scripts already accept the forwarded `--mode hosted` argument.
- Enabled exactly the planned `tower-http` features: `catch-panic`, `fs`, `set-header`, and `trace`.

The plan's full-workspace clippy command initially exposed warnings in Tasks 5–7 code already present at the Task 8 base. The checkpoint includes behavior-preserving cleanup only: shared aliases for identical debug/test gate types, cfg-tail expressions, equivalent iterator and range forms, an internal `NewJob` argument record, equivalent condition/result expressions, and test-only borrow/condition cleanup. No public contract, persistence shape, scheduling order, or state-machine branch changed.

## RED evidence

The new static host tests were run before production implementation:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host --jobs 2
  RED: 0 passed, 3 failed.
  Root and nested routes returned 404, the asset returned 404, and the unknown API route had no JSON content type.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api server_config_requires_an_existing_web_directory --jobs 2
  RED: the invalid web-root case was accepted and the configured path was not canonicalized.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api server_config_rejects_web_root_overlap_with_source_or_private_state --jobs 2
  RED: source/web overlap was accepted.
```

The prescribed workspace clippy command was also a reproducible RED on the Task 8 base. Its findings were handled iteratively rather than waived: the first wave contained six cache and one indexer warning; the next contained fifteen app-service warnings; later all-target passes exposed seven test/source-layout warnings. Each was a mechanical lint shape, and each affected crate's focused tests passed before the workspace gate was retried.

## Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host --test health_api --jobs 2
  PASS: 14 tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --jobs 2
  PASS: 49 tests across library, derivative, event, folder, gallery, health, and static-host suites

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --jobs 2
  PASS: 45 tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-indexer --jobs 2
  PASS: 39 tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --jobs 2
  PASS: 210 tests, including the source-safety harness

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_runtime --jobs 2
  PASS: 16 tests after the final test-only lint cleanup

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test events_api --jobs 2
  PASS: 10 tests after the final server test-only lint cleanup

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-cache --test image_derivative --jobs 2
  PASS: 15 tests after the final cache test-only lint cleanup
```

## Final gates

```text
cargo fmt --all -- --check
  PASS

npm run check
  PASS: Biome checked 75 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 154 tests

npm run test:browser
  PASS: 4 files, 165 tests

npm run web:build
  PASS: Vite 8.2.2 transformed 1,891 modules
  dist/index.html uses only /assets/... script and stylesheet URLs

CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace and doc tests

CARGO_BUILD_JOBS=2 cargo clippy --offline --workspace --all-targets --all-features --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo check --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --jobs 2
  PASS: desktop and dependent crates checked successfully

CARGO_BUILD_JOBS=2 cargo build --offline --release -p photo-server --jobs 2
  PASS

git diff --check
  PASS: no whitespace errors
```

The first sandboxed browser run could not bind its ephemeral IPv6 loopback port (`EPERM`). The approved rerun with scoped loopback permission passed. It emitted the branch's existing non-fatal ViewerStage `act(...)` warning, with no failed test. The desktop check regenerated two dependency-list lines in its nested lockfile from earlier manifests; that generated churn was restored, leaving the nested lock unchanged.

One scoped online `cargo fetch` was approved because the exact planned `tower-http` `fs` feature required `http-range-header` and `mime_guess`, which were absent locally. The root lockfile gained the five expected packages or transitive entries: `futures-sink`, `http-range-header`, `mime_guess`, `tokio-util`, and `unicase`. Every build, lint, check, and test after that fetch ran offline.

## Production-process demonstration

The release binary was started once in the foreground with fresh temporary source, data, and cache directories plus the built `apps/interface/dist`:

- `/` returned the built shell with 200, `no-cache`, an ETag, and all planned security headers.
- `/gallery/demo-selection` returned the same shell with 200 and no redirect while supplied `Host`, `X-Forwarded-Host`, and `X-Forwarded-Proto` values were not reflected into the document or an absolute URL.
- `/healthz` returned 200 with path-free healthy database, cache, source-count, and warning state.
- `/api/v1/unknown` returned 404 JSON: `{"code":"notFound","message":"That route is unavailable."}`.
- The built hashed JavaScript asset returned an ETag and the immutable one-year cache policy.

The process was stopped, port 18080 was clear, and its temporary state was removed.

## Cleanup and constraints

- Generated `apps/interface/dist` remains ignored and was not staged. The nested desktop lockfile has no diff. No browser attachment or screenshot artifact was produced.
- No source-media file was modified. The full source-safety harness passed on final source.
- No background build, `cargo clean`, merge, or push occurred. Cargo and npm commands ran serially; Cargo used at most two jobs.
- No OCI image, container files, deployment configuration, hosted smoke script, or deployment documentation was added. Those remain Task 9 ownership.
- This task does not claim Windows runtime execution or a cross-process cache transaction guarantee; it preserves the reviewed Task 5 portability contract.

## Fix round 1: static-file containment and normalized fallback

The first Sol review found that the original pathname-based `ServeDir` and `ServeFile` calls could follow a descendant symlink after startup validation. It also found inconsistent encoded-path classification and a nested fallback that discarded request headers. Commit `90d48d1` replaces those seams without changing the API, SSE, derivative, cache, or source-scanning contracts.

### Behavioral RED evidence

The expanded static-host suite ran against `edfd339` before the containment implementation:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host --jobs 2
  RED: 3 passed, 4 failed.
  nested fallback with Accept-Encoding: br returned the uncompressed index without Content-Encoding.
  /missing%2Etxt returned the interface shell with 200 instead of rejecting the encoded dot.
  nested If-None-Match returned 200 instead of 304.
  a regular asset symlink returned the exact bytes "SOURCE ORIGINAL ASSET" from outside the web root.
```

The first deterministic swap tests were also compiled before `SecureRoot` existed:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib secure_open_tests --jobs 2
  RED: unresolved import super::SecureRoot
```

The original combined symlink test stopped at its first asset assertion. The RED therefore proves direct asset disclosure, but it does not claim that the old code separately returned the index or sidecar sentinels. Those cases remain regression coverage for the same pathname-open defect.

### Containment implementation

- `SecureRoot` pins the configured canonical web directory with a descriptor opened from `/`. Every root and request component is opened relative to its pinned parent with `openat`, `O_NOFOLLOW`, and `O_CLOEXEC`; directories also require `O_DIRECTORY`.
- Final opens use `O_NONBLOCK` and then require an actual regular file. Metadata lookups use the same no-follow traversal. The file object handed to `tower-http` is the already-open descriptor, so `ServeDir` never reopens a checked pathname.
- A renamed ancestor remains bound to its pinned directory. A final file or `.br`/`.gz` sidecar replaced by a source symlink fails closed. Symlinked sidecars can only fall back to the safe identity asset.
- The same backend remains behind `tower-http` `fs`, preserving MIME selection, compression negotiation, ETags, preconditions, ranges, HEAD, and streaming.
- Non-Unix builds return `Unsupported` while creating the secure static root and install an unavailable static backend. Static requests then return not found while API and health routes remain available. This is an explicit fail-closed portability boundary, not a claim of Windows static hosting.

### Path and fallback behavior

- One parser percent-decodes and validates the request path before both filesystem access and route classification. It rejects malformed escapes, encoded dots or separators, backslashes, NUL, invalid UTF-8, empty interior components, and traversal components.
- Encoded API/health separator attempts such as `/api%2Fv1%2Funknown` and `/healthz%2Funknown` return 400 and never receive the SPA shell. `/missing%2Etxt` also returns 400. Literal API and health misses retain their fixed path-free JSON 404 envelopes.
- The normalized components produce the only URI passed to `ServeDir`. Nested interface fallback retains the original GET/HEAD method and request headers, including `Accept-Encoding`, `If-None-Match`, and `Range`.
- The broad-bind warning now says that the photo viewer server, not only health, is exposed beyond loopback.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib secure_open_tests --jobs 2
  PASS: 3 tests
  The ancestor swap returned SAFE ASSET from the pinned directory.
  Final asset and Brotli/gzip sidecar swaps to source symlinks failed closed.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host --jobs 2
  PASS: 9 tests
  Regular asset, index, Brotli sidecar, and gzip sidecar sentinels were not served.
  Nested Brotli, ETag 304, byte range, and HEAD behavior passed.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --jobs 2
  PASS: 58 tests

CARGO_BUILD_JOBS=2 cargo clippy --offline -p photo-server --all-targets --all-features --no-deps --jobs 2 -- -D warnings
  PASS
```

The direct `libc` dependency supplies the Unix descriptor flags and `openat` call. No new registry package was needed because the locked version was already present.

### Final fix-round gates

```text
cargo fmt --all -- --check
  PASS

npm run check
  PASS: Biome checked 75 files, no fixes required

npm run typecheck
  PASS

npm test
  PASS: 18 files, 154 tests

npm run test:browser
  PASS: 4 files, 165 tests

npm run web:build
  PASS: Vite 8.2.2 transformed 1,891 modules
  dist/index.html uses only /assets/... script and stylesheet URLs

CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace and doc tests, including the source-safety harness

CARGO_BUILD_JOBS=2 cargo clippy --offline --workspace --all-targets --all-features --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo check --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --jobs 2
  PASS

CARGO_BUILD_JOBS=2 cargo build --offline --release -p photo-server --jobs 2
  PASS

git diff --check
  PASS
```

The sandboxed browser run again failed before test collection because it could not bind an ephemeral IPv6 loopback port. The approved scoped rerun passed and emitted only the branch's known non-fatal `ViewerStage` `act(...)` warning. The desktop check regenerated the same two dependency-list lines recorded at the initial checkpoint; those lines were removed, leaving its nested lockfile unchanged.

### Live release-process evidence

One foreground release process used the built hosted interface, the existing six-photo demo directory as a read-only source, and fresh temporary data/cache directories:

- `/` and `/gallery/live-demo` returned the built shell with 200, `no-cache`, ETag, no redirect, and the exact CSP/nosniff/referrer headers. Supplied Host and forwarded headers did not create or reflect an absolute origin.
- `/healthz` returned healthy JSON with one available source and no native path. `/api/v1/unknown` returned the fixed JSON 404.
- A created selection exposed an SSE stream with `text/event-stream`, `no-cache`, and `x-accel-buffering: no`. A future replay probe returned a framed `wallUpdate` event with authoritative `id: 0` and path-free `resyncRequired` data.
- A visible wall-thumbnail request returned 204. Its opaque derivative URL then returned 200 `image/jpeg`, a 190,681-byte managed body, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

The server was interrupted in the foreground, port 18080 had no listener, and the temporary data/cache tree was removed. Process enumeration is restricted on this host. Every command ran in the foreground and exited, and no background process was launched. Generated `dist` remains ignored. No source photo changed, and no merge, push, OCI work, or Task 9 deployment work occurred.

### Limitation

Only the installed `aarch64-apple-darwin` target was compiled and executed. The `cfg(not(unix))` fail-closed branch and its unit test document the Windows behavior, but this round does not claim a Windows compile or runtime result. Windows static interface hosting remains disabled until it has an equivalent handle-relative, no-reparse implementation.
