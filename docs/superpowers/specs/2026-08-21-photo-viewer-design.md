# Photo viewer system design

Date: 2026-08-21

Status: Approved in conversation, awaiting review of this written specification

## Summary

Build a read-only photo viewer for macOS, Windows, Linux, and a trusted-LAN web portal. The desktop application must work without a server and read local or mounted folders. The web deployment runs in Docker and reads one or more SMB-backed mounts.

The product uses a shared React and TypeScript interface, a shared Rust application core, Tauri desktop shells, and a small Rust HTTP server for the web portal. Every installation keeps its own SQLite catalog and derivative cache on local writable storage. Source media remains read-only.

The intended character is calm, precise, and restrained. The photo wall takes visual priority. Search, filters, metadata, and recovery controls appear when needed rather than occupying permanent screen space.

## Goals

- Browse a catalog of up to one million media items without total catalog size affecting interaction cost.
- Combine several selected folders into one deduplicated view.
- Sort by capture date, filename, or rating.
- Include or exclude media types such as RAW, JPEG, HEIC, and video.
- Search embedded and sidecar keywords.
- Create saved smart collections from keyword, source, date, rating, and media-type rules.
- Render a vertical justified photo wall with an adjustable target row height.
- Open photos in an immersive viewer with progressive resolution, zoom, and pan.
- Keep desktop and web behavior consistent while respecting platform capabilities.
- Index and refine configured libraries during idle time without blocking interaction.
- Remain useful when a source is temporarily unavailable.

## Non-goals for the first release

- Editing ratings, keywords, metadata, sidecars, or source files
- Moving, renaming, deleting, importing, or exporting source media
- RAW development or colour correction tools
- Public-internet deployment, accounts, or folder permissions
- Catalog synchronization between installations
- Face recognition, object recognition, or other generated tags
- Manual albums containing copied or explicitly ordered asset references
- Duplicate-photo detection across unrelated source files
- Automatic transcoding of unsupported video codecs
- Full parity with a digital asset manager such as Lightroom

## Success criteria

The performance target is an indexed catalog containing one million assets. Measurements use a local SQLite catalog and represent product goals, not guarantees for first access to a slow SMB share.

- Cached wall queries respond immediately enough to preserve continuous scrolling and resizing.
- The first visible cached thumbnails appear within 200 ms after a view change.
- A cached screen preview opens within 300 ms.
- Filename and keyword search returns its first page within 500 ms.
- Wall scrolling and viewer transitions target 60 frames per second on supported reference hardware.
- Background indexing lowers its concurrency as soon as the user interacts.
- An uncached folder begins emitting stable, correctly shaped wall items before its full metadata scan completes.
- Loading a better preview never changes the tile geometry, zoom position, or pan position.

The timings will be validated with profiling builds and adjusted after usability testing. Any optimization that produces a meaningful measured improvement without breaking isolation or correctness belongs in scope.

## Terminology

### Installation

One desktop app profile or one web-server deployment. Installations do not share catalogs or caches.

### Library

A configured filesystem root. A library has a stable ID, a current absolute path, folder policies, scan state, and availability state.

### Recent source

An arbitrary folder opened from the desktop without adding it as a configured library. It receives a stable local root ID, uses the recursive fallback policy, and retains its catalog and cache between sessions. It appears under Recents and can be promoted to a library. The web portal does not create recent sources through the browser.

### Folder

A physical folder below a library root. A folder view may recursively include descendants and may apply temporary filters.

### Folder group

A physical cache and browsing unit detected by a library's folder policy. In the example layout, an activity folder containing `RAW`, `Videos`, and `Processed` is one folder group. Folder groups are not smart collections.

### Smart collection

A named, saved query. It stores rules, not assets or derivative ownership. It may span selected libraries and updates as their catalogs change.

### Asset

One indexed media file. Its identity is the library ID plus normalized relative path. A file signature records size, modified time, and other cheap identity information for invalidation.

### Derivative

A generated thumbnail, screen preview, deep-zoom tile, poster frame, RAW decode, or future video proxy stored outside SQLite in the local cache.

