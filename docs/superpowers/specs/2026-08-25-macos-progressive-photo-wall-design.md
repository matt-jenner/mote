# macOS progressive photo wall

Date: 2026-08-25

Status: Approved in conversation, awaiting review of this written specification

## Summary

Build checkpoint 2 of the macOS browse vertical slice: a vertically scrolling justified wall that begins showing real photos before an uncached folder finishes indexing. The wall reserves geometry from lightweight image probes, reveals complete rows in small batches, and replaces stable colour tiles with cached thumbnails without moving them.

The default settled order is capture date from oldest to newest. A visible direction control switches between oldest-first and newest-first using SQLite data. An uncached folder may use a deterministic provisional order while metadata is incomplete, then performs one smooth reorder after the first metadata pass settles.

The media pipeline creates both durable wall thumbnails and cache-limited screen previews. Reopening an indexed folder uses the local catalog and derivative cache before reconciling the source. Routine wall browsing must not wait for SMB access.

## Relationship to approved designs

This specification refines checkpoint 2, "See the photos," in the approved [macOS browse vertical slice](2026-08-24-macos-browse-vertical-slice-design.md). It preserves the [Photo Viewer system design](2026-08-21-photo-viewer-design.md), including these rules:

- Source media is read-only.
- SQLite stores metadata and derivative references, not image blobs.
- Generated derivatives live below the managed local cache root.
- React components depend only on the host-neutral `PhotoService` contract.
- The future hosted web portal can implement the same contract without changing wall components.
- Cached catalog entries and derivatives remain useful while a root is offline.
- Metadata precedence and stable library identity remain unchanged.

Where this document is more specific than the parent slice, it controls checkpoint 2. In particular, geometry may precede representative-colour extraction, provisional rows may appear before date metadata settles, and screen previews begin caching in this checkpoint even though the immersive viewer remains a later checkpoint.

## Product outcome

A user chooses a local or mounted folder and sees the first complete rows as soon as enough files have yielded dimensions. The window fills before background work expands to the rest of the folder. Rows fade in quickly, thumbnails crossfade into already-sized tiles, and scrolling remains available while indexing continues.

On a later launch, the app restores the wall from SQLite and serves ready thumbnails from the local derivative cache. The source scan reconciles in the background after the cached view is usable.

## Scope

### Included

- Lightweight shape probing for supported still-image files
- Geometry-ready catalog records committed in small transactions
- A viewport-first hybrid indexing and derivative pipeline
- Complete justified-row batching and vertical scrolling
- Stable neutral or representative-colour placeholders
- Batched wall-thumbnail requests and crossfades
- Durable wall-thumbnail caching
- Proactive screen-preview caching under the existing large-derivative policy
- Quiet, non-blocking indexing progress
- Capture-date sorting, oldest-first by default
- A visible oldest-first or newest-first direction control
- One settled reorder after an uncached folder's first metadata pass
- Cached restart and offline-safe catalog behavior
- Shared interface behavior at desktop, tablet, and phone widths

### Deferred

- Row virtualization, exact scroll anchoring, and sustained million-asset DOM bounds
- Continuous thumbnail-size adjustment
- Filename, rating, media-type, and keyword controls
- Multi-folder wall queries
- The immersive photo viewer, zoom, pan, filmstrip, and control fading
- Deep-zoom tiles and original-resolution delivery
- Full video playback

Checkpoint 3 adds the deferred browsing-at-speed behavior. Checkpoint 4 consumes the screen previews produced here in the immersive viewer.

## Selected approach

Use a hybrid two-lane pipeline.

The foreground lane discovers files, probes enough bytes to identify media shape, and produces wall thumbnails for the visible window. The background lane enriches metadata, generates remaining wall thumbnails, and fills the screen-preview cache. Both lanes use the existing interaction-aware scheduler, bounded concurrency, and transactional catalog writer.

This avoids two weaker designs:

- A strict whole-folder shape pass would reserve every tile before producing thumbnails. Large SMB folders could remain a wall of blank slots for too long.
- Completing every operation for one file before moving to the next would make the first few photos look finished, but populate the wall slowly and use network access poorly.

