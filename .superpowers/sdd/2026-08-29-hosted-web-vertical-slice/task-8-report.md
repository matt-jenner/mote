# Task 8 implementation report

Status: fix round 5 complete, awaiting task-scoped re-review.

Initial implementation commit: `e5c96b9` (`feat: serve the hosted photo viewer securely`), based on `bce40c4`.

Fix round 1 implementation commit: `90d48d1` (`fix: contain hosted static file access`), based on `edfd339`.

Fix round 2 implementation commit: `9ee0916` (`fix: bind hosted static roots at validation`), based on `dd8ef60`.

Fix round 3 implementation commit: `170071b` (`fix: pin hosted roots before validation`), based on `d480996`.

Fix round 4 implementation commit: `6c0c47b` (`fix: enforce hosted mount boundaries`), based on `38a5f97`.

Fix round 5 implementation commit: `caa708e` (`fix: prevalidate hosted source startup`), based on `a4e0d6e`.

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
- The local-state validation descriptors are short-lived and close after configuration. Round 4 supersedes the source-descriptor lifetime described at this checkpoint: the configuration now retains the validated source descriptor through the operational startup boundary, then deliberately returns to path-based source access. The web descriptor remains retained for the router lifetime. Validation never creates or modifies source content.

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

## Fix round 4: mount identity and source startup consistency

The fourth review identified two different identity lifetimes. Device/inode ancestry alone cannot recognize a Linux bind-mounted web root because `..` follows the mountpoint namespace rather than the bound directory's physical ancestry. Separately, configuration validated a source descriptor but discarded it before `AppState` and `GalleryEngine` construction. Commit `6c0c47b` adds a static-only mount policy and retains a source validation lease through the operational startup boundary. It does not permanently pin the source inode.

### Behavioral RED evidence

The new tests were compiled against `38a5f97` before production changes:

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib synthetic_bind_mounts_fail_closed_even_when_directory_identity_matches --jobs 2
  RED (compile): MountIdentity, validate_mount_chain, and parse_fdinfo_mount_id did not exist.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib every_static_open_rejects_a_mount_transition_with_an_empty_response --jobs 2
  RED (compile): the mount provider and mount-aware pinned-root constructor did not exist.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api application_rejects_a_source_swap_at_the_operational_startup_boundary --jobs 2
  RED (compile): AppState had no source-startup validation seam.
