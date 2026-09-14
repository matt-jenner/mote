# HEIF decoding design

## Status

Approved through the design conversation on 14 September 2026. This document
defines decode-only HEIF and HEIC support for Mote's shared Rust engine and
current release packages.

## Purpose

Mote must display common iPhone HEIC photos on every platform where its shared
engine runs. The engine will decode the primary still image and continue
producing the JPEG wall thumbnails and screen previews already consumed by the
interface. Source files remain read-only.

HEIF support is enabled in normal builds. A Mote-facing `--no-heic` build flag
removes only HEIF support and its native libraries while retaining every other
normal feature. This escape hatch must reproduce the current behavior: HEIF
files remain catalogued but do not appear in photo views or photo progress
totals.

## Scope

This change includes:

- HEIC and HEIF primary-still decoding through `libheif` and decode-only
  `libde265`
- display-ready dimensions, HEIF geometric transformations, embedded capture
  metadata, and SDR sRGB output
- wall-thumbnail and screen-preview generation through the existing JPEG
  derivative pipeline
- default-on feature propagation through desktop and hosted binaries
- a `--no-heic` option on Mote's build and packaging entry points
- macOS universal, Windows CI, Flatpak, and hosted-container dependency work
- LGPL notices, source-version records, and rebuild instructions for the two
  dynamically linked libraries
- cross-platform tests using redistribution-cleared HEIC fixtures

This change does not include:

- HEIF or HEVC encoding
- the GPL `x265` encoder
- Live Photo video playback
- burst or sequence browsing
- depth-map display
- HDR or gain-map preservation in derivatives
- a Windows installer that does not otherwise exist yet
- a runtime preference for enabling or disabling HEIF

## Codec architecture

Add a focused `photo-codec` crate as the only code that converts a source image
into display-ready pixels. Existing JPEG, PNG, TIFF, and WebP decoding moves
behind this boundary without changing its output. The optional `heic` Cargo
feature adds `libheif-rs` and routes HEIC and HEIF sources through its direct
API backed by `libheif` and `libde265`.

The crate exposes operations needed by indexing and derivative generation:

- determine whether a media kind is viewable in this build
- probe display-ready primary-image dimensions
- decode a source into the pixel representation used by the existing image
  resize and JPEG encoder code
- extract the HEIF primary image's embedded EXIF payload for the existing
  metadata resolver
- report a media-specific decoder fingerprint for derivative keys

The direct `libheif` API is preferred over its global `image`-crate hooks. Mote
must control strict decoding, resource limits, primary-image selection, colour
conversion, metadata extraction, and transform behavior explicitly.

### Feature propagation

`heic` is a default feature of the desktop and server entry crates. Internal
crates disable dependency default features and forward `heic` deliberately so
that one top-level choice controls the whole graph. Direct Cargo builds include
HEIF by default.

Mote's build scripts accept `--no-heic`. With no flag, they build the normal
feature set including HEIF. With `--no-heic`, they remove only `heic` and keep
the rest of the normal feature set. The scripts hide Cargo's additive feature
mechanics and own the explicit list of normal non-HEIF features. Tests prevent
that list from drifting from the declared defaults as features are added.

The compiled media capability, not filename recognition alone, controls
viewability. Discovery always recognizes `.heic` and `.heif`, but inventory
counts, progressive scan totals, gallery queries, and derivative scheduling
include them only when the binary contains the HEIF decoder.

## Decode and metadata flow

For HEIC and HEIF sources, `photo-codec` performs these steps:

1. Open the container under strict parsing and fixed input, dimension, pixel,
   and memory limits.
2. Select the declared primary still image.
3. Read its transformed display dimensions.
4. Extract its embedded EXIF block and pass the TIFF payload to Mote's existing
   metadata parsing and precedence rules.
5. Decode the primary image while applying HEIF crop, rotation, and mirroring.
6. Convert wide-colour or HDR pixels to the existing 8-bit SDR sRGB working
   format.
7. Return the normalized pixels to the current representative-colour, resize,
   and JPEG encoding path.

HEIF container transformations are authoritative. The derivative path must not
apply an embedded EXIF orientation a second time. Catalogued width and height
describe the transformed image, so wall geometry matches the generated JPEG.

The first version ignores auxiliary depth images, gain maps, alternate images,
bursts, and sequences. It does not associate or play the separate MOV component
of an Apple Live Photo. These omissions do not prevent the primary still from
being displayed.

### Derivative caching

Generated output remains unchanged:

- a durable JPEG wall thumbnail with a 1024-pixel long edge
- a cache-budgeted JPEG screen preview with a 4096-pixel long edge, capped at
  source dimensions