The foreground and background lanes are priorities over one shared work system, not separate scanners. Each asset keeps one stable catalog identity as its state advances.

## Progressive indexing

### Discovery

Discovery enumerates supported paths, associates sidecars, reads file signatures, applies the active folder policy, and commits cheap asset records in bounded batches. It does not wait for the full tree walk before emitting work.

Discovery order is not presented as final date order. The scan assigns each newly discovered asset a monotonically increasing provisional order and persists it with the asset. Enrichment preserves that value. The provisional wall queries this value with stable asset ID as the tie-breaker, so later discoveries append instead of inserting above rows already shown.

### Shape probe

The shape probe reads only enough of the file or container to obtain width, height, orientation, and media type. Many formats expose this near the header, but the implementation must use a bounded format-aware probe rather than assume a fixed byte count. RAW containers may use their cheapest suitable embedded preview metadata.

As soon as an asset has usable display dimensions, the indexer writes its geometry-ready state. It does not wait for full metadata extraction, representative-colour calculation, thumbnail encoding, or a complete folder pass.

If enough geometry-ready assets form a complete justified row, the service publishes that row's records in the next interface batch. The scheduler first covers the visible window, then a small scroll-ahead region, then the remaining active folder.

### Metadata enrichment

The enrichment pass reads embedded EXIF, IPTC, and XMP plus adjacent sidecars. It preserves the approved field-specific precedence. Capture date therefore prefers original-capture metadata, then other capture and container dates, filesystem birth time, and finally modified time.

Enrichment updates the existing asset row and never creates a second identity. Arriving metadata does not continuously move visible tiles. When the first metadata pass for the active folder completes, the backend emits one settled notification. The interface re-queries the selected date direction and performs one restrained wall transition.

### Derivative work

Derivative generation follows this priority:

1. Wall thumbnails for the visible rows
2. Screen previews for an explicitly requested asset once a viewer exists
3. Wall thumbnails for the scroll-ahead rows
4. Remaining wall thumbnails for the active folder
5. Screen previews for visible and recent rows after foreground indexing work is satisfied
6. Remaining screen previews while the app is idle and cache policy allows

Pointer, keyboard, touch, scroll, resize, and later viewer activity lower background concurrency. Work that has moved far outside the scroll-ahead region may be deprioritized. Completed durable derivatives remain cached.

## Derivative tiers and cache policy

### Wall thumbnail

The wall thumbnail is a compact, colour-managed derivative with a 1024-pixel long edge. This single checkpoint-2 size remains crisp at the fixed target row height on high-density displays. Checkpoint 3 may add bounded smaller size classes when the continuous sizing control makes them useful.

Wall thumbnails use the existing durable tier. They are stored as files below the cache root and referenced by immutable derivative keys in SQLite.

### Screen preview

The screen preview is a colour-managed derivative with a 4096-pixel long edge, capped at the source dimensions. It is suitable for full-window display and moderate zoom without another source read. RAW processing may first use a suitable embedded preview, then replace it with a higher-quality generated preview when available.

Screen previews use the configurable large-derivative budget. The existing default remains automatic at 10 percent of the cache volume, capped at 100 GiB, unless the user sets an explicit limit.

Large-derivative eviction operates on physical folder groups. Opening or viewing one asset refreshes its whole group's last-viewed time. When space is required, the cache removes the oldest eligible group first. It never evicts the active group or a group with a write in progress.

### Storage and safety

SQLite stores derivative kind, immutable key, relative cache path, byte count, and readiness. It never stores complete image bytes. A derivative key includes the source signature, resolved orientation, decoder version, colour conversion, and target size.

Writers use temporary cache files and atomic promotion. Startup reconciliation removes abandoned partial files and repairs stale catalog references. Cache cleanup never writes, renames, or deletes source media.

## Wall query and sorting

The wall reads SQLite only. Scrolling, sorting, resizing, and cached reopening never enumerate source files.

Each wall record contains:

