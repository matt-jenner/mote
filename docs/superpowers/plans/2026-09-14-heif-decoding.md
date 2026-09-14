# HEIF Decoding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make common iPhone `.heic` and `.heif` still images viewable by default in Mote on desktop and hosted builds, while a Mote-facing `--no-heic` build option removes only HEIF support and its native libraries.

**Architecture:** A new `photo-codec` crate becomes the sole source-image decoding boundary. Existing formats keep using `image`; the default `heic` feature adds the direct `libheif-rs` API backed by dynamically linked, decode-only `libheif` and `libde265`. Media capability drives indexing, wall queries, progress totals, and derivative scheduling. Entry-point feature bundles and packaging scripts expose one default-on/no-HEIC choice without leaking Cargo's additive-feature behavior.

**Tech Stack:** Rust 1.97.1, `image` 0.25.10, `libheif-rs` 3.0.0, `libheif` 1.23.4, `libde265` 1.1.1, Cargo features, CMake, pkg-config, Tauri 2, Flatpak, OCI containers, GitHub Actions, Node test runner

**Spec:** `docs/superpowers/specs/2026-09-14-heif-decoding-design.md`

## Global Constraints

- HEIF is decode-only. Do not enable or ship `x265`, `heif-enc`, an encoder plugin, AV1 codecs, or unrelated libheif codec plugins.
- Mote retains its own project licence. `libheif` and `libde265` remain separately replaceable, dynamically linked LGPL components in distributed packages.
- Normal builds include HEIF. `--no-heic` must retain every normal non-HEIF feature and must not compile, link, or bundle either LGPL library.
- `.heic` and `.heif` recognition is unconditional. Build capability alone controls whether those assets enter photo queries, progress totals, and derivative work.
- Decode only the declared primary still. Ignore sequences, Live Photo video, alternate images, auxiliary depth images, and gain maps.
- Apply HEIF crop, rotate, and mirror transforms once. Do not apply embedded EXIF orientation again to HEIF pixels.
- Normalize decoded HEIF pixels to 8-bit SDR sRGB before representative-colour calculation, resize, and JPEG derivative encoding.
- Keep the existing 1024-pixel wall-thumbnail and 4096-pixel screen-preview policies and existing JPEG qualities.
- Keep non-HEIF decoder output and the `image-0.25-v1` derivative fingerprint unchanged.
- Use `libheif-1.23.4-libde265-1.1.1-sdr-v1` as the first HEIF decoder fingerprint.
- Read source files only. No decoder or test may change source bytes, timestamps, names, or permissions.
- Configure strict parsing and explicit production limits: 512 MiB encoded input, 150 million decoded pixels, 1,024 items, 4,096 tiles, 16 MiB colour profiles, 512 MiB for one allocation, and 768 MiB total libheif memory.
- Environment variables must not disable libheif security limits in production.
- Keep the existing untracked `dist/` tree untouched.

---

### Task 1: Lock the default-on feature graph

**Files:**
- Create: `tests/build/heic-feature-contract.test.mjs`
- Create: `crates/codec/Cargo.toml`
- Create: `crates/codec/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `crates/domain/Cargo.toml`
- Modify: `crates/catalog/Cargo.toml`
- Modify: `crates/metadata/Cargo.toml`
- Modify: `crates/indexer/Cargo.toml`
- Modify: `crates/cache/Cargo.toml`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/app-service/Cargo.toml`
- Modify: `crates/server/Cargo.toml`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `package.json`
- Modify: `Cargo.lock`
- Modify: `apps/desktop/src-tauri/Cargo.lock`

**Interfaces:**
- `photo-codec/default = ["heic"]`
- Entry crates expose `default = ["mote-defaults", "heic"]`, `mote-defaults`, and `heic`.
- Every internal path dependency uses `default-features = false`; each affected crate forwards `heic` explicitly.
- `libheif-rs = { version = "=3.0.0", default-features = false, features = ["v1_23"], optional = true }`

- [ ] **Step 1: Add a failing manifest contract test**

Create `tests/build/heic-feature-contract.test.mjs`. Parse the relevant manifests as text and assert all of these invariants:

```js
test("HEIC is a default feature with a stable non-HEIC bundle", () => {
	for (const manifest of [serverManifest, desktopManifest]) {
		assert.match(manifest, /default\s*=\s*\["mote-defaults",\s*"heic"\]/);
		assert.match(manifest, /mote-defaults\s*=\s*\[/);
		assert.match(manifest, /heic\s*=\s*\[/);
	}
});

test("the native binding is optional and never embedded", () => {
	assert.match(codecManifest, /libheif-rs.*optional\s*=\s*true/);
	assert.doesNotMatch(codecManifest, /embedded-libheif/);
});

test("internal dependencies do not activate defaults implicitly", () => {
	for (const [name, manifest] of internalManifests) {
		for (const line of manifest.split(/\r?\n/).filter((line) => /photo-[a-z-]+\s*=/.test(line))) {
			assert.match(line, /default-features\s*=\s*false/, `${name}: ${line}`);
		}
	}
});
```

Add `node --test tests/build/*.test.mjs` to the existing build-test script rather than creating a competing command.

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
node --test tests/build/heic-feature-contract.test.mjs
```

Expected: FAIL because neither `photo-codec` nor the default feature bundles exist.

- [ ] **Step 3: Create the crate and forward the feature deliberately**

Add `crates/codec` to the wildcard workspace with this public shell:

```rust
#![forbid(unsafe_code)]