```

An exploratory rootless Podman proof on Fedora 44, Linux 7.0.12, bind-mounted `/probe/source/child` at `/probe/app/web`. The source-child and web descriptors had the same `st_dev:st_ino` (`85:3`) but different mount IDs (`703` and `416`); the web descriptor's parent had mount ID `703`, and the exact `SOURCE_SENTINEL` was readable through the bind alias. This is supporting evidence for the production mount-ID abstraction, not an always-run or privileged repository test. The ephemeral container was removed.

The offline/remount expansion was a characterization gate rather than a product-behavior RED. It preserves the existing contract: after an offline cached-browsing session, remounting the source path and opening a new path-based engine resolves the same opaque selection and cached wall.

### Static mount policy

- On Linux, the retained web descriptor obtains a stable mount ID with `statx(fd, "", AT_EMPTY_PATH | AT_NO_AUTOMOUNT, STATX_MNT_ID)`. If that facility or mask is unavailable, a strict, bounded parser reads `mnt_id` from `/proc/self/fdinfo/<fd>`. Missing, duplicate, non-decimal, trailing, oversized, or inaccessible provider data fails closed with a fixed path-free error.
- Configuration walks retained ancestor descriptors to the stable namespace root. Linux permits no mount-ID transition. A web root that is itself a bind/separate mount, or is beneath an ancestor mount transition, is rejected before the router exists.
- Every descriptor-relative intermediate directory, final asset, interface index, Brotli sidecar, and gzip sidecar must match the pinned web root's mount identity. Provider failure or mismatch becomes fixed `PermissionDenied`; `tower-http` returns an empty 404 and does not treat a rejected compressed sidecar as a missing representation eligible for identity fallback.
- On macOS, mount identity is the strongest stable tuple exposed by `fstatfs`: raw `fsid`, filesystem type, mounted-on name, and mounted-from name. Every retained ancestor descriptor is inspected and every ordinary tuple transition is rejected. Every descendant/final static open still requires exact equality with the pinned web tuple.
- Normal writable macOS paths cross the built-in APFS Data-to-System firmlink/volume-group boundary. The observed tuple was `apfs`, `/System/Volumes/Data`, `/dev/disk3s5`, fsid bytes `[17,0,0,1,26,0,0,0]` to `apfs`, `/`, `/dev/disk3s1s1`, fsid bytes `[19,0,0,1,26,0,0,0]`. The only exception requires those exact mount-on roles, `apfs` on both sides, canonical `/dev/diskNs...` sources, and the same physical `diskN`. Negative tests cover wrong child and parent mount points, `nullfs`, a different physical store, and malformed device sources.
- Unix platforms other than Linux and macOS, and non-Unix platforms, retain the explicit unavailable static backend. API and health behavior remains fail closed rather than reopening static paths unsafely.

### Source startup lease

- `ServerConfig` retains the descriptor and device/inode identity captured by the configuration-time source open. `AppState::open` opens and fstats the operational source path before catalog preflight or local-state preparation, requires it to match, and holds that new lease through contained-folder and `GalleryEngine` construction.
- The operational pathname is revalidated against the lease immediately before contained-folder construction, immediately before gallery construction, and immediately after gallery construction. A deterministic swap to a replacement directory is rejected before any library or sentinel is cataloged; unchanged identity succeeds.
- The lease is intentionally dropped after successful startup. Gallery scans and later availability checks retain the product's existing path-based offline/remount behavior. This is startup identity consistency, not a promise that the source inode remains permanently pinned.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --lib --jobs 2
  PASS: 16 tests, including synthetic bind identity, strict parser/provider failure, exact macOS boundary, and empty-response intermediate/final/Brotli/gzip mismatch coverage.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api application_rejects_a_source_swap_at_the_operational_startup_boundary --jobs 2
  PASS: replacement identity rejected before catalog admission.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --test health_api unchanged_source_identity_crosses_the_operational_startup_boundary --jobs 2
  PASS.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-app-service --test hosted_selections completed_selection_reopens_for_cached_browsing_when_source_is_offline --jobs 2
  PASS: offline cached browsing and path-based remount/reopen resolve the same selection and one-item cached wall.

CARGO_BUILD_JOBS=2 cargo test --offline -p photo-server --jobs 2
  PASS: 75 tests across library and integration targets.

CARGO_BUILD_JOBS=2 cargo clippy --offline -p photo-server --all-targets -- -D warnings
  PASS.
```

### Final round-4 gates

All final gates ran on the exact implementation source committed as `6c0c47b`:

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
  PASS on approved loopback rerun: 4 files, 165 tests

npm run web:build
  PASS: Vite 8.2.2 transformed 1,891 modules; index asset URLs remain root-relative `/assets/...` and application API URLs remain `/api/v1/...`

CARGO_BUILD_JOBS=2 cargo test --workspace --all-targets --all-features --offline --jobs 2
  PASS: all workspace targets and doc tests, including the source-safety harness and the new 26-test hosted-selection suite

CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets --all-features --offline --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --offline --jobs 2
  PASS

CARGO_BUILD_JOBS=2 cargo build --release -p photo-server --offline --jobs 2
  PASS

git diff --check
  PASS