- Stable asset ID
- Display width and height after orientation
- Media kind
- Persisted provisional order
- Provisional or settled date state
- Representative colour when ready
- Availability and warning state
- Wall-thumbnail derivative reference when ready
- Screen-preview readiness without exposing a source path

The default settled sort is resolved capture date ascending, oldest first. A visible direction control switches to descending, newest first. Stable relative path and asset ID break equal-date ties. Changing direction resets the scroll container to the top and runs a new SQLite query. It does not rescan the source.

During the first uncached scan, the deterministic provisional wall remains stable while new complete rows append. Metadata updates do not individually reorder it. The single settled reorder applies the active direction after enrichment completes.

## Justified wall behavior

A pure TypeScript layout function groups ordered assets into justified rows for the available content width and fixed checkpoint-2 target row height. Completed rows fill the width while preserving aspect ratios. The final incomplete row stays left aligned and does not stretch excessively. While discovery is active, the interface holds an incomplete row until more geometry arrives. When discovery completes, it reveals the remaining incomplete row, which also lets a small folder display even when it cannot fill one justified row.

The interface reveals only complete rows during progressive indexing. New row batches fade in over 120 to 180 milliseconds. Reduced-motion settings disable the fade.

Every tile receives its final row geometry before it renders. Its visual layers progress without changing that geometry:

1. A restrained theme-neutral fill
2. A representative colour when cheaply available
3. The cached wall thumbnail

The thumbnail crossfades over the existing fill. Image decode completion must not change row height, tile width, scroll height for an already completed row, or neighboring tile positions.

Checkpoint 2 supports ordinary vertical scrolling and requests another bounded catalog batch near the end of loaded content. Rows may accumulate for the duration of this checkpoint. Checkpoint 3 adds row virtualization, bounded mounted rows, continuous sizing, and exact anchor preservation for very large collections.

## Components and interfaces

### Rust application service

The host-neutral application service adds operations equivalent to:

- Querying a bounded wall batch with sort direction
- Starting or resuming active-source indexing
- Requesting batched derivatives for a visible range
- Reporting derivative and screen-preview readiness
- Subscribing to ordered progress, catalog-batch, derivative-ready, and metadata-settled updates

DTOs contain stable IDs and cache references. They do not expose unrestricted absolute source paths.

### Desktop adapter

The Tauri adapter maps the shared contract to typed commands and an ordered channel. It is the only interface code that imports Tauri APIs. It resolves derivative references through the approved read-only `photo-derivative:` protocol, which serves cache files only.

### React interface

The interface adds focused units with one responsibility each:

- A wall controller that queries batches, merges by stable asset ID, tracks provisional or settled state, and reacts to ordered notifications
- A pure justified-row layout function
- A wall toolbar with the date-direction control and quiet progress
- A vertical wall that renders complete rows
- A photo tile that owns placeholder and thumbnail refinement without owning geometry

Shared components consume `PhotoService`. They do not call `fetch`, import Tauri, inspect hostnames, or construct transport-specific URLs.

## Data flow

Opening or restoring a folder follows this sequence:

1. The service returns cached catalog rows immediately when they exist.
2. The interface lays out complete rows from catalog dimensions and requests visible wall derivatives in one batch.
3. The service starts or resumes discovery, shape probing, enrichment, and derivative scheduling.
4. Geometry-ready catalog batches arrive through the ordered update stream.
5. The wall controller merges new assets by ID, requests the next bounded query range, and reveals newly complete rows.
6. Derivative-ready updates replace immutable thumbnail references inside existing tiles.
7. Metadata completion emits one settled update and the interface re-queries the selected date direction.
8. Background reconciliation and remaining screen-preview generation continue only after foreground work is satisfied.

Changing sort direction skips source work. It resets the wall to the top, clears loaded display batches, and queries SQLite in the new direction.

## Failure and offline behavior

One damaged or unreadable asset never stops the scan.

