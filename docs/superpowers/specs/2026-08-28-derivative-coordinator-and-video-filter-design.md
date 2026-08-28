# Derivative coordinator and temporary video filtering

Date: 2026-08-28

Status: Approved direction in conversation; awaiting review of this written specification

## Summary

Replace the scattered thumbnail and screen-preview gates with one bounded,
selection-aware derivative coordinator. The coordinator makes thumbnail-first
ordering a durable rule rather than an emergent property of several queues.

For every still image, the wall thumbnail is the first display derivative. A
tile becomes interactive only after that exact thumbnail has decoded and its
fade has completed. Background screen previews begin only after the selected
collection has finished its thumbnail phase. An explicitly opened photo may
request its larger preview after its own thumbnail is ready, without waiting
for the whole background pass.

Videos remain indexed internally but are completely invisible in this release.
They do not appear in walls, counts, filmstrips, navigation, search results, or
derivative queues. Playback and visible video representation return only with a
separately designed cross-platform player.

## Relationship to existing work

This specification refines the approved thumbnail-first loading and immersive
viewer designs. It preserves:

- read-only source media;
- SQLite metadata plus a managed derivative cache, never image blobs in SQLite;
- multiple roots and folder-group isolation;
- cached offline browsing;
- the host-neutral `PhotoService` boundary;
- a dark immersive viewer in every application appearance mode;
- continued background work while the viewer is open;
- viewport-first wall loading and bounded concurrency.

The experimental fixes currently on `codex/viewer-zoom-pan` are not accepted as
the final architecture. The implementation plan may retain proven tests or
small helpers, but it must replace race-prone orchestration rather than add more
independent gates.

## Product behaviour

### Thumbnail before preview

For an eligible still image, the observable order is:

1. publish geometry and representative colour;
2. generate or recover the 1024-pixel wall thumbnail;
3. publish its catalogue reference;
4. decode and fade that thumbnail into the wall;
5. enable the tile for opening;
6. generate or recover the 4096-pixel screen preview when its priority permits.

A screen-preview record never makes a colour-block tile interactive. A legacy
screen-preview-only entry is repaired into a thumbnail from the managed local
preview when valid. It falls back to the read-only source only when the local
preview cannot be used and the source is available.

### Priority order

The coordinator uses these logical priorities:

1. thumbnails in the current wall viewport;
2. the explicitly opened photo's screen preview, after that photo's thumbnail
   is ready;
3. thumbnails in the near-viewport rows;
4. remaining collection thumbnails during idle time;
5. background collection screen previews during idle time.

Existing work may be promoted when its priority rises. Promotion must not
duplicate cache writes or lose waiters. Interaction reduces background
concurrency but never cancels ready catalogue state.

### Collection phases

Background collection work is genuinely two phase:

- **Thumbnail phase:** visit the selected folder group in pages of at most 250.
  Every eligible still becomes thumbnail-ready or reaches a terminal thumbnail
  outcome for its current source fingerprint.
- **Preview phase:** only after the thumbnail phase completes, visit the group
  again in bounded pages and prepare screen previews for thumbnail-ready stills.

An explicitly opened photo is not background prefetch. It may request its
screen preview once its own thumbnail is ready, even while the collection's
thumbnail phase continues.

The coordinator pauses idle work between jobs and pages when interaction
becomes active. A new visible request preempts queued idle work. No collection
phase builds an in-memory list proportional to library size.

### Failures do not starve healthy photos

A corrupt, unsupported, unreadable, or unavailable asset records a bounded
terminal thumbnail outcome for the current source fingerprint and decoder
version. That asset remains a non-interactive fallback with the existing subtle
warning treatment. It does not prevent thumbnails or previews for other
assets.

A changed source fingerprint or decoder version makes the asset eligible for a
new attempt. Transient cache-wide failures remain retryable with bounded
backoff. Permanent asset failures are not retried on every coordinator wake.

### Tile paint lifecycle

The tile's image node and visual layers remain mounted while readiness changes.
The interactive overlay is separate from the image node, so enabling interaction
does not remount the decoded image.

Tile phases are:

- colour placeholder;
- current thumbnail decoding;
- current thumbnail fading;
- painted and interactive;
- unavailable or failed and inert.

Callbacks carry the current derivative key or render generation. A stale load,
decode, error, or transition callback cannot promote a replacement image. The
interactive overlay appears only after the current image's opacity transition
finishes. Under reduced motion, it appears immediately after successful decode
and paint scheduling without waiting for a transition event that may never
fire.

## Coordinator design

### One authority

`AppService` owns one derivative coordinator for the active selection. It is
the sole authority for:

- per-asset derivative state;
- priority promotion and bounded queues;
- collection-phase cursors;
- background cancellation generations;
- coalescing and waiter completion;
- admission to managed-cache and catalogue commit.

The image cache remains responsible for deterministic keys, decoding,
transforming, encoding, and contained writes. The catalogue remains responsible
for metadata and derivative records. Neither independently decides scheduling.

### Per-asset state

Coordinator state is ephemeral and derived from the catalogue when a selection
opens. Each asset tracks only the active fingerprint and bounded state:

- thumbnail: missing, queued, running, ready, terminal asset failure, or
  retryable cache failure;
- preview: missing, queued background, queued foreground, running background,
  running foreground, ready, or retryable failure;
- current priority and request generation;
- foreground and background waiters.

Videos never enter this state machine.