## Architecture

### Shared interface

React and TypeScript implement the photo wall, viewer, search, filters, smart-collection editor, library navigation, settings, and status surfaces. The interface depends on a typed application-service contract rather than Tauri or HTTP directly.

Two transport adapters implement that contract:

- Desktop uses typed Tauri commands and event streams. It opens no local HTTP port.
- Web uses same-origin typed HTTP endpoints and a progress-event stream.

The adapters must return the same domain response types and error categories. Platform-only capabilities, such as choosing a replacement folder, are exposed explicitly and never faked on the web.

### Shared Rust core

The Rust core contains independent components with narrow interfaces:

- Library service manages roots, folder policies, availability, and relinking.
- Catalog repository owns SQLite schema, migrations, transactions, and cursor queries.
- Index scheduler owns discovery, priorities, concurrency, retries, and cancellation.
- Metadata adapters read file headers, EXIF, IPTC, XMP, adjacent sidecars, and video containers.
- Media pipeline extracts embedded previews and generates display derivatives.
- Cache manager accounts for derivative size, group recency, eviction, and reconciliation.
- Query service implements folder unions, sorting, filtering, search, and smart collections.
- Progress service publishes coarse status without coupling the interface to individual jobs.

CPU-heavy or crash-prone decoders run behind process or worker boundaries where the chosen libraries permit it. A bad media file must not bring down the application service.

### Desktop host

Tauri packages the shared interface and Rust core for macOS, Windows, and Linux. The core owns database connections and global job state. The WebView remains unprivileged and receives only the commands it needs.

Desktop source paths may be local folders or network folders mounted by the operating system. Indexing continues while the app is open. Idle work yields when the user scrolls, searches, opens an image, or manipulates zoom.

### Web host

The Docker image contains the Rust server, shared frontend bundle, and required media tools. It has two storage classes:

- `/photos` contains one or more read-only library mounts.
- `/data` contains the SQLite catalog, derivatives, settings, smart collections, and migration backups.

Compose supports a host-mounted SMB path as the recommended setup. It may also define a named CIFS volume where the Docker host supports it. Credentials remain outside the image and repository. The container does not require privileged filesystem access.

The first release targets trusted LAN or VPN use. The server warns when configured to bind broadly. It has no public-internet security profile.

### Local storage rule

SQLite and derivatives always live on local writable storage. They never live on the SMB share. This permits SQLite write-ahead logging and prevents catalog traffic from competing with source reads over the network.

## Catalog model

The catalog stores normalized records for libraries, folders, folder groups, assets, sidecars, metadata values and provenance, derivatives, warnings, scan generations, smart collections, and settings.

SQLite FTS5 indexes normalized filenames and keywords. Structured columns and indexes serve library, folder, date, rating, type, and availability filters. Stable cursor pagination uses the requested sort key plus asset ID as a deterministic tie-breaker.

The database stores metadata and derivative references, not thumbnail blobs. Derivative files use content-addressed paths below the cache root.

A single catalog spans all roots in one installation. Cross-library smart collections therefore require one local query. An unavailable root leaves its assets searchable.

Overlapping or nested roots are detected during setup. The app warns before adding them and must not index the same canonical file twice. Relinking changes a library's absolute path while preserving its stable ID and every smart-collection reference.

## Folder policies

Each library owns an ordered list of folder policies. Policies use guided settings and glob-style patterns rather than a general scripting language.

For each candidate folder group:

1. Evaluate structural policies in order.
2. Apply the first matching policy.
3. If no policy matches, recursively include supported media below the folder.
4. Apply library-wide exclusion patterns in either case.

A policy may:

- Define the depth at which folders become folder groups.
- Prefer a named subtree when it exists, such as `Processed/**`.
- Flatten descendants of the preferred subtree into the parent folder-group view.
- Hide storage-oriented subfolders from navigation without losing their original paths.
- Exclude sibling subtrees such as `RAW` from the default view while retaining them in the catalog when configured.
- Derive fallback metadata from a matched path, such as rating `3` from a `3 Stars` folder. Explicit sidecar or embedded metadata wins, and the catalog records the path policy as provenance.