- If geometry is known but derivative generation fails, the stable fill remains and the catalog records a retryable warning.
- If geometry cannot be read, the asset uses a stable 4:3 fallback aspect ratio and the approved subdued question-mark treatment. It keeps that geometry for the current scan. A later successful retry may replace it during an explicit refresh transition.
- If the source becomes unavailable, cached thumbnails and previews remain viewable. Uncached assets retain catalog positions and unavailable treatment.
- The desktop recovery flow may later offer Try Again and Locate Folder. The hosted web interface never offers local folder selection for a server-mounted source.
- A full or unwritable cache pauses new derivatives while catalog browsing continues and reports one quiet source-level warning.
- Interrupted scans resume from committed batches. Incomplete cache writes are discarded safely during reconciliation.

## Performance requirements

The existing system targets remain in force:

- First visible cached thumbnails within 200 milliseconds after a view change
- Cached screen-preview delivery within 300 milliseconds
- Wall scrolling and transitions targeting 60 frames per second
- No geometry change when image quality improves
- Interaction immediately reducing background concurrency

Checkpoint-specific requirements are:

- An uncached folder emits its first complete row without waiting for full discovery or metadata enrichment.
- The scheduler fills the visible window before expanding to the rest of the folder.
- Shape, catalog, and derivative updates cross the host boundary in batches rather than one command per asset.
- The interface performs no synchronous source read or image decode.
- Cached launch paints catalog geometry and ready thumbnails before source reconciliation becomes visible.
- Individual metadata or thumbnail arrivals do not trigger wall reordering.

Checkpoint 2 proves progressive value and stable geometry. Checkpoint 3 provides strict bounds for mounted rows and sustained navigation through million-asset results.

## Testing

### Rust

- Shape probes produce oriented dimensions without requiring full image decode where the format permits.
- Geometry-ready records commit before metadata completion.
- The catalog preserves one asset identity across discovery, shape, metadata, and derivative states.
- Visible wall thumbnails outrank scroll-ahead and background work.
- Wall and screen-preview derivative keys differ by kind and target size.
- Wall thumbnails are durable while screen previews follow the large-derivative policy.
- Interrupted scan batches resume without duplicate assets.
- Offline reconciliation preserves catalog and derivative records.
- Production source operations remain read-only.

### TypeScript and browser rendering

- Justified-row layout is deterministic and preserves aspect ratios.
- Only complete progressive rows are revealed.
- Batch merging is idempotent by asset ID.
- Thumbnail readiness never changes measured tile geometry.
- Oldest-first is the default and direction changes reset to the top.
- A metadata-settled update causes one reorder rather than per-asset movement.
- A deliberately slow in-memory service proves that rows appear before scan completion.
- Reduced-motion mode disables row and thumbnail fades.
- Desktop, tablet, and phone-width renders retain the same service contract and usable vertical wall.

### macOS demonstration

The checkpoint demonstration uses controlled real image fixtures in the packaged or development Tauri application. It shows:

1. A clean profile opening an uncached folder
2. Complete rows appearing before indexing finishes
3. The visible window receiving thumbnails before later rows
4. Vertical scrolling while progress continues
5. Oldest-first and newest-first queries resetting to the top
6. Thumbnail refinement without tile movement
7. Restarting into the cached wall before source reconciliation
8. Screen-preview derivatives present in the managed cache for later viewer use

The demonstration records the exact commit, fresh verification results, screenshots, and a short motion capture for progressive row and thumbnail behavior.

## Completion criteria

Checkpoint 2 is complete when:

- The running macOS app shows real photos in a vertically scrolling justified wall.
- An uncached folder displays complete provisional rows before its full scan finishes.
- The visible window receives work before the rest of the folder.
- Tiles retain exact geometry while fills and thumbnails refine.
- The final settled order defaults to oldest first and can switch to newest first without source access.
- Reopening the same folder renders from cached catalog and wall thumbnails.
- Wall thumbnails and screen previews are stored as safe cache files, not SQLite blobs.
- Slow, missing, unreadable, or corrupt source files do not block healthy rows.
- Shared React components contain no direct Tauri or HTTP dependency.
- Relevant Rust, TypeScript, browser, integration, formatting, lint, and macOS build checks pass.
- No production operation writes, renames, or deletes source media.