pub const IMAGE_DECODER_FINGERPRINT: &str = "image-0.25-v1";
#[cfg(feature = "heic")]
pub const HEIC_DECODER_FINGERPRINT: &str =
    "libheif-1.23.4-libde265-1.1.1-sdr-v1";
```

Use this feature shape in `crates/codec/Cargo.toml`:

```toml
[features]
default = ["heic"]
heic = ["dep:libheif-rs", "photo-domain/heic"]

[dependencies]
photo-domain = { path = "../domain", default-features = false }
image.workspace = true
imagesize.workspace = true
thiserror.workspace = true
libheif-rs = { version = "=3.0.0", default-features = false, features = ["v1_23"], optional = true }
```

Add `default = ["heic"]` and an explicit `heic` forwarding list to each affected workspace crate. At the server and desktop entry points, use:

```toml
[features]
default = ["mote-defaults", "heic"]
mote-defaults = []
heic = ["photo-app-service/heic"]
```

Keep `server-internal-prevalidated-source` explicitly enabled for `photo-server`; it is not an optional normal feature. Do not rely on a downstream crate's defaults anywhere in the workspace graph.

- [ ] **Step 4: Prove the enabled and disabled dependency graphs**

Resolve and inspect the feature graphs without compiling native code:

```bash
cargo metadata --format-version 1 --no-deps
cargo tree -p photo-server -e features
cargo tree -p photo-server -e features --no-default-features --features mote-defaults
cargo tree --manifest-path apps/desktop/src-tauri/Cargo.toml -e features
cargo tree --manifest-path apps/desktop/src-tauri/Cargo.toml -e features --no-default-features --features mote-defaults
node --test tests/build/heic-feature-contract.test.mjs
```

Expected: default trees contain `libheif-rs`; both `mote-defaults`-only trees omit `libheif-rs`; the Node contract test passes.

- [ ] **Step 5: Commit the feature graph**

```bash
git add Cargo.toml Cargo.lock package.json crates apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/Cargo.lock tests/build/heic-feature-contract.test.mjs
git commit -m "build: define default HEIC feature graph"
```

---

### Task 2: Put existing image decoding behind `photo-codec`

**Files:**
- Create: `crates/codec/src/error.rs`
- Create: `crates/codec/src/image_backend.rs`
- Create: `crates/codec/tests/image_backend.rs`
- Modify: `crates/codec/src/lib.rs`
- Modify: `crates/metadata/src/probe.rs`
- Modify: `crates/cache/src/image_derivative.rs`
- Modify: `crates/metadata/Cargo.toml`
- Modify: `crates/cache/Cargo.toml`

**Interfaces:**

```rust
pub struct DisplayShape { pub width: u32, pub height: u32 }
pub enum CodecLimit {
    EncodedBytes,
    DecodedPixels,
    ItemCount,
    TileCount,
    ColourProfileBytes,
    AllocationBytes,
    TotalMemoryBytes,
}
pub enum CodecError {
    Io { path: PathBuf, kind: io::ErrorKind, message: String },
    Unsupported { kind: MediaKind },
    LimitExceeded { limit: CodecLimit, actual: u64, maximum: u64 },
    Container { message: String },
    Decode { message: String },
    MissingPrimaryImage,
    MissingInterleavedPlane,
    InvalidPixelLayout { width: u32, height: u32, stride: usize },
    InvalidExif { message: String },
}
pub fn display_shape(path: &Path, kind: MediaKind) -> Result<DisplayShape, CodecError>;
pub fn decode_display_image(path: &Path, kind: MediaKind, orientation: u16)
    -> Result<DynamicImage, CodecError>;