For the example structure, the recommended preset treats each activity folder as one folder group. If `Processed` exists, its descendants supply the default images and the star folders do not become separate navigation levels. If the structure does not match, the safe fallback includes all supported media recursively.

Desktop ad hoc folders become recent sources and use the recursive fallback unless the user promotes one to a library and applies a policy. Several recent sources and library folders may participate in the same temporary view. Library policies can be copied to another library but do not become global automatically.

## Indexing and change detection

Indexing proceeds in stages so first use remains responsive.

### Stage 1: discovery

Enumerate paths in batches, apply folder policies, identify sidecars, stat supported files, and write cheap asset records. Discovery emits work incrementally rather than waiting for a complete tree walk.

### Stage 2: wall shape

Read dimensions, orientation, media type, the cheapest useful embedded preview, and a representative colour. This gives the interface enough information to reserve each tile and display a stable wall.

### Stage 3: metadata

Read embedded EXIF, IPTC, and XMP plus adjacent XMP sidecars. Normalize values and retain source provenance.

Metadata resolution rules are field-specific:

- Capture date prefers original-capture metadata, then other capture metadata, container creation metadata, filesystem birth time, and modified time.
- Rating prefers an explicitly set adjacent XMP value, then embedded XMP, then format-native rating fields.
- Keywords are the normalized union of nonempty sidecar XMP, embedded XMP, and IPTC keywords. Case-insensitive duplicates collapse while the first useful display spelling is retained.
- Hierarchical XMP keywords preserve their hierarchy as well as searchable leaf values.

### Stage 4: derivatives

Generate durable wall thumbnails, screen previews, RAW refinements, poster frames, and deep-zoom tiles according to queue priority.

### Work priority

One global scheduler orders jobs by user value:

1. Currently visible assets
2. Assets near the viewport
3. Remaining assets in the open folder or smart collection
4. Configured libraries during idle time

Interactive work may preempt queued background work. Running jobs receive bounded concurrency and cooperative cancellation where safe.

### Reconciliation

Native filesystem events provide low-latency hints but are not treated as authoritative, especially on SMB. Periodic reconciliation and startup scans detect missed events and changes made while the app was closed.

The catalog uses file signatures and sidecar state to decide whether to rebuild metadata or derivatives. A temporarily unavailable root never triggers mass deletion. Missing assets remain cataloged and retryable until the user removes the library or a confirmed online reconciliation updates their state.

## Browse and query behavior

The compact navigation rail expands to show libraries, recent sources, physical folders, and smart collections. Users may select several folders. The result is a deduplicated union of recursively included assets.

The main wall uses vertical justified rows. Every row fills the available width while preserving aspect ratios. The page scrolls vertically. A drag control changes the target row height in real time and keeps the first visible asset anchored.

The interface virtualizes rows around the viewport and requests stable catalog pages. It never enumerates the SMB tree during scrolling, sorting, filtering, or searching.

Search covers normalized filenames and keywords. The curated filter set contains:

- Capture-date range
- Rating range or threshold
- Included and excluded media types
- Selected libraries and folders
- Keyword rules

Sort options are capture date, natural filename order, and rating. Each supports ascending or descending order. Active filters appear as removable chips; inactive filter controls stay behind one filter action.

Physical folder views may apply temporary keyword filters. Temporary combinations remain unsaved views and the previous session's active view may be restored on the same installation.

## Smart collections

A smart collection is a named query scoped to one or more selected libraries or folders. It supports:

- Match any of these keywords
- Match all of these keywords
- Match none of these keywords
- Optional capture-date constraint
- Optional rating constraint
- Optional media-type inclusion or exclusion

Smart collections update as indexing changes. An asset appears once in a smart-collection wall even when several keywords match. Smart collections may span roots because one SQLite catalog owns the installation.

Smart collections do not own derivatives and never act as cache-eviction units.

## Photo-wall loading

Indexed dimensions reserve final geometry before a thumbnail exists. A representative colour fills the reserved tile. The thumbnail crossfades over that colour when ready. The transition does not cause layout shift.