```

The first browser attempt was blocked before test collection by sandboxed IPv6 loopback bind `EPERM`; the approved loopback rerun passed with only the branch's known non-fatal `ViewerStage` `act(...)` warning. The desktop check regenerated its known `image` and `libc` dependency-list entries; removing only those generated lines restored the prior nested-lock checksum `213cfa1d6961bb7bdb72326f64517afc2bda09ea`.

### Round-4 live release-process evidence

One foreground optimized process used the built hosted interface, the existing six-photo demo source, and fresh temporary data/cache directories:

- `/gallery/live-demo` returned 200 HTML with `no-cache`, a quoted ETag, no redirect, and the exact CSP, `nosniff`, and referrer headers. Host `hostile.example` and forwarded host `forwarded.example` were absent from the body; all interface asset links remained origin-relative.
- `/healthz` returned healthy path-free JSON with one available source. Encoded `/%61pi/v1/unknown` returned the fixed path-free JSON 404.
- Selection creation returned 201 and the wall exposed all six photos. A bounded future replay returned 200 `text/event-stream`, `no-cache`, `x-accel-buffering: no`, and a framed path-free `resyncRequired` event with authoritative `id: 0`.
- A visible wall-thumbnail request returned 204. The resulting opaque derivative returned 200 `image/jpeg`, 190,681 bytes, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

The process was interrupted in its foreground session, `lsof` found no listener on port 18080, the explicit temporary tree was removed, and the source-photo aggregate checksum remained `aaf9c7784207843dd4ece72ddc799f196a7c3a7d`. Generated `apps/interface/dist` remains ignored. No background build, `cargo clean`, merge, push, OCI work, Task 9 packaging, or deployment work occurred.

### Round-4 limitations

Only `aarch64-apple-darwin` was compiled and executed in this workspace. Linux mount-ID behavior is backed by the real rootless-container proof and platform-independent synthetic/parser tests, but this report does not claim a Linux Rust build or runtime test. If both Linux `statx` and bounded `/proc/self/fdinfo` access are unavailable, static startup fails closed. macOS does not expose a Linux-equivalent mount ID through this API; the `fstatfs` tuple detects distinct mount identities but cannot prove separation if the kernel presents an alias with an identical full tuple. Windows and other unsupported platforms continue to disable static hosting until they have an equivalent handle-relative, no-reparse and stable-mount implementation.

## Fix round 5: one-shot prevalidated source startup

The fifth Sol review found a remaining startup interval: the server validated the configured source descriptor, then both `ContainedFolderRoot` and `GalleryEngine` reopened the pathname before a later identity check. A transient symlink replacement could therefore be adopted by either constructor and restored before the post-check. The configuration also retained its original source descriptor indefinitely through every clone, which contradicted the intended offline/remount lifecycle.

### Behavioral RED evidence

Two deterministic stage hooks reproduced the old behavior on `a4e0d6e`:

```text
CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test health_api transient_source_swap_cannot_retarget_the_folder_root -- --exact --nocapture
  RED: the folder API returned `source-sentinel-album` from the transient replacement instead of `safe-album` from the configured source.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test health_api transient_source_swap_cannot_change_the_configured_library_key -- --exact --nocapture
  RED: the post-check passed after restoration, but the catalog stored `/.../replacement` rather than `/.../photos`.
```

These are behavioral failures at the two exact pathname-reopen windows. They do not infer sentinel admission from a compile failure or from the earlier pre-validation swap test.

### Prevalidated startup implementation

- `ServerConfig` now owns the source validation inside one shared `Arc<Mutex<Option<...>>>`. All configuration clones share that slot. `AppState::open` atomically takes it; a second start receives the typed, path-free `SourceStartupUnavailable` error instead of reusing or duplicating the descriptor.
- Taking the slot opens and fstats the current configured source path once and requires the same device/inode identity captured during configuration. The resulting move-only lease owns that operational directory descriptor.
- `ContainedFolderRoot::from_prevalidated_operational_path` stores the configured operational path without canonicalizing, testing, listing, or reopening it during startup.
- The server-only `server-internal-prevalidated-source` app-service feature exposes a doc-hidden capability requiring an owned `File`, not a path-only assertion. `GalleryEngine::open_prevalidated_hosted` consumes that capability, uses the normalized configured key verbatim for exact lookup or catalog admission, and performs no source-root canonicalize/is-dir/read operation.
- Local-state and catalog overlap checks remain active. Their prevalidated path normalizes absolute and dot components without source I/O; on macOS it also normalizes only the exact built-in `/var`, `/tmp`, and `/etc` aliases to their `/private/...` forms. A negative `/various` case prevents prefix overmatching. An overlapping normalized catalog key returns typed `AddLibraryError::Overlaps` and does not admit another library.
- The validated descriptor and its lifecycle marker are dropped on every success, error, or unwind path before `AppState::open` returns. A source moved offline immediately after the atomic validation remains untouched through both constructors; startup records only the configured key. Runtime folder resolution, scans, availability checks, and a later remount continue to use the configured pathname exactly as before.

The ordinary public `GalleryEngine::open` and `ContainedFolderRoot::new` paths remain unchanged for desktop and non-hosted callers. Within this workspace only `photo-server` enables the internal feature. Rust cannot restrict a public cross-crate constructor to one named downstream crate, so the API is doc-hidden, explicitly server-internal, and requires ownership of the validated descriptor; it cannot be fabricated from paths alone.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test health_api
  PASS: 22 tests.
  Both constructor-window swaps keep the configured folder/key; moving the source offline after operational validation succeeds without a source reopen; clones share one consumed slot; the descriptor marker releases on success, pre-Gallery error, in-Gallery overlap error, and unwind; second startup is typed and path-free.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-core --test local_state
  PASS: 5 tests, including missing-source no-I/O overlap checks and exact macOS alias/near-miss normalization.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-app-service --test hosted_selections completed_selection_reopens_for_cached_browsing_when_source_is_offline -- --exact --nocapture
  PASS: cached offline browsing and later path remount resolve the same selection.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server
  PASS: 82 tests across library, derivative, event, folder, gallery, health, and static-host targets.

CARGO_BUILD_JOBS=2 cargo clippy --offline --jobs 2 -p photo-server --all-targets -- -D warnings
  PASS.

CARGO_BUILD_JOBS=2 cargo clippy --offline --jobs 2 -p photo-app-service --all-targets --no-default-features -- -D warnings
  PASS: the ordinary non-hosted feature shape remains warning-free.

CARGO_BUILD_JOBS=2 cargo check --offline --jobs 2 --release -p photo-server
  PASS.
```

