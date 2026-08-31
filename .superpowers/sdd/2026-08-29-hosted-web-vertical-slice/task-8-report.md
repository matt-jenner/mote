# Task 8 implementation report

Status: fix round 3 complete, awaiting task-scoped re-review.

Initial implementation commit: `e5c96b9` (`feat: serve the hosted photo viewer securely`), based on `bce40c4`.

Fix round 1 implementation commit: `90d48d1` (`fix: contain hosted static file access`), based on `edfd339`.

Fix round 2 implementation commit: `9ee0916` (`fix: bind hosted static roots at validation`), based on `dd8ef60`.

Fix round 3 implementation commit: `170071b` (`fix: pin hosted roots before validation`), based on `d480996`.

## Delivered behavior

- `photo-server` now composes the existing API, SSE, derivative, and `/healthz` routes ahead of one static fallback rooted at the validated `PHOTO_VIEWER_WEB_ROOT`.
- The static host uses `tower-http` `ServeDir` over a descriptor-relative backend, including ETags, conditional requests, ranges, HEAD parity, and precompressed Brotli/gzip variants when present. Hashed `/assets/*` responses use `public, max-age=31536000, immutable`; interface HTML uses `no-cache`.
- `/` and extensionless non-API GET/HEAD routes serve `index.html` without redirects. File-like misses remain 404, and unknown `/api/v1/*` and `/healthz/*` routes return fixed, path-free JSON 404 responses rather than the interface shell.
- Interface responses carry the planned CSP, `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`. The CSP remains:

  ```text
  default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'
  ```

- On Unix, `ServerConfig` opens and pins the configured web directory before deriving any display path, then rejects descriptor-identity or ancestry overlap with the opened source and local-state locations. The existing source-root containment, read-only scan/derivative behavior, request-target limits, 64 KiB JSON limit, opaque errors, SSE streaming, and bounded derivative streaming remain unchanged.
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

## Fix round 2: validated root identity and bounded route classification

The second Sol review found a remaining interval between web-root validation and descriptor acquisition. A different directory moved onto the validated pathname during that interval could be pinned without repeating overlap validation against the opened object. It also found that percent-encoded unreserved bytes could hide the reserved `api` or `healthz` first component from Axum route matching, and that static paths had no explicit traversal-independent size bounds. Commit `9ee0916` closes those three seams while retaining the round-1 descriptor-relative file backend.

### Behavioral RED evidence

The new tests were first run against `dd8ef60`:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host configured_web_root_swap_cannot_pin_and_serve_the_source_inode --jobs 2
  RED: after configuration, replacing the web-root pathname with the configured source directory caused `/` to return the exact bytes `SOURCE ROOT SENTINEL`.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host encoded_reserved_namespaces_return_the_fixed_api_miss_before_static_lookup --jobs 2
  RED: `/%61pi/v1/unknown` returned 200 with the exact bytes `PHYSICAL API SHADOW` from the web tree.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host oversized_static_paths_are_rejected_with_a_fixed_path_free_response --jobs 2
  RED: the first over-4096-byte extensionless path returned the SPA shell with 200 instead of a fixed 414. The completed matrix also covers 65 components and a 256-byte decoded component.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib path_limit_tests --jobs 2
  RED: the zero-traversal seam did not compile because `SecureBackend::filesystem_calls` did not exist.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib validated_root_identity_rejects_a_different_inode_before_pin --jobs 2
  RED: the identity-validation seam did not compile because `StaticWebRootValidation` did not exist.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib operating_system_name_limit_errors_are_path_free_not_found_errors --jobs 2
  RED: before the error mapping was added, `ENAMETOOLONG` remained `InvalidFilename` rather than the fixed path-free `NotFound` error.
```

### Root identity binding

- `ServerConfig` captures the canonical web directory plus its Unix device and inode before overlap checks. Source/web and private-state/web validation use that captured canonical object.
- Configuration then opens the root from `/` component by component with `O_DIRECTORY`, `O_NOFOLLOW`, and `O_CLOEXEC`, and requires the opened descriptor's `fstat` identity to equal the captured device and inode. A different inode fails startup with the fixed web-root-unavailable configuration error.
- The validated descriptor is retained inside a cloneable `StaticWebRoot`; all router clones share its `Arc<SecureRoot>`. Production startup no longer reopens `PHOTO_VIEWER_WEB_ROOT`. Renaming the validated inode after startup remains safe because serving continues through the pinned descriptor.
- The existing descendant `openat` chain still rejects symlinked or swapped ancestors, final files, index files, and Brotli/gzip sidecars. The static body handed to `tower-http` is still an already-open file descriptor.
- On non-Unix targets configuration installs the documented unavailable backend. API and health composition can start, but static content fails closed; no pathname-based fallback is installed.

### Reserved namespaces and static-path bounds

- One strict parser checks the raw path before allocating decoded storage, decodes once, validates components, and emits the canonical URI used by `ServeDir`.
- The explicit limits are 4,096 raw bytes, 4,096 decoded bytes, 64 components, and 255 decoded bytes per component. Any excess returns an empty, path-free 414 before the backend's `open` or `metadata` methods run. A test-only counter proves zero filesystem calls for each limit class.
- Malformed escapes, encoded dots or separators, traversal, backslashes, NUL, invalid UTF-8, and empty interior components remain fixed empty 400 responses. `ENAMETOOLONG` from an operating-system seam maps to a fixed path-free static miss rather than a 500 or native-path-bearing error.
- After decoding, an exact first component of `api` or `healthz` returns the same fixed JSON 404 contract as a literal reserved miss before static lookup. Encoded unreserved spellings such as `/%61pi` and `/h%65althz` cannot reach physical shadow files or the SPA shell. The boundary name `apiary` remains an ordinary static/interface namespace.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib static_host --jobs 2
  PASS: 6 tests, including root-identity swap, all three pre-traversal bounds, ENAMETOOLONG mapping, and prior descendant/sidecar swap cases

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test static_host --test health_api --jobs 2
  PASS: 24 tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --jobs 2
  PASS: 64 tests across library and integration targets

CARGO_BUILD_JOBS=2 cargo clippy --offline -p photo-server --all-targets --all-features --no-deps --jobs 2 -- -D warnings
  PASS
```