The wall requests thumbnail size classes based on rendered tile size and device pixel ratio. It prefetches only a bounded region beyond the viewport. Fast scrolling cancels or deprioritizes work for rows left far behind.

Unavailable assets retain their original wall positions. Their cached thumbnails reduce saturation and contrast, and a faded question mark appears in the centre. Text labels and accessible names also identify the unavailable state, so colour is not the only cue.

## Immersive viewer

Opening a wall item transitions into an edge-to-edge viewer without rebuilding the surrounding query. Previous and next navigation follows the current stable result order.

On desktop:

- Pointer movement reveals the title and essential controls.
- Moving into the bottom reveal zone shows the horizontal filmstrip.
- Controls fade after roughly 2.5 seconds of inactivity.
- Keyboard navigation, zoom shortcuts, and escape-to-close are supported.

On touch:

- One tap toggles controls and the filmstrip.
- Controls fade after roughly 3.5 seconds of inactivity.
- Pinch and drag remain reserved for zoom and pan.

The viewer supports fit, actual-size, continuous zoom, pan, and return-to-fit. Replacing a preview with a higher-resolution derivative preserves the exact zoom and pan transform.

## Progressive media pipeline

The viewer selects the best immediately available source and refines it in place:

1. Embedded preview or wall thumbnail
2. Colour-managed screen preview
3. Higher-resolution tiles when zoom demands them

Poor embedded RAW previews are temporary. The scheduler produces a better decode off the interaction path and crossfades when it is complete.

Large images and RAW files use on-demand deep-zoom tiles rather than loading one full-resolution bitmap into the WebView. Display derivatives default to sRGB for consistent cross-platform rendering. The catalog retains source profile information for future colour-management improvements.

The first format target includes JPEG, PNG, TIFF, HEIF/HEIC, WebP, AVIF, and common camera RAW formats. Metadata sources include embedded EXIF, IPTC, and XMP plus adjacent XMP sidecars.

Video support is secondary. The index records container and codec information and generates a poster frame. The viewer plays formats supported by the active browser or WebView. MP4 with H.264 video and AAC audio is the expected common case. Unsupported codecs show a clear state and do not trigger automatic transcoding in the first release.

## Cache policy

The cache has three tiers:

1. Catalog metadata remains until the user removes the library.
2. Wall thumbnails are durable and are reported separately in storage settings.
3. Screen previews, full RAW decodes, deep-zoom tiles, poster frames, and future video proxies share a configurable large-derivative limit.

The default large-derivative limit uses an automatic setting based on 10 percent of the cache volume, capped at 100 GiB. The user may set an explicit limit.

Large-derivative eviction operates on physical folder groups, never individual assets. Viewing an asset refreshes its folder group's last-viewed time. When space is required, the cache manager removes the oldest eligible group first. It does not evict the active viewer's group or a group with a derivative write in progress.

This group policy intentionally means that opening one image refreshes the whole group. Very large groups may therefore occupy significant space until an older access time makes them eligible.

Every derivative key includes the source signature, resolved orientation, decoder version, requested size or tile coordinate, and colour conversion. Source changes invalidate stale outputs without discarding unrelated cache entries.

Derivative writers use temporary files and atomic replacement. Startup reconciliation removes abandoned temporary files and repairs database references to missing derivatives.

## Availability and recovery

The system distinguishes an unavailable root from an online root containing a missing file. Both preserve cached catalog entries.

Clicking an unavailable wall item does not open the viewer.

On desktop, a recovery sheet offers:

- Try again
- Locate folder
- The last known path

Locate folder opens a native directory chooser. Relinking occurs at the library or folder-group level. The core verifies enough expected relative paths and signatures to prevent an accidental bind to the wrong folder.

On the web portal, the sheet shows the missing mount path and offers Try again. It explains that the Docker host or container mount must be restored. The browser never receives a server-side folder picker.

## Error handling and recovery