### Final round-5 gates

All gates ran on the exact implementation source committed as `caa708e`:

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
  PASS on approved loopback rerun: 4 files, 165 tests

npm run web:build
  PASS: Vite 8.2.2 transformed 1,891 modules; index script and stylesheet URLs are root-relative `/assets/...`

CARGO_BUILD_JOBS=2 cargo test --offline --workspace --jobs 2
  PASS: all workspace and doc tests, including the source-safety harness, 26 hosted-selection tests, 57 progressive-wall tests, and the 22-test source-startup/server-health suite

CARGO_BUILD_JOBS=2 cargo clippy --offline --workspace --all-targets --all-features --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo check --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --jobs 2
  PASS

CARGO_BUILD_JOBS=2 cargo build --offline --release -p photo-server --jobs 2
  PASS

git diff --check
  PASS
```

The first browser attempt was blocked before collection by the sandboxed IPv6 loopback bind (`EPERM`); the approved scoped rerun passed with only the branch's existing non-fatal `ViewerStage` `act(...)` warning. The desktop check added its known generated `image` and `libc` dependency-list lines; removing only those lines restored the exact pre-check nested-lock checksum `60ddf95256a7c20613c9f9f700db5793ab4bc04ce11cfa9375d9076612594a4c`.

### Round-5 live release-process evidence

One foreground optimized process used the hosted build, the existing six-photo demo source, and fresh explicit temporary data/cache directories:

- `/gallery/live-demo` returned 200 HTML with `no-cache`, a quoted ETag, no redirect, the exact CSP, `nosniff`, and referrer headers. Host `hostile.example` and forwarded host `forwarded.example` were absent from the body; the generated script and stylesheet references remained root-relative.
- `/healthz` returned healthy, path-free JSON with one available source. Encoded `/%61pi/v1/unknown` returned the fixed JSON 404 envelope.
- Creating the root selection returned 201, and the wall exposed all six photos. A bounded future replay returned 200 `text/event-stream`, `no-cache`, `x-accel-buffering: no`, and one path-free `resyncRequired` event with authoritative `id: 0`.
- A visible wall-thumbnail request returned 204. The resulting opaque derivative returned 200 `image/jpeg`, 190,681 bytes, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

All six source hashes were identical before and after the live probe. The server was interrupted in its foreground session, `lsof` found no listener on port 18080, and the explicit temporary tree was removed. Generated `apps/interface/dist` remains ignored. No source photo changed, and no background build, `cargo clean`, merge, push, OCI work, Task 9 packaging, or deployment work occurred.

### Round-5 limitations

- Source identity is guaranteed across the hosted startup admission boundary, not pinned for the process lifetime. This is deliberate: after startup, path-based offline/remount semantics remain the product contract.
- The installed and executed target remains `aarch64-apple-darwin`. Linux mount-ID behavior retains the round-4 synthetic/parser and rootless-container evidence; unsupported Unix and non-Unix static hosting remains fail closed.
- The macOS no-I/O key normalizer handles only the three exact built-in aliases above. Arbitrary source symlink aliases are admitted initially only through the pinned configuration descriptor and stored under its canonical configured key; the prevalidated catalog path never follows arbitrary aliases.

## Fix round 6: explicit trust capabilities and source-disabled startup

The sixth review found two bounded problems. Core local-state code still exposed safe raw-path methods for checks that intentionally skip source I/O, and the server-only hosted-source constructor was safe even though a caller could supply an unrelated regular file and arbitrary paths. Separately, non-Unix startup always tried to convert its source validation into a Unix-only token and returned `SourceRootChanged`, so API and health could not start even though static hosting was already designed to fail closed there.

Implementation commit `6825f70` makes both trust boundaries explicit and restores the non-Unix source-disabled composition.

### Behavioral RED evidence

The new tests ran against `81a729a` before production changes:

```text
CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-app-service --features server-internal-prevalidated-source arbitrary_regular_file_cannot_fabricate_a_prevalidated_hosted_source -- --nocapture
  RED: the safe constructor accepted a regular-file descriptor containing `source sentinel`; `GalleryEngine::open_prevalidated_hosted` returned success and cataloged it, so the rejection assertion failed.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test source_disabled --no-run
  RED: E0599 for both missing seams, `AppState::open_source_disabled_for_test` and `StaticWebRoot::unavailable_for_test`.