HEIF uses a media-specific decoder fingerprint that includes the pinned
`libheif` and `libde265` versions plus Mote's conversion revision. Existing
formats retain their current fingerprint. Adding or upgrading HEIF support
therefore invalidates only HEIF-derived cache entries rather than rebuilding
JPEG, PNG, TIFF, and WebP derivatives.

## Failure behavior and safety

HEIF decode errors use the existing per-asset warning and terminal derivative
failure paths. A corrupt or unsupported HEIF file must not stop the folder scan
or block healthy photos.

The decoder rejects inputs that are malformed, encrypted, unsupported, or over
Mote's resource limits. Production builds enable strict decoding and retain
`libheif` security limits. They do not enable experimental libheif features or
allow an environment variable to disable the configured limits.

Source files are opened read-only. Decoded pixels and metadata stay in memory
until Mote atomically commits a JPEG derivative to its managed cache. No source
file is modified, renamed, replaced, or deleted.

A build without `heic` cannot enter the HEIF decode path. Such assets remain in
the catalogue for future compatible builds. The lightweight container shape
probe may still record their dimensions, but wall queries, derivative requests,
and photo progress accounting exclude them.

## Packaging

Mote keeps its own project licence. `libheif` and `libde265` remain separate
LGPL components and are dynamically linked in distributed builds. Release
artifacts include:

- a third-party notices file
- the applicable LGPL text
- exact upstream versions, source URLs, checksums, and Mote-applied patches
- instructions for obtaining the corresponding source and rebuilding or
  replacing the libraries

Mote does not compile or distribute `x265` or any other HEIF encoder.

### macOS

The app bundle contains universal `libheif` and `libde265` dynamic libraries
with both Apple Silicon and Intel slices. Their install names and transitive
references use app-relative paths. Packaging validation uses `lipo` and
`otool` to confirm both architectures and ensure no build-machine paths remain.

### Windows

Windows CI builds and tests against pinned dynamic `vcpkg` packages. This keeps
the shared engine ready for the planned Windows installer without adding that
installer to this change. A future installer must bundle the required DLLs and
not depend on a separately installed system codec.

### Linux and Flatpak

Ordinary Linux development discovers shared libraries through `pkg-config`.
The Flatpak manifest builds pinned `libde265` and `libheif` modules before Mote,
installs the runtime libraries and licence files, and excludes encoders and
unused codec plugins.

### Hosted container

The container builds pinned native libraries in a dependency stage and copies
only the required shared libraries, loader metadata, and notices into the final
image. Both AMD64 and ARM64 builds use the same upstream versions and options.
The final image contains no encoder.

### HEIF-disabled packages

Every Mote packaging entry point accepts `--no-heic`. That path neither builds,
links, nor bundles `libheif` or `libde265`. Package inspection tests fail if an
HEIF-disabled artifact still depends on either library.

## Verification

Commit redistribution-cleared fixtures covering:

- an 8-bit HEIC still
- a 10-bit tiled iPhone-style still
- portrait rotation or mirroring
- Display P3 input
- a malformed or truncated container

Unit tests cover primary-image selection, display dimensions, geometric
transformations, SDR sRGB conversion, EXIF extraction, decoder fingerprints,
resource limits, truncation, and unsupported HEIF features.

Integration tests run the progressive scanner and derivative coordinator over
HEIF fixtures. They verify photo counts, progress totals, capture metadata,
wall thumbnails, screen previews, cached reopening, per-asset failures, and
source read-only guarantees. The same catalogue cases run with HEIF disabled
and verify that files remain indexed but absent from photo results and totals.

CI performs both the normal and `--no-heic` builds on Linux, macOS, and Windows.
The normal build decodes real fixtures. Package checks inspect macOS library
slices and load paths, Flatpak runtime dependencies, and container libraries.
HEIF-disabled checks confirm that neither LGPL library is linked or shipped.

The hosted smoke test indexes a HEIC fixture and requests its JPEG derivative
through the existing HTTP API. Browser code continues to receive `image/jpeg`
and requires no HEIF support.

## Acceptance criteria

The change is complete when:

- normal builds include HEIF support without an extra flag
- common 8-bit and 10-bit tiled iPhone HEIC files appear in the wall and viewer
  with correct dimensions, orientation, colour, and capture date
- desktop and hosted variants use the same decode behavior
- all displayed HEIF content is derived from the primary still as SDR sRGB JPEG
- malformed, unsupported, or oversized HEIF files fail per asset without
  stopping a scan
- existing non-HEIF derivative keys and cached files remain valid
- source media remains read-only
- macOS, Windows CI, Flatpak, and hosted-container builds resolve the pinned
  decoder libraries reproducibly
- distributed packages contain the required LGPL notices and source details
- `--no-heic` retains other normal features, excludes HEIF from photo views and
  progress, and ships no HEIF decoder library