- An unreadable or corrupt file creates a catalog warning and does not stop its scan.
- A decoder crash fails the current job, records a bounded diagnostic, and leaves the worker restartable.
- A full cache volume pauses derivative generation while catalog browsing continues.
- A failed SQLite migration restores the pre-migration backup and leaves the prior application version usable.
- Catalog rebuild removes only derived local data and never touches media roots.
- Settings and smart collections receive a separate backup so a catalog rebuild does not lose them.
- Progress and warnings remain available in the library panel without permanent banners over the wall.

## Security and path safety

Source roots are read-only by product policy and deployment configuration. The service resolves every requested asset or derivative through catalog IDs. It does not accept arbitrary absolute paths from the interface.

The Tauri WebView is unprivileged and receives a narrow command allowlist. The web API is same-origin and serves derivatives rather than arbitrary filesystem content.

The first web release is for a trusted LAN or VPN. Public exposure is unsupported. Authentication, rate limiting, and a hardened public deployment profile require a separate design.

## Accessibility and motion

- All wall, navigation, filter, and viewer operations have keyboard paths.
- Focus remains visible when controls appear or disappear.
- Unavailable, warning, rating, and type states do not depend on colour alone.
- Touch targets meet mobile sizing requirements.
- Animations preserve spatial continuity and respect reduced-motion preferences.
- Control fade timers pause while a control has keyboard focus or an open menu.

## Operations

The library panel shows source availability, scan progress, warnings, and pause or resume controls. It avoids per-file progress noise during large scans.

The Docker server exposes a health endpoint that reports application readiness, database state, cache writability, and source availability without leaking credentials or unrestricted paths.

Desktop packages target signed and notarized macOS builds, signed Windows installers, and documented Linux packages. Release automation builds and smoke-tests each platform independently.

## Testing strategy

### Rust unit tests

Test folder-policy matching, metadata precedence, keyword normalization, stable sorting, cursor pagination, smart queries, path safety, derivative keys, cache-group recency, and eviction.

### Media fixtures

Maintain a small licensed fixture set covering supported still formats, representative RAW formats, XMP sidecars, conflicting metadata, orientation, colour profiles, malformed files, unavailable files, and common MP4 codec combinations.

### Integration tests

Exercise initial scans, incremental rescans, missed filesystem events, offline roots, individual missing files, relinking, source edits, cache pressure, interrupted writes, worker crashes, and migration rollback.

### Interface tests

Use component and accessibility tests for React behavior. Browser end-to-end tests cover library selection, multi-folder unions, wall resizing, filters, search, smart collections, unavailable recovery, viewer navigation, and mobile control toggling.

### Platform tests

Run packaged smoke tests on macOS, Windows, and Linux. Maintain a small set of visual snapshots on WKWebView, WebView2, and WebKitGTK to catch platform-specific rendering changes.

### Performance tests

Generate a synthetic one-million-asset catalog and measure query latency, search latency, memory growth, wall virtualization, resize anchoring, and sustained scrolling. Media benchmarks measure first thumbnail, cached preview, progressive RAW refinement, and deep-zoom tile delivery over both local and delayed filesystem fixtures.

## Implementation decomposition

This design sets system-wide contracts. The project is too large for one safe implementation batch, so delivery is split into independently verifiable slices.

1. Catalog foundation: Rust workspace, SQLite schema, library and folder policies, staged indexing, metadata provenance, derivative primitives, and a minimal HTTP diagnostic surface.
2. Browse vertical slice: shared React shell, typed service contract, web transport, vertical justified wall, virtualization, progressive thumbnails, sorting, and multi-folder queries.
3. Search and organization: FTS5 search, filters, keyword navigation, and cross-library smart collections.
4. Viewer and cache: immersive viewer, responsive controls, progressive RAW refinement, deep zoom, cache tiers, group eviction, and unavailable states.
5. Desktop delivery: Tauri transport, native folder selection and relinking, idle scheduling, installers, and platform smoke tests.
6. Web delivery: Docker image, Compose SMB mount profiles, LAN configuration, mobile behavior, health checks, and supported MP4 playback.

Each slice should receive a detailed implementation plan and pass its tests before the next slice begins. The first implementation plan will cover the catalog foundation only.