The configured-root integration test creates distinct validated web and source directories, constructs `ServerConfig`, then moves the source directory onto the configured web pathname before router construction. The router still returns `VALIDATED WEB ROOT` through the retained descriptor and never returns the source sentinel. The lower-level capture-to-pin hook separately proves that swapping to a different inode during the validation window is rejected.

### Final round-2 gates

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
  dist/index.html uses only `/assets/...` script and stylesheet URLs

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

The sandboxed browser run could not bind its temporary IPv6 loopback port (`EPERM`). The approved scoped rerun passed and emitted only the branch's known non-fatal `ViewerStage` `act(...)` warning. The desktop check regenerated its known `image` and `libc` dependency-list lines in the nested lockfile; those generated lines were removed, leaving the lockfile unchanged.

### Round-2 live release-process evidence

One foreground release process used the origin-relative hosted build, the existing six-photo demo directory as a read-only source, and fresh temporary data/cache directories:

- `/` and `/gallery/live-demo` returned 200 HTML with `no-cache`, ETag, no redirect, and the exact CSP/nosniff/referrer headers. Supplied Host and forwarded headers did not appear in the body or generate an absolute origin.
- `/healthz` returned healthy JSON with one available source and no path. Literal `/api/v1/unknown` and encoded `/%61pi/v1/unknown` both returned the fixed `{"code":"notFound","message":"That route is unavailable."}` JSON 404.
- A created selection exposed `text/event-stream`, `no-cache`, and `x-accel-buffering: no`. A bounded future replay returned a framed `wallUpdate` with authoritative `id: 0` and path-free `resyncRequired` data.
- A visible wall-thumbnail request returned 204. Its opaque derivative URL returned 200 `image/jpeg`, a 190,681-byte managed body, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

The process was interrupted in the foreground, port 18080 had no listener, and its temporary data/cache tree was removed. Generated `apps/interface/dist` remains ignored. No source photo changed; the complete source-safety harness passed. No background process, `cargo clean`, merge, push, OCI work, or Task 9 deployment work occurred.

### Round-2 limitation

Only the installed `aarch64-apple-darwin` target was compiled and executed. The non-Unix branch remains explicit fail-closed static behavior, but this round does not claim a Windows compile or runtime result. Windows static interface hosting remains disabled until an equivalent handle-relative, no-reparse implementation exists.

## Fix round 3: open-first root identity and descriptor ancestry

The final Sol review identified one remaining race in the round-2 design: it canonicalized the web-root pathname and then captured or reopened its identity. A pathname replacement between those operations could associate the earlier canonical path with a different directory. Path-only local-state checks also could not prove that two aliases referred to the same filesystem object. Commit `170071b` makes descriptor acquisition the identity-capture operation and uses held descriptors for the overlap decision.

### Behavioral RED evidence

The exact identity-capture seam was introduced as a test before the implementation:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib identity_capture_cannot_be_redirected_after_path_resolution --jobs 2
  RED (compile): StaticWebRootValidation::capture_with_identity_hook did not exist.

With a temporary hook placed between the old canonicalize and symlink_metadata calls:
  RED (behavior): replacing the configured pathname with the source directory made the old code pin and return the exact bytes `SOURCE ROOT SENTINEL` instead of `SAFE WEB ROOT`.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib pinned_root_survives_same_inode_rename_during_display_path_resolution --jobs 2
  RED: the old pathname-reopen design returned ENOENT after the already-validated inode was renamed.