pub fn decoder_fingerprint(kind: MediaKind) -> Result<&'static str, CodecError>;
```

- [ ] **Step 1: Add failing compatibility tests for existing formats**

In `crates/codec/tests/image_backend.rs`, generate small asymmetric JPEG, PNG, TIFF, and WebP fixtures in a temporary directory. For each type, compare dimensions and exact decoded RGBA bytes from `photo_codec::decode_display_image` with the current `image::ImageReader` path. Add orientation cases for values 1 through 8 using a 3-by-2 image with six distinct pixels.

Also assert:

```rust
assert_eq!(decoder_fingerprint(MediaKind::Jpeg).unwrap(), "image-0.25-v1");
assert_eq!(decoder_fingerprint(MediaKind::Webp).unwrap(), "image-0.25-v1");
assert!(matches!(decoder_fingerprint(MediaKind::Video), Err(CodecError::Unsupported { .. })));
```

- [ ] **Step 2: Run the focused test and verify it fails**

```bash
cargo test -p photo-codec --test image_backend --no-default-features
```

Expected: FAIL because the decoding functions and error type do not exist.

- [ ] **Step 3: Implement the image backend and typed errors**

Move the current `ImageReader::open(...).with_guessed_format()?.decode()?` and orientation transform mapping from `photo-cache` into `image_backend.rs`. Keep the current order and return type so JPEG, PNG, TIFF, and WebP output stays byte-for-byte equivalent. `display_shape` uses `imagesize` for these formats. Reject `Heif` when the crate is compiled without `heic`; reject `Raw`, `Avif`, `Video`, and `Unknown` in all builds.

`CodecError` must retain `std::io::ErrorKind` for I/O failures and convert third-party decode errors to owned diagnostic text at the boundary. It must not expose `libheif-rs` types in the public API.

- [ ] **Step 4: Route metadata probing and derivatives through the boundary**

Replace direct source-pixel `ImageReader` calls in `crates/metadata/src/probe.rs` and `crates/cache/src/image_derivative.rs` with `photo_codec` calls. Keep `image::load_from_memory` for Mote's already-generated JPEG cache entries. Map `CodecError` into the existing per-asset derivative and metadata error categories.

- [ ] **Step 5: Run compatibility tests**

```bash
cargo test -p photo-codec --no-default-features
cargo test -p photo-metadata --no-default-features
cargo test -p photo-cache --no-default-features
```

Expected: PASS with no native HEIF library required.

- [ ] **Step 6: Commit the shared boundary**

```bash
git add crates/codec crates/metadata crates/cache
git commit -m "refactor: centralize source image decoding"
```

---

### Task 3: Bootstrap the pinned native decoder for development

**Files:**
- Create: `packaging/heic/versions.env`
- Create: `packaging/heic/build-unix.sh`
- Create: `tests/packaging/heic-native-build.test.mjs`
- Modify: `.gitignore`

**Interfaces:**
- `packaging/heic/build-unix.sh --platform macos|linux --arch ARCH` installs shared decode-only libraries below an ignored `build/heic-native/` prefix.
- `packaging/heic/versions.env` is the single literal source of native versions, release archive URLs, and SHA-256 hashes.
- The script prints shell-safe `PKG_CONFIG_PATH` and dynamic-loader settings for the caller; it does not install into `/usr/local` or another host-wide prefix.

- [ ] **Step 1: Add a failing pinned-build contract test**

Create `tests/packaging/heic-native-build.test.mjs` and assert that the version file contains libde265 1.1.1 and libheif 1.23.4 with HTTPS archive URLs and 64-character lowercase SHA-256 values. Assert the Unix script verifies hashes before extraction, uses `BUILD_SHARED_LIBS=ON`, enables the in-process libde265 decoder, and explicitly disables x265, encoders, examples, tests, experimental APIs, and plugin loading.

- [ ] **Step 2: Run the focused test and verify it fails**

```bash
node --test tests/packaging/heic-native-build.test.mjs
```

Expected: FAIL because the version and native-build files do not exist.

- [ ] **Step 3: Implement the local Unix builder**

Write literal pins and hashes to `versions.env`. The builder downloads into a content-addressed cache, verifies SHA-256, extracts into the lifecycle-managed build tree, builds shared libde265 first, then shared libheif, and installs both under `build/heic-native/<platform>-<arch>`. Use the complete decode-only CMake switch set defined in Global Constraints and fail if CMake reports an unused or unknown codec switch.

Add only `build/heic-native/` to `.gitignore`. Do not ignore or remove the user's existing `dist/` directory.

- [ ] **Step 4: Build the current host dependency and rerun the contract**

```bash
node --test tests/packaging/heic-native-build.test.mjs
packaging/heic/build-unix.sh --platform macos --arch "$(uname -m)"
```

On Linux, substitute `--platform linux`. Expected: both shared libraries and `libheif.pc` exist below the managed prefix; no x265 library, encoder plugin, or `heif-enc` exists there.

- [ ] **Step 5: Commit the reproducible development dependency**

```bash
git add .gitignore packaging/heic/versions.env packaging/heic/build-unix.sh tests/packaging/heic-native-build.test.mjs
git commit -m "build: bootstrap decode-only HEIC dependencies"
```

---

### Task 4: Decode bounded primary HEIF images

**Files:**
- Create: `crates/codec/src/heif_backend.rs`
- Create: `crates/codec/tests/heif_backend.rs`
- Create: `crates/codec/tests/fixtures/heif/iphone-8bit.heic`
- Create: `crates/codec/tests/fixtures/heif/iphone-10bit-grid.heic`
- Create: `crates/codec/tests/fixtures/heif/portrait-rotated.heic`
- Create: `crates/codec/tests/fixtures/heif/display-p3.heic`
- Create: `crates/codec/tests/fixtures/heif/truncated.heic`
- Create: `crates/codec/tests/fixtures/heif/fixtures.json`
- Create: `crates/codec/tests/fixtures/heif/README.md`
- Modify: `crates/codec/src/lib.rs`

**Interfaces:**
- The backend reads bytes from `&Path`, preserving non-UTF-8 Unix filenames.
- It selects `primary_image_handle()` and decodes interleaved RGB with transformations enabled.
- It copies each row using the returned stride; no code assumes a packed plane.

- [ ] **Step 1: Import redistribution-cleared fixtures and record provenance**

Add the five fixture files named above. `fixtures.json` records for each file: source URL or author, licence/grant, SHA-256, encoded bit depth, expected transformed width and height, expected corner colours, and expected capture timestamp where present. `README.md` explains that fixtures are committed test data and are not sourced from a user's private library.

Create `truncated.heic` by taking the licensed 8-bit fixture's first 256 bytes; record its derived origin and SHA-256 rather than treating it as independently licensed.

- [ ] **Step 2: Add failing primary-image, transform, colour, and limit tests**

Test these observable contracts:

```rust
let manifest = FixtureManifest::load();
assert_fixture("iphone-8bit.heic", manifest.expected("iphone-8bit.heic"));
assert_fixture("iphone-10bit-grid.heic", manifest.expected("iphone-10bit-grid.heic"));
assert_fixture("portrait-rotated.heic", manifest.expected("portrait-rotated.heic"));
assert_eq!(decoder_fingerprint(MediaKind::Heif).unwrap(),
           "libheif-1.23.4-libde265-1.1.1-sdr-v1");