The recent-priority set is an ordered, de-duplicated window capped at 250 asset
IDs. Older assets return to ordinary collection order rather than retaining
near-viewport priority indefinitely.

### Foreground promotion

A foreground viewer request never silently joins background work whose
generation has been invalidated. If compatible work has not started, it is
promoted in place. If a running background encode is still current when the
foreground request acquires the coordinator boundary, that job is promoted:
its completion is admitted as foreground work and the foreground waiter joins
it. If the background job was already invalidated, its result is discarded and
the coordinator starts exactly one foreground replacement. Foreground waiters
receive the promoted or replacement result, never the stale job's failure.

Duplicate requests for the same immutable derivative share one admitted job
and one commit. Selection, asset fingerprint, derivative key, and request class
must all match before work is coalesced.

### Linearizable commit boundary

Screen encoding runs outside the coordinator lock because it has no cache,
catalogue, eviction, warning, or publication side effect. Commit admission is
serialized with background invalidation:

1. encode into memory or a contained staging value;
2. acquire the coordinator's commit/invalidation boundary;
3. revalidate selection, fingerprint, prerequisite thumbnail, request class,
   and generation;
4. if invalid, discard the encoded value with no side effect;
5. if valid, admit the commit and treat it as ordered before any later
   invalidation;
6. perform cache-budget planning, contained cache write, catalogue transaction,
   warning convergence, and publication as one admitted outcome.

A visible request uses the same boundary when invalidating background work.
Whichever operation enters first defines the order. This removes the gap where
a generation can change after validation but before eviction or catalogue
registration.

No coordinator lock is held while reading or decoding source media.

## Video filtering

Video metadata remains in SQLite so a future player does not require a fresh
library scan. User-facing photo queries exclude `media_kind = video` at the
catalogue projection boundary, before pagination and counts are calculated.

Consequently videos are absent from:

- photo-wall items and ready/progress totals;
- folder and smart-collection photo counts;
- filmstrip and previous/next sequences;
- keyword and filename search results shown by this release;
- thumbnail and screen-preview candidate queries;
- empty-state decisions.

There is no “videos hidden” message or count. A folder containing only videos
uses the ordinary `No photos found` state. The source files and their indexed
catalogue rows are not modified or deleted.

## Viewer details retained in this slice

Escape unwinds the immersive viewer in layers:

1. close the information drawer when open;
2. otherwise reset a zoomed photo to Fit and remain in the viewer;
3. otherwise return to the thumbnail wall.

A discrete Fit reset always produces a polite live-region event even when the
last announced label was already `Fit`. Continuous wheel, pinch, pan, and
navigator movement remain silent.

The immersive viewer remains intentionally dark under light, dark, and system
appearance settings. This is product behaviour, not a missed theme binding.

## Safety and performance

- Source files may be opened only for reads. Production code never writes,
  renames, moves, copies, or deletes source media.
- Legacy repair reads only a validated managed-cache path. Cache containment
  checks remain mandatory.
- Thumbnail cache entries remain durable; screen previews remain subject to
  whole-folder-group eviction policy.
- Collection traversal and recent-priority memory are bounded at 250 assets.
- Worker concurrency retains the existing active/idle limits. Visible work can
  preempt queued idle work without spawning an unbounded task per request.
- Selection and source-fingerprint checks occur before source work, before
  commit admission, and before publishing an outcome.
- Native source paths never cross the service boundary.

## Testing strategy

Implementation follows strict red-green-refactor and includes:

- coordinator unit tests for priority promotion, bounded recent history,
  coalescing, foreground adoption/restart, terminal failures, and phase cursors;
- multi-page integration tests proving no background screen preview begins
  before all eligible stills reach thumbnail-ready or terminal state;
- corrupt, unsupported, unavailable, and video fixtures proving one asset does
  not starve others;
- a deterministic encode/commit race proving invalidated work performs no
  eviction, cache write, catalogue change, warning clear, or publication;
- explicit viewer-request tests covering collision with cancelled background
  work;
- offline legacy repair tests proving the source is not read or mutated;
- browser tests that hold decode and fade completion, proving a tile remains
  inert until the current image is painted and stays inert on failure or stale
  callbacks;
- browser tests for drawer, zoom, and wall Escape unwinding plus discrete live
  announcements;
- catalogue/service tests proving videos are indexed but absent from items,
  counts, pagination, empty states, sequences, search projections, and
  derivative work;
- million-asset or existing large-catalogue benchmark coverage showing bounded
  page and queue behaviour.

## Acceptance criteria

- A clean collection never creates a screen preview for an asset before its
  wall thumbnail is ready.
- Background preview generation does not begin until the collection thumbnail
  phase is complete, excluding terminally unthumbnailable stills.
- A corrupt or unsupported still does not block healthy thumbnails or previews.
- A foreground viewer request cannot inherit cancellation from stale background
  work.
- Background invalidation and commit are linearizable; stale work has no cache,
  eviction, catalogue, warning, or publication side effect.
- Queue memory and collection traversal remain bounded for very large libraries.
- A tile cannot be opened before its current thumbnail is visibly painted.
- Legacy screen-only entries repair locally when possible.
- Videos remain indexed but are completely invisible and generate no
  derivatives.
- Escape closes the drawer, then resets zoom, then returns to the wall.
- The immersive viewer remains dark in every application appearance mode.
- Interface, WebKit, Rust, Clippy, formatting, desktop, benchmark, cache
  containment, offline, and source-read-only checks pass before native user
  acceptance.