```

The missing-local tests were characterization tests rather than claimed REDs: the path-only code already rejected ordinary missing children beneath source/web and accepted ordinary disjoint missing children. The round-3 change adds descriptor-backed evidence for those decisions. A `/dev/fd/<n>` directory alias was considered as a macOS fixture, but macOS can stat that alias while rejecting an `O_DIRECTORY` reopen with `ENOTDIR`; it was discarded rather than reported as security evidence. The final identity-alias test instead obtains a second independently owned descriptor using `openat(fd, ".")` and gives it a deliberately different informational label.

### Open-first retained identity

- `PinnedDirectory::open` performs one `O_DIRECTORY | O_CLOEXEC` open of the configured directory and immediately obtains device/inode identity with `fstat` through that same descriptor. An explicitly configured root symlink may be followed by this one root open; no later pathname reopen selects the served directory.
- `StaticWebRootValidation` owns that descriptor, and `pin` moves the same object into the cloneable static backend. Router clones share one `Arc<PinnedDirectory>` and every asset, index, Brotli, and gzip access remains descriptor-relative with no-follow descendant opens.
- Canonical and absolute paths are diagnostic values only. A canonical display value is accepted only when its identity still matches the held descriptor; otherwise the absolute configured spelling is retained as an informational label. `StaticWebRoot::path`, `ServerConfig::web_root`, and `PinnedDirectory::display_path` document that they have no security role.
- A same-inode rename during display-path resolution is accepted and serving continues through the retained descriptor. A different directory moved onto the configured pathname cannot affect the retained object or serve its sentinel.
- On non-Unix targets the existing explicit unavailable backend remains fail closed; there is still no pathname static fallback.

### Descriptor ancestry and missing local state

- Source and web roots are opened independently and compared by device/inode identity. Each ancestry walk clones its starting descriptor and repeatedly opens `..` relative to the current descriptor until the root identity repeats. The walk checks ancestry in both directions, bounds cycles/depth, propagates permission or traversal errors as startup failure, and releases every short-lived descriptor through RAII.
- Existing data/cache directories are opened and checked the same way. For a missing local-state path, validation lexically normalizes an absolute path, opens the nearest existing directory without creating anything, and retains the ordered missing-component remainder. If the pinned parent is inside source or web, startup fails; a disjoint missing child is allowed for later private-state preparation.
- Focused tests cover identical independently owned descriptors with distinct informational labels, real symlink alias rejection, source/web parent-child overlap in both directions, a missing child beneath source, a missing child beneath web, and disjoint missing data/cache children. They also preserve the existing descendant/symlink/sidecar swap coverage.
- The source descriptor and local-state validation descriptors are short-lived and close after configuration. The one web descriptor is intentionally retained for the router lifetime. Validation never creates or modifies source content.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib static_host --test health_api --jobs 2
  PASS: 9 static-host library tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api --jobs 2
  PASS: 13 tests

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --jobs 2
  PASS: 68 tests across library and integration targets

CARGO_BUILD_JOBS=2 cargo clippy --offline -p photo-server --all-targets --all-features --no-deps --jobs 2 -- -D warnings
  PASS
```

### Final round-3 gates

All commands below ran on the final implementation source before commit:

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
  dist/index.html uses root-relative `/assets/...`; emitted application API calls use `/api/v1/...`

CARGO_BUILD_JOBS=2 cargo test --workspace --all-targets --all-features --offline --jobs 2
  PASS: all workspace targets, including 68 photo-server tests and the source-safety harness

CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets --all-features --offline --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --offline --jobs 2
  PASS

CARGO_BUILD_JOBS=2 cargo build --release -p photo-server --offline --jobs 2
  PASS

git diff --check
  PASS
```

The browser gate emitted only the branch's known non-fatal `ViewerStage` `act(...)` warning. The desktop check regenerated the known `image` and `libc` dependency-list lines in its nested lockfile; those generated lines were removed, leaving the lockfile unchanged.

### Round-3 live release-process evidence

One foreground optimized process used the hosted build, the existing six-photo demo source, and fresh temporary data/cache directories:

- `/` and `/gallery/live-demo` returned identical 200 HTML with `no-cache`, ETag, no redirect, and the exact CSP/nosniff/referrer headers. Supplied hostile Host and forwarded headers were absent from the response body and did not create an absolute origin.
- `/healthz` returned healthy path-free JSON with one available source. Literal `/api/v1/unknown` and encoded `/%61pi/v1/unknown` returned the same fixed `{"code":"notFound","message":"That route is unavailable."}` JSON 404.
- Selection creation returned 201. A bounded future replay returned 200 `text/event-stream`, `no-cache`, `x-accel-buffering: no`, and a framed path-free `resyncRequired` event with authoritative `id: 0`.
- The wall contained all six demo photos. A visible wall-thumbnail request returned 204, after which the wall exposed an opaque derivative key. Its delivery returned 200 `image/jpeg`, 190,681 bytes, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

The process was interrupted in the foreground, `lsof` found no listener on port 18080, and the explicit temporary tree was removed. The initial sandboxed bind failed with `EPERM`; the approved loopback-only rerun supplied the live evidence. Generated `apps/interface/dist` remains ignored, no source photo changed, and no background process, `cargo clean`, merge, push, OCI work, or Task 9 deployment work occurred.

### Round-3 limitation

Only the installed `aarch64-apple-darwin` target was compiled and executed. The separately owned descriptor-alias test proves identity-based alias handling without requiring a privileged mount, but this environment did not create an actual bind mount. The non-Unix static branch remains explicit fail closed and was not compiled on Windows; Windows static interface hosting remains disabled until an equivalent handle-relative, no-reparse implementation exists.