```

The helper checks the exact dimensions and pixel tolerances recorded in `fixtures.json`. Verify that the portrait fixture's four decoded corners are in the transformed positions. For Display P3 and 10-bit inputs, verify output is `DynamicImage::ImageRgb8`, channels are within the recorded sRGB tolerances, and no auxiliary image is selected.

Add rejection tests for the truncated file, a sparse file over 512 MiB, a synthetic container declaring over 150 million pixels, and a container exceeding item/tile limits. Assert a typed limit/container error, never panic or process abort.

- [ ] **Step 3: Run the focused tests and verify they fail**

```bash
cargo test -p photo-codec --test heif_backend
```

Expected: FAIL because the HEIF backend still rejects `MediaKind::Heif`.

- [ ] **Step 4: Implement strict, bounded primary decode**

In `heif_backend.rs`:

1. Check `std::fs::metadata` and reject encoded inputs over `512 * 1024 * 1024` before allocation.
2. Read the file to memory and call `HeifContext::read_from_bytes`, not a UTF-8 path API.
3. Construct `SecurityLimits` with the values in Global Constraints and set them on the context before parsing.
4. Select only `primary_image_handle()`.
5. Use transformed handle dimensions and checked multiplication to enforce 150 million pixels and `usize` bounds.
6. Create `DecodingOptions`, enable strict decoding, leave transformations enabled, and request HDR-to-8-bit conversion.
7. Decode to `ColorSpace::Rgb(RgbChroma::Rgb)`.
8. Copy `height` rows of `width * 3` bytes from the interleaved plane using its stride into an owned `RgbImage`.
9. Return `DynamicImage::ImageRgb8` and convert all library errors into `CodecError`.

Call `libheif_rs::LibHeif::new()` once through `OnceLock`; do not register `image` hooks or plugins. HEIF ignores the catalogued EXIF orientation argument because the container transform is authoritative.

- [ ] **Step 5: Run decoder tests and sanitizing checks**

```bash
cargo test -p photo-codec --test heif_backend
cargo test -p photo-codec
cargo clippy -p photo-codec --all-targets --all-features -- -D warnings
```

Expected: all fixtures decode within their tolerances; malformed and oversized inputs return errors without a crash.

- [ ] **Step 6: Commit HEIF pixel decoding**

```bash
git add crates/codec
git commit -m "feat: decode primary HEIF still images"
```

---

### Task 5: Preserve HEIF EXIF and display geometry

**Files:**
- Modify: `crates/codec/src/heif_backend.rs`
- Modify: `crates/codec/src/lib.rs`
- Modify: `crates/codec/tests/heif_backend.rs`
- Modify: `crates/metadata/src/exif_reader.rs`
- Modify: `crates/metadata/src/probe.rs`
- Modify: `crates/metadata/tests/exif_reader.rs`
- Modify: `crates/indexer/tests/scanner.rs`

**Interfaces:**

```rust
pub fn embedded_exif_tiff(path: &Path, kind: MediaKind)
    -> Result<Option<Vec<u8>>, CodecError>;
```

- [ ] **Step 1: Add failing EXIF and no-double-rotation tests**

Assert that the HEIF fixture's EXIF block resolves through the existing metadata precedence pipeline to the recorded capture time. Verify both `II*\0` and `MM\0*` TIFF headers. Add malformed cases for a block shorter than four bytes, an overflowing big-endian offset, and an offset beyond the block.

In the scanner integration test, assert that `portrait-rotated.heic` stores transformed width/height and that generating a derivative does not rotate it a second time.

- [ ] **Step 2: Run the focused tests and verify they fail**

```bash
cargo test -p photo-codec --test heif_backend exif
cargo test -p photo-metadata --test exif_reader heif
cargo test -p photo-indexer --test scanner heif
```

Expected: FAIL because HEIF metadata is not routed to `kamadak-exif` and the shape still comes from the generic probe.

- [ ] **Step 3: Extract the TIFF payload safely**

Ask the primary handle for metadata blocks of type `Exif`. For the first block, read the leading four-byte big-endian TIFF offset, compute `4 + offset` with checked arithmetic, and return only the TIFF bytes after verifying a valid little- or big-endian TIFF header. Return `Ok(None)` when no EXIF block exists; return a typed metadata error when a declared block is malformed.

- [ ] **Step 4: Integrate with current metadata precedence**

Teach `EmbeddedExifReader` to request HEIF TIFF bytes from `photo-codec` and pass them to its existing `read_tiff` path. Keep sidecar precedence and all current non-HEIF behavior unchanged. Teach `MediaProbe::shape` to use `photo_codec::display_shape` for HEIF so the catalogue records transformed dimensions.

- [ ] **Step 5: Run metadata and scanner tests**

```bash
cargo test -p photo-codec --test heif_backend
cargo test -p photo-metadata
cargo test -p photo-indexer
```

Expected: PASS; captured-at data is present when embedded, shape matches decoded output, and orientation is applied once.

- [ ] **Step 6: Commit HEIF metadata support**

```bash
git add crates/codec crates/metadata crates/indexer
git commit -m "feat: read HEIF metadata and display geometry"
```

---

### Task 6: Make derivative keys media-aware

**Files:**
- Modify: `crates/cache/src/key.rs`
- Modify: `crates/cache/src/image_derivative.rs`
- Modify: `crates/cache/tests/cache_policy.rs`
- Modify: `crates/cache/tests/image_derivative.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- Add `media_kind: MediaKind` to `DerivativeSpec`.
- All spec constructors call `photo_codec::decoder_fingerprint(asset.media_kind)`.
- `validate_spec` rejects a decoder fingerprint that does not match the media kind.