```

The first result is a behavioral trust-bypass reproduction. The second is compile evidence that the source-disabled composition did not exist; it is not presented as a Windows runtime failure.

### Sealed trust entry points

- `PrevalidatedSourceKeys` has private fields and exists only behind the internal feature. Its doc-hidden constructor is `unsafe`, rejects relative inputs with a fixed error, and stores normalized keys. `LocalStatePaths::{validate,prepare}_prevalidated_source_keys` now accept only this capability, not raw `PathBuf` slices.
- `PrevalidatedHostedSource::from_server_validated_directory` is doc-hidden and `unsafe`. It requires an owned validation descriptor, rejects a non-directory descriptor, and rejects non-absolute or non-normalized operational and canonical keys with fixed path-free errors.
- The constructor's Safety contract states the part runtime checks cannot prove: the descriptor, operational path, and canonical key must identify the same directory and must have been validated together without a pathname reopen interval. `SourceStartupLease` is the production constructor call site and has an audited safety comment tying that obligation to the retained and identity-matched descriptor.
- The server and hosted gallery create source-key capabilities only in small documented unsafe blocks. The configured key comes from the pinned startup validation. Other keys come from the catalog's typed canonical-root field and are used only for no-source-I/O lexical overlap checks.

Safe callers cannot fabricate either capability or invoke either unsafe constructor. Rust cannot stop a caller that explicitly enables the internal feature and writes an incorrect unsafe block, so this report does not claim absolute impossibility. Runtime directory and key checks still reject the demonstrated regular-file and malformed-key misuse.

### Non-Unix source-disabled composition

- Unix keeps the one-shot pinned-source startup path. Release Unix builds do not contain a way to select the source-disabled helper.
- On non-Unix, `AppState::open` now uses a separate composer. It reads persisted catalog roots, performs the ordinary path-based local-state overlap validation and preparation, opens the catalog, reconciles the cache, and returns `AppState::new`. The resulting state has no folder root or gallery engine.
- API and health stay available in that state. Bootstrap reports `folderBrowser: false` and `sourceAvailable: false`. Folder, selection, wall, event, and derivative handlers retain their existing fixed, path-free `sourceUnavailable` responses. The unsupported static backend returns an empty 404 and never opens the web pathname.
- A debug-only seam runs the exact composer on the installed macOS target. Its test also creates and verifies removal of a partial cache file, so the evidence covers preparation and reconciliation rather than state construction alone. A `cfg(not(unix))` test calls the production branch when such a target is available.

Desktop continues to use the ordinary path-based `GalleryEngine` constructors. Hosted Unix source startup still drops its one-shot descriptor before returning and later uses the configured operational path, so offline and remount behavior is unchanged.

### Focused GREEN evidence

```text
CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-app-service --features server-internal-prevalidated-source prevalidated_hosted_source_tests -- --nocapture
  PASS: 2 tests. Regular-file descriptors and non-normalized absolute keys return fixed `InvalidInput` errors.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-core --features server-internal-prevalidated-source --test local_state -- --nocapture
  PASS: 6 tests, including relative capability rejection and no-I/O checks for missing source keys.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test source_disabled -- --nocapture
  PASS: 1 test. Local-state directories were prepared, one partial cache file was reconciled, health returned 200, API source routes returned fixed path-free 503 envelopes, and static returned an empty 404.

CARGO_BUILD_JOBS=2 cargo test --offline --jobs 2 -p photo-server --test health_api --test static_host --test source_disabled
  PASS: health 22, static host 12, source-disabled composition 1.

