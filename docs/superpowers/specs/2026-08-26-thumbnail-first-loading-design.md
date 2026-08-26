# Thumbnail-first loading remediation design

**Status:** Approved in conversation on 2026-08-26.

## Problem

The first native exploratory run showed coloured geometry without reliable thumbnail refinement, no useful loading progress, multi-minute first-cache work for a small LAN collection, and thumbnail loss or permanent stalls after changing sort direction.

The causes are independent but reinforce one another:

- initial derivative requests depend entirely on an `IntersectionObserver` callback;
- the interface treats a requested derivative as permanently complete even when delivery fails or is missed;
- sorting clears populated items before the replacement page arrives;
- the development desktop build runs image dependencies without optimisation;
- 4096-pixel screen previews begin too soon and compete with contact-sheet work;
- scan progress crosses the native boundary but the interface ignores it, and derivative progress is not represented.

## Required behaviour

### Loading passes

The contact sheet loads in ordered passes:

1. Request missing wall thumbnails for rows intersecting the current viewport with `visible` priority as soon as row geometry exists. This must work even if `IntersectionObserver` never calls back.
2. Request missing wall thumbnails for the next two rows with `nearViewport` priority.
3. Request missing wall thumbnails for the rest of the current catalog page in bounded `nearViewport` batches during browser idle time. Scrolling promotes newly visible assets to `visible` priority.
4. Generate 4096-pixel screen previews only after indexing has settled, all queued wall-thumbnail work has drained, and interaction is idle. A later visible thumbnail request must take precedence over screen-preview work.

No pass waits for a source image to paint before the next contact-sheet pass can be queued. The backend priority queue, bounded batches, and two-worker ceiling provide back-pressure.

### Request lifecycle

The interface tracks derivative requests as queued or in flight, not permanently requested. A `wallThumbnail` ready event clears pending state and marks the asset ready through catalog state. Failed or warning events make the asset eligible for a later request. Changing sort direction begins a new request epoch, so assets still missing thumbnails can be requested again.

### Sorting

Changing oldest/newest direction keeps the current wall and its thumbnail references visible while the replacement SQLite page is loading. When the first replacement page arrives, the interface swaps order atomically and merges existing derivative references by asset ID. Scroll returns to the top once per direction change. Stale query results remain fenced.

### Progress feedback

The toolbar contains one subtle, accessible progress track and concise status text. It reports the most useful active stage:

- before shaped assets exist: indexing progress from `ScanProgressDto`;
- while known wall thumbnails are missing: `Preparing previews · ready of known`;
- after contact thumbnails are ready but screen previews are still being prepared: `Photos ready · preparing larger previews · ready of known`;
- when all known work is ready: the existing ready count.

The track is determinate when a meaningful total is known and indeterminate otherwise. The wall remains interactive. Motion uses the existing fade token and becomes static under `prefers-reduced-motion`.

### Performance and safety

- Optimise dependencies in the standalone Tauri development profile while leaving the desktop crate itself debuggable.
- Keep wall thumbnails at the approved 1024-pixel long edge and screen previews at 4096 pixels. Do not trade visible quality for the first fix before measurement.
- Add stage timings around source read/decode, transform/encode, cache write, catalog commit, and publication using tracing or benchmark-only measurement without exposing native paths.
- Source media stays read-only. SQLite and the managed cache remain the only writable locations.
- The web interface keeps using the host-neutral `PhotoService`; no React component imports Tauri APIs.

## Acceptance criteria

- A populated first viewport requests thumbnails without an observer callback.
- Visible requests precede next-row requests, which precede the remaining-page batches.
- Sorting never turns loaded thumbnails back into coloured blocks while the replacement page is pending.
- Sorting or a retry can request an asset again when no ready thumbnail exists.
- Progress distinguishes indexing, contact-thumbnail preparation, and background larger-preview preparation.
- Concurrent wall-thumbnail waves finish before screen-preview prefetch starts.
- The controlled derivative workload is materially faster under the desktop development profile than the previous unoptimised development build.
- Existing unit, WebKit, Rust, Clippy, formatting, bundle, cache containment, offline-catalog, and source-read-only checks remain green.