- [ ] **Step 1: Add failing key-isolation tests**

Add tests proving:

```rust
assert_ne!(key_for(MediaKind::Jpeg, "image-0.25-v1"),
           key_for(MediaKind::Heif, "libheif-1.23.4-libde265-1.1.1-sdr-v1"));
assert!(generator.generate(&jpeg, &spec_with_heif_fingerprint).is_err());
```

Also preserve the current key for an existing JPEG golden vector after adding the field. Do this by hashing no new media-kind bytes for the four legacy image formats; for HEIF, hash a domain separator plus `heif`. This keeps existing cache entries reusable while preventing HEIF collisions.

- [ ] **Step 2: Run focused cache tests and verify they fail**

```bash
cargo test -p photo-cache key
cargo test -p photo-cache --test image_derivative fingerprint
```

Expected: FAIL because specs do not carry media kind and validation accepts only one global decoder constant.

- [ ] **Step 3: Update specs and remove duplicated decoder constants**

Add the field, update every literal, and remove `DECODER_VERSION`/`DERIVATIVE_DECODER_VERSION` copies from cache and app service. The generator calls `decode_display_image(source, spec.media_kind, spec.orientation)`. Cached-preview downscaling remains on `image::load_from_memory` because that input is Mote-generated JPEG.

- [ ] **Step 4: Verify real HEIF wall and screen derivatives**

Extend `crates/cache/tests/image_derivative.rs` to generate both derivative classes from `iphone-8bit.heic`. Assert JPEG content, 1024/4096 long-edge policy, representative colour tolerance, cache reuse, and unchanged source metadata and SHA-256 before/after generation.

- [ ] **Step 5: Run cache and app-service suites**

```bash
cargo test -p photo-cache
cargo test -p photo-app-service --all-features
```

Expected: PASS; existing formats retain their golden keys and HEIF derivatives reopen from cache.

- [ ] **Step 6: Commit media-aware derivatives**

```bash
git add crates/cache crates/app-service
git commit -m "feat: generate cached derivatives from HEIF"
```

---

### Task 7: Drive photo visibility and progress from compiled capability

**Files:**
- Modify: `crates/domain/src/media.rs`
- Modify: `crates/domain/tests/media.rs`
- Modify: `crates/catalog/src/wall_repo.rs`
- Modify: `crates/catalog/tests/wall_query.rs`
- Modify: `crates/indexer/src/scanner.rs`
- Modify: `crates/indexer/tests/scanner.rs`
- Modify: `crates/app-service/src/derivatives.rs`
- Modify: `crates/app-service/src/gallery.rs`
- Modify: `crates/app-service/tests/progressive_wall.rs`

**Interfaces:**
- `MediaKind::from_path` always recognizes `heic` and `heif`.
- `MediaKind::is_wall_viewable` includes `Heif` only under `cfg(feature = "heic")`.
- Catalog wall SQL uses the same compile-time media-kind set rather than `media_kind <> 'video'`.

- [ ] **Step 1: Add paired enabled/disabled behavior tests**

Under `#[cfg(feature = "heic")]`, assert HEIF is wall-viewable and appears in count, page, selected-record, scan-total, and progressive-derivative cases. Under `#[cfg(not(feature = "heic"))]`, assert the same HEIF rows remain retrievable from inventory but are absent from every photo count/page/progress/derivative result.

Keep AVIF, RAW, video, and unknown formats excluded in both modes. Add a regression case showing `.HEIC` and `.HEIF` are recognized case-insensitively even with HEIF disabled.

- [ ] **Step 2: Run both modes and verify the enabled case fails**

```bash
cargo test -p photo-domain
cargo test -p photo-catalog
cargo test -p photo-indexer
cargo test -p photo-app-service --all-features
cargo test -p photo-domain --no-default-features
cargo test -p photo-catalog --no-default-features
cargo test -p photo-indexer --no-default-features
cargo test -p photo-app-service --no-default-features
```

Expected: the new default-enabled HEIF visibility assertions fail before implementation; disabled assertions preserve current behavior.

- [ ] **Step 3: Replace scattered media filters with one capability set**

Update `MediaKind::is_wall_viewable`. In `wall_repo.rs`, use a cfg-selected constant predicate:

```rust
#[cfg(feature = "heic")]
const WALL_MEDIA_KINDS: &str = "('jpeg','png','tiff','heif','webp')";
#[cfg(not(feature = "heic"))]
const WALL_MEDIA_KINDS: &str = "('jpeg','png','tiff','webp')";
```

Use it in count, page, selected-record, and preview/coordinator queries. Do not change historical migrations to be feature-dependent; migrations define durable schema, while live queries and rebuild/reconciliation code apply compiled capability. Replace app-service checks of `!= Video` that schedule photo work with `is_wall_viewable()`.

- [ ] **Step 4: Run both modes again**

Run the eight commands from Step 2.

Expected: PASS in both modes; the disabled binary catalogs but never presents or processes HEIF.

- [ ] **Step 5: Commit capability-driven visibility**

```bash
git add crates/domain crates/catalog crates/indexer crates/app-service
git commit -m "feat: include HEIF in capable photo builds"
```

---

### Task 8: Expose one human-facing `--no-heic` build option