CARGO_BUILD_JOBS=2 cargo clippy --offline --jobs 2 -p photo-core -p photo-app-service -p photo-server --all-targets --all-features -- -D warnings
  PASS.
```

One source-disabled test fixture first used the invalid query value `direction=newest`; the API correctly rejected it with 400 before the missing-gallery guard. Changing the fixture to the contract value `newestFirst` made the intended 503 assertion reach the source-disabled path. One Clippy finding was a mechanical needless borrow in the new capability loop.

### Final round-6 gates

All final gates ran on the source committed as `6825f70`:

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
  PASS on the approved loopback rerun: 4 files, 165 tests

npm run web:build
  PASS: Vite 8.2.2 transformed 1,891 modules; generated script and stylesheet URLs are root-relative `/assets/...`, and application API/EventSource URLs remain `/api/v1/...`

CARGO_BUILD_JOBS=2 cargo test --offline --workspace --all-targets --all-features --jobs 2
  PASS: every workspace target, including app-service 104 unit tests, hosted selections 26, progressive wall 57, source safety, server library 16, health 22, static host 12, source-disabled 1, and core local-state 6

CARGO_BUILD_JOBS=2 cargo clippy --offline --workspace --all-targets --all-features --jobs 2 -- -D warnings
  PASS

CARGO_BUILD_JOBS=2 cargo clippy --offline --jobs 2 -p photo-app-service --all-targets --no-default-features -- -D warnings
  PASS: the ordinary desktop/non-hosted feature shape remains warning-free

CARGO_BUILD_JOBS=2 cargo check --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --jobs 2
  PASS

CARGO_BUILD_JOBS=2 cargo build --offline --release -p photo-server --jobs 2
  PASS without warnings

git diff --check
  PASS
```

The first workspace-test attempt reached linking and stopped with `errno=28`, no space left on device; no test assertion failed. The worktree target directory was 26 GiB, including 8.4 GiB of regenerable incremental cache. Removing only `target/debug/incremental`, rather than running `cargo clean`, restored enough space. The exact workspace command then passed, and it passed again after the final release-only `cfg` cleanup. The sandboxed browser attempt was blocked before collection by IPv6 loopback bind `EPERM`; the approved rerun passed with the branch's existing non-fatal `ViewerStage` `act(...)` warning. The desktop check regenerated its known `image` and `libc` dependency-list lines; removing only those generated lines restored the nested lockfile checksum `60ddf95256a7c20613c9f9f700db5793ab4bc04ce11cfa9375d9076612594a4c`.

### Round-6 live release-process evidence

One foreground optimized process used the built hosted interface, the six-photo demo source, and fresh explicit temporary data and cache directories:

- `/gallery/live-demo` returned 200 HTML with `no-cache`, a quoted ETag, no redirect, the exact CSP, `nosniff`, and referrer headers. Host `hostile.example` and forwarded host `forwarded.example` were absent from the body. The script and stylesheet references were root-relative.
- `/healthz` returned healthy path-free JSON with one available source. Encoded `/%61pi/v1/unknown` returned the fixed JSON 404 envelope.
- Root selection creation returned 201 and the settled wall contained all six photos. A bounded future replay returned 200 `text/event-stream`, `no-cache`, `x-accel-buffering: no`, and the framed path-free `resyncRequired` event with authoritative `id: 0`.
- A visible wall-thumbnail request returned 204. Its opaque derivative key returned 200 `image/jpeg`, 190,681 bytes, `public, max-age=31536000, immutable`, `nosniff`, and an opaque quoted ETag.

The six source hashes matched before and after the probe. The server was interrupted in its foreground session, `lsof` found no listener on port 18080, and the explicit temporary directory was removed. Generated `apps/interface/dist` remains ignored. No source photo changed, and no background build, `cargo clean`, merge, push, OCI work, Task 9 packaging, or deployment work occurred.

### Round-6 limitations

- Only `aarch64-apple-darwin` was compiled and executed. The debug seam exercises the source-disabled state on macOS, and a `cfg(not(unix))` test describes the production branch, but this round has no Windows-target compile or runtime evidence.
- Windows and other unsupported static platforms still return the explicit unavailable backend. API and health can start there, but the interface remains disabled until a handle-relative, no-reparse static implementation exists.
- Hosted source identity remains a startup guarantee. Runtime scans intentionally retain path-based offline and remount behavior after the startup capability is consumed.