**Files:**
- Create: `scripts/cargo-feature-mode.sh`
- Modify: `scripts/desktop-build.sh`
- Modify: `scripts/flatpak.sh`
- Modify: `scripts/hosted-smoke.sh`
- Modify: `tests/build/build-lifecycle.test.mjs`
- Modify: `tests/build/heic-feature-contract.test.mjs`
- Modify: `tests/packaging/flatpak-cli.test.mjs`
- Modify: `tests/deployment/hosted-smoke-engine.test.mjs`
- Modify: `package.json`
- Modify: `README.md`

**Interfaces:**
- Accepted forms: no flag or exactly one `--no-heic` before passthrough arguments.
- Default Cargo args: none.
- Disabled Cargo args: `--no-default-features --features mote-defaults`.
- Flatpak and container builds receive an explicit `MOTE_HEIC=enabled|disabled` build value.

- [ ] **Step 1: Add failing CLI translation tests**

Extend fake-command fixtures to capture argv and assert:

```js
assert.deepEqual(defaultCargoFeatureArgs, []);
assert.deepEqual(noHeicCargoFeatureArgs,
	["--no-default-features", "--features", "mote-defaults"]);
```

Verify `scripts/desktop-build.sh native --no-heic`, `universal --no-heic`, `scripts/flatpak.sh package --no-heic`, and `scripts/hosted-smoke.sh --no-heic` consume the Mote flag rather than forwarding it to Tauri, Flatpak, or Docker as an unknown option. Assert duplicate or misspelled Mote flags exit 2 with usage text.

- [ ] **Step 2: Run focused script tests and verify they fail**

```bash
node --test tests/build/build-lifecycle.test.mjs --test-name-pattern=HEIC
node --test tests/packaging/flatpak-cli.test.mjs --test-name-pattern=HEIC
node --test tests/deployment/hosted-smoke-engine.test.mjs --test-name-pattern=HEIC
```

Expected: FAIL because current scripts forward or reject `--no-heic` inconsistently.

- [ ] **Step 3: Implement and source the shared translator**

`scripts/cargo-feature-mode.sh` exports only task-specific names:

```sh
MOTE_HEIC_MODE=enabled
MOTE_CARGO_FEATURE_ARGS=

mote_disable_heic() {
	MOTE_HEIC_MODE=disabled
	MOTE_CARGO_FEATURE_ARGS='--no-default-features --features mote-defaults'
}
```

Scripts must expand the two disabled options as separate argv entries without `eval`. Parse `--no-heic` at the Mote layer, preserve all unrelated caller arguments, and pass the mode explicitly into the Flatpak manifest/container build.

- [ ] **Step 4: Prevent default-feature drift**

Extend `heic-feature-contract.test.mjs` so `mote-defaults` must contain every entry-crate default feature except `heic` and itself. A future feature added to `default` without being forwarded by the no-HEIC bundle must fail the test.

- [ ] **Step 5: Document and verify both commands**

Document:

```bash
npm run desktop:build
npm run desktop:build -- --no-heic
npm run flatpak -- package
npm run flatpak -- package --no-heic
scripts/hosted-smoke.sh
scripts/hosted-smoke.sh --no-heic
```

Run:

```bash
npm run test:build
npm run test:flatpak
node --test tests/deployment/*.test.mjs
```

Expected: PASS and no generated artifact replaces the stable output when an injected build fails.

- [ ] **Step 6: Commit the build interface**

```bash
git add scripts tests package.json README.md
git commit -m "build: add default-on HEIC build mode"
```

---

### Task 9: Extend native dependency packaging to macOS universal and Windows CI

**Files:**
- Modify: `packaging/heic/versions.env`
- Modify: `packaging/heic/build-unix.sh`
- Create: `packaging/heic/build-windows.ps1`
- Create: `packaging/heic/verify-native-deps.sh`
- Create: `packaging/heic/verify-native-deps.ps1`
- Modify: `tests/packaging/heic-native-build.test.mjs`
- Modify: `scripts/desktop-build.sh`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/build-macos.yml`

**Interfaces:**
- Pinned native versions live only in `packaging/heic/versions.env` and agree with the Rust fingerprint.
- Native builds install shared libraries into a Mote-owned staging prefix.
- Verification fails on encoder symbols/tools, `x265`, missing architectures, absolute build paths, or missing runtime libraries.

- [ ] **Step 1: Add failing native-build contract tests**

Parse both scripts/workflows and assert version pins, SHA-256 fields, shared-library mode, libde265 enabled, x265 disabled, examples disabled, tests disabled, experimental API disabled, plugin loading disabled, and both default/no-HEIC CI lanes. Assert the disabled lane runs `cargo tree` plus binary dependency inspection.

- [ ] **Step 2: Run the contract test and verify it fails**

```bash
node --test tests/packaging/heic-native-build.test.mjs
```

Expected: FAIL because the pinned native build and verification scripts do not exist.

- [ ] **Step 3: Implement the pinned decode-only builds**

`versions.env` contains literal version, release URL, and SHA-256 values for libde265 1.1.1 and libheif 1.23.4. Both platform scripts download into a content-addressed cache, verify SHA-256 before extraction, and invoke CMake with shared libraries enabled and only the in-process libde265 decoder enabled. Explicitly disable x265, kvazaar, x264, AOM, dav1d, rav1e, SVT-AV1, vvdec/vvenc, OpenH264, FFmpeg, JPEG/JPEG-2000 codecs, examples, tests, experimental APIs, and runtime plugin discovery.

On Windows, build DLLs with MSVC and point `libheif-sys` at the staged prefix through its supported vcpkg/pkg-config discovery path; do not use the crate's embedded static source. On macOS universal builds, compile both `arm64` and `x86_64`, merge the two libraries with `lipo`, and rewrite install names and dependencies to `@rpath`.

- [ ] **Step 4: Bundle and verify macOS dylibs**

Teach `desktop-build.sh` to place universal `libheif` and `libde265` in `Mote.app/Contents/Frameworks`, add `@executable_path/../Frameworks` to the executable, and validate:

```bash
lipo -archs Mote.app/Contents/Frameworks/libheif.dylib
lipo -archs Mote.app/Contents/Frameworks/libde265.dylib
otool -L Mote.app/Contents/MacOS/mote
otool -L Mote.app/Contents/Frameworks/libheif.dylib
```

Expected: both dylibs contain `arm64 x86_64`; references are `@rpath`/`@loader_path`; no staging path or x265 reference appears. Disabled builds contain neither dylib and the executable has no HEIF dependency.

- [ ] **Step 5: Add normal and disabled CI lanes**

Linux, macOS, and Windows CI each build the pinned native libraries and run the real fixture test in default mode. A second lane builds `mote-defaults` only and verifies `libheif-rs`, `libheif`, and `libde265` are absent. Keep the existing all-feature lint/test coverage.

- [ ] **Step 6: Run available host checks**

```bash
node --test tests/packaging/heic-native-build.test.mjs
packaging/heic/build-unix.sh --platform macos --arch "$(uname -m)"
cargo test -p photo-codec --test heif_backend
packaging/heic/verify-native-deps.sh --prefix "$MOTE_HEIC_PREFIX"
```

Expected on macOS: native libraries build, fixture tests pass, and inspection finds no encoder or x265. Windows PowerShell build and DLL inspection run in CI.

- [ ] **Step 7: Commit desktop native packaging**

```bash
git add packaging/heic scripts/desktop-build.sh tests/packaging/heic-native-build.test.mjs .github/workflows/ci.yml .github/workflows/build-macos.yml
git commit -m "build: package decode-only HEIC libraries"
```

---

### Task 10: Package HEIF for Flatpak and the hosted container

**Files:**
- Modify: `packaging/flatpak/io.github.matt_jenner.mote.yml`
- Modify: `packaging/flatpak/generated/source-lock.json`
- Modify: `packaging/flatpak/README.md`
- Modify: `scripts/update-flatpak-sources.sh`
- Modify: `tests/packaging/flatpak-metadata.test.mjs`
- Modify: `tests/packaging/flatpak-cli.test.mjs`
- Modify: `Containerfile`
- Modify: `compose.yaml`
- Modify: `scripts/hosted-smoke.sh`
- Modify: `tests/deployment/hosted-smoke-engine.test.mjs`
- Modify: `.github/workflows/release-flatpak.yml`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Flatpak builds libde265, then libheif, then Mote from pinned offline sources.
- `MOTE_HEIC=disabled` skips the two native modules and builds `mote-defaults` only.
- The OCI final stage contains only the server, runtime shared libraries, loader metadata, and notices.

- [ ] **Step 1: Add failing package-content tests**

Assert the default Flatpak manifest has ordered `libde265` and `libheif` modules with verified archives and the decode-only CMake switches. Assert the Containerfile has separate native-build/runtime-copy stages and never installs `x265` or `heif-enc`. Extend hosted smoke to upload a real HEIC fixture, wait for scan completion, request wall and screen derivatives, and reopen the cached result.

Add disabled-mode assertions that inspect the Flatpak app and container image with `ldd`/`readelf` and filesystem search, failing if `libheif`, `libde265`, `heif-enc`, or x265 is present.

- [ ] **Step 2: Run contract tests and verify they fail**

```bash
npm run test:flatpak
node --test tests/deployment/hosted-smoke-engine.test.mjs --test-name-pattern=HEIC
```

Expected: FAIL because native modules, runtime copies, and HEIF smoke coverage are absent.

- [ ] **Step 3: Add offline Flatpak native sources and modules**

Extend `scripts/update-flatpak-sources.sh` and `source-lock.json` with the same verified release archives from `packaging/heic/versions.env`. Add shared-library modules before Mote. Install the LGPL files under `/app/share/licenses/io.github.matt_jenner.mote/`. The Mote module receives either default Cargo args or `--no-default-features --features mote-defaults` from `MOTE_HEIC`.

Use a manifest template or generated manifest selected by `scripts/flatpak.sh`; do not make offline `flatpak-builder` depend on shell environment expansion inside YAML. The disabled generated manifest omits both native modules entirely.

- [ ] **Step 4: Add a decode-only hosted dependency stage**

In `Containerfile`, build both native libraries from verified pinned archives for the target architecture. Compile `photo-server` normally when `MOTE_HEIC=enabled`; compile with the `mote-defaults` bundle when disabled. Copy only SONAME-linked `libheif`, `libde265`, their licence/source records, and required loader metadata into the Debian slim final image. Do not copy CMake files, headers, static archives, pkg-config metadata, examples, or encoders.

- [ ] **Step 5: Run available package verification**

```bash
npm run test:flatpak
npm run flatpak -- check
scripts/hosted-smoke.sh
scripts/hosted-smoke.sh --no-heic
```

Expected: enabled hosted smoke displays the fixture; disabled smoke indexes it but reports no photo/derivative work. Package inspections find only the two intended codec libraries in enabled outputs and neither in disabled outputs.

- [ ] **Step 6: Commit Linux and hosted packaging**

```bash
git add packaging/flatpak scripts/update-flatpak-sources.sh tests/packaging Containerfile compose.yaml scripts/hosted-smoke.sh tests/deployment .github/workflows
git commit -m "build: ship HEIC decoding in Linux packages"
```

---

### Task 11: Ship LGPL compliance material and audit the codec closure

**Files:**
- Create: `THIRD_PARTY_NOTICES.md`
- Create: `packaging/licenses/LGPL-3.0-or-later.txt`
- Create: `packaging/licenses/libheif.md`
- Create: `packaging/licenses/libde265.md`
- Create: `packaging/heic/README.md`
- Create: `tests/packaging/heic-compliance.test.mjs`
- Modify: `README.md`
- Modify: `packaging/flatpak/README.md`

**Interfaces:**
- Every enabled release package exposes notices, applicable licence text, exact source URLs/checksums, patches, build switches, and relink/replacement instructions.
- The audit is a packaging/compliance record, not a conclusion about HEVC patent coverage in a particular territory.

- [ ] **Step 1: Add failing compliance tests**

Test that notices name `libheif` 1.23.4 and `libde265` 1.1.1, identify their LGPL terms, link the exact verified source archives, record every Mote patch (or the literal word `none`), explain dynamic replacement, and state that x265/encoding is absent. Assert the source SHA-256 values match `versions.env` and the Flatpak lock.

- [ ] **Step 2: Run the test and verify it fails**

```bash
node --test tests/packaging/heic-compliance.test.mjs
```

Expected: FAIL because notices and source/rebuild instructions do not exist.

- [ ] **Step 3: Write and install the compliance files**

Include copyright notices and the complete applicable LGPL text. `packaging/heic/README.md` documents reproducible commands for rebuilding compatible shared libraries and replacing the bundled macOS/Windows/Linux libraries. State clearly that Mote performs decoding only, but do not claim that open-source licensing or non-commercial use automatically resolves third-party patent obligations.

Ensure Tasks 9 and 10 copy these files into macOS resources, Windows CI staging, Flatpak licences, and the hosted image.

- [ ] **Step 4: Audit linked and shipped files**

Run the platform inspection helpers against every locally available enabled package. Search file names, symbols, and dynamic dependencies for `x265`, `heif-enc`, `heif_encoder`, and unexpected codec plugins. The library itself may export generic encoder API symbols; verification therefore treats an exported declaration differently from a linked encoder implementation and fails specifically on encoder libraries/plugins/tools or build flags.

- [ ] **Step 5: Run compliance tests**

```bash
node --test tests/packaging/heic-compliance.test.mjs
npm run test:flatpak
```

Expected: PASS; pins, hashes, notices, and packaged files agree.

- [ ] **Step 6: Commit notices and rebuild instructions**

```bash
git add THIRD_PARTY_NOTICES.md packaging/licenses packaging/heic README.md packaging/flatpak/README.md tests/packaging/heic-compliance.test.mjs
git commit -m "docs: add HEIC dependency compliance material"
```

---

### Task 12: Run the complete enabled/disabled acceptance matrix

**Files:**
- Modify if required by evidence: `.github/workflows/ci.yml`
- Modify if required by evidence: `.github/workflows/build-macos.yml`
- Modify if required by evidence: `.github/workflows/release-flatpak.yml`
- Modify if required by evidence: `README.md`

**Interfaces:**
- Enabled mode must decode and present real fixtures.
- Disabled mode must compile without native codec discovery and preserve all other application behavior.

- [ ] **Step 1: Run formatting and static checks**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm run check
```

Expected: PASS with no warnings.

- [ ] **Step 2: Run the complete normal test suite**

```bash
cargo test --workspace --all-features
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --all-features
npm test
```

Expected: PASS, including real HEIF codec, scanner, cache, app-service, script, package, and hosted tests.

- [ ] **Step 3: Run the HEIF-disabled Rust matrix without native discovery**

Clear only codec discovery variables in the command environment and run:

```bash
cargo test --workspace --no-default-features
cargo test -p photo-server --no-default-features --features mote-defaults
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features --features mote-defaults
cargo tree -p photo-server -e features --no-default-features --features mote-defaults
```

Expected: PASS; the dependency tree contains no `libheif-rs` or `libheif-sys`, while JPEG/PNG/TIFF/WebP tests remain enabled.

- [ ] **Step 4: Build and inspect release artifacts**

On each available platform, build normal and `--no-heic` release variants. Verify enabled artifacts load only the pinned dynamic decoder pair and disabled artifacts load neither. For macOS universal, verify both slices. For Flatpak, verify installed runtime files. For OCI, run both AMD64 and ARM64 CI jobs and the hosted HEIF smoke test.

- [ ] **Step 5: Review failure isolation and source immutability evidence**

Run the corrupt-container integration case alongside healthy JPEG and HEIF files. Confirm the scan finishes, healthy assets display, the corrupt asset records a per-asset warning/terminal derivative failure, and all source hashes and filesystem metadata remain unchanged.

- [ ] **Step 6: Review the diff and unfinished-work scan**

```bash
git diff --check
git status --short
rg -n "TBD|TODO|FIXME|implement later" crates packaging scripts tests README.md THIRD_PARTY_NOTICES.md
```

Expected: no unfinished-work markers, no modifications below `dist/`, and only intended HEIF files remain changed.

- [ ] **Step 7: Commit any evidence-driven CI/documentation corrections**

If Steps 1-6 require corrections, commit only those verified changes:

```bash
git add .github/workflows README.md
git commit -m "test: complete HEIC release verification"
```

If no corrections were necessary, do not create an empty commit.
