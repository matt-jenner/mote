# Hosted web vertical slice design

Date: 2026-08-29

Status: approved in conversation, awaiting written-spec review

## Goal

Deliver a responsive hosted version of Photo Viewer as one OCI container. The
same image must run under Podman during development and Docker in deployment.
The production instance will sit behind an HTTPS reverse proxy at
`https://photos.docker.jenner.lan/`.

The container receives one photo root mounted read-only at `/photos`. Each
browser chooses its own folder beneath that root. One browser must never change
another browser's folder, ordering, appearance, subfolder scope, wall, or
viewer sequence. Browsers may share deduplicated scan and derivative work.

This is a functional product slice, not a throwaway web mock. It reuses the
existing wall, progressive thumbnail loading, viewer, filmstrip, zoom and pan,
offline catalogue, and cache behavior.

## Four-hour boundary

The slice includes:

- a lazy server-backed folder browser;
- independent browser selections;
- the existing justified wall and immersive viewer;
- oldest-first and newest-first ordering;
- per-browser current-folder or include-subfolders scope;
- progressive scan, thumbnail, and preview updates;
- cached derivative delivery over HTTP;
- retained catalogue and cache volumes;
- offline cached browsing;
- a health endpoint;
- an OCI build plus a Podman smoke test;
- deployment documentation for Docker and an HTTPS reverse proxy.

The slice does not include authentication, authorization, PWA installation,
service-worker caching, video playback, remote administration, public sharing,
or a general file manager. The reverse proxy and local network control access.

## Existing boundaries to preserve

React components depend on the host-neutral TypeScript `PhotoService`
interface. They do not import Tauri or HTTP clients directly. Tauri remains the
desktop adapter. A new HTTP adapter implements the same contract for hosted
browsers.

Source media stays read-only. Production code may read files beneath the
configured photo root but may not create, update, rename, or delete them. SQLite
and generated derivatives remain in separate local data and cache directories.

The catalogue remains shared across browsers. Browser state is not written to
the global `app_state` row used by the desktop application.

## Runtime architecture

`photo-server` becomes the hosted application process. It owns four clear
parts:

1. The static host serves the production React bundle and falls back to
   `index.html` for interface routes.
2. The folder API lists and validates directories beneath the configured photo
   root.
3. Selection-explicit gallery APIs expose catalogue pages, derivative requests,
   interaction state, and progressive events.
4. The derivative handler resolves opaque derivative identifiers to managed
   cache files. It never serves a source-media path.

The Rust application layer gains selection-explicit operations. Existing
desktop methods remain small wrappers that supply the persisted desktop
selection. Hosted handlers always supply their requested selection. This keeps
transport and client state out of the catalogue, scanner, cache, and metadata
layers.

A hosted selection runtime is keyed by folder group. It owns that group's scan
task, derivative coordinator, bounded update channel, and current client-demand
summary. Two browsers viewing the same folder share work. Different folder
groups may progress independently through the common bounded scheduler.
Selection runtimes stop and leave their catalogue state durable after the last
subscriber has gone and queued work has drained.

Scanning is recursive regardless of a browser's display scope. This lets a
scope toggle requery SQLite immediately without another SMB walk. Derivative
demand is the union of connected browsers for that group. Include-subfolders
demand dominates current-folder demand for background prefetch. Each browser
still receives only rows allowed by its own scope.

## Folder browser

The existing Folders control opens a hosted folder browser instead of a native
picker. Desktop behavior does not change.

The folder browser loads one directory level per request. It shows breadcrumbs,
a Back action, readable child directories, loading and failure states, and an
Open this folder action. It lists directories only. It does not inspect photos
or recurse merely to render the picker.

On desktop the browser is a centred dialog. On a narrow or coarse-pointer
viewport it becomes a full-height sheet. Controls meet the existing touch-size,
keyboard, focus-containment, contrast, and reduced-motion requirements.

The API uses mount-relative path segments because the interface must display
folder names and breadcrumbs. It never returns `/photos`, a host path, or a
container path. The server rejects absolute paths, parent traversal, NUL bytes,
files, unreadable directories, and any canonical result outside the configured
root. An in-root symlink may be followed after containment validation. A
symlink that resolves outside the root is rejected.

Selecting a folder creates or reuses its durable folder group and returns a
stable opaque selection ID. The browser stores that ID and its display
breadcrumbs in local storage. If the saved selection no longer resolves, the
interface opens the folder browser at the nearest valid ancestor, falling back
to the mounted root.

## Per-browser state

The hosted interface stores these values in browser local storage:

- selected folder ID and display breadcrumbs;
- appearance;
- gallery scope;
- sort direction.

Each open tab also creates a random client instance ID in session storage. The
HTTP adapter sends it with interaction and event requests. The server aggregates
active-interaction leases by client, so one idle browser cannot resume
background work while another browser is still scrolling. A lease ends when
its event stream closes or its heartbeat expires. The identifier carries no
user identity and is not an authentication mechanism.

Every gallery request carries the opaque selection ID. Wall cursors are bound
to the selection, order, and scope that created them. The server rejects a
cursor reused with different values.

No server cookie or mutable global active selection is required. Opening a
second browser, private window, or phone creates an independent client view.

## HTTP API

All endpoints use the `/api/v1` prefix. JSON errors contain a stable code and a
short user-safe message. They contain no source or cache path.

- `GET /api/v1/bootstrap` returns capabilities and source availability.
- `GET /api/v1/folders?path=<encoded-relative-path>` returns breadcrumbs and
  immediate child directories.
- `POST /api/v1/selections` validates a relative folder and returns its stable
  selection summary.
- `GET /api/v1/selections/{id}` resolves a saved selection or returns 404.
- `GET /api/v1/selections/{id}/wall` accepts scope, direction, cursor, and a
  limit of at most 250.
- `POST /api/v1/selections/{id}/derivatives` accepts scope plus a bounded
  derivative request and returns after the requested foreground work reaches
  its existing success or typed-failure boundary.
- `POST /api/v1/selections/{id}/interaction` records a client's active or idle
  wall interaction lease for scheduling.
- `GET /api/v1/selections/{id}/events` opens a client-identified server-sent
  event stream.
- `GET /api/v1/derivatives/{id}` serves one catalogue-approved cache file.
- `GET /healthz` keeps its existing path-free health response.

The HTTP `PhotoService` maps these endpoints to the existing interface types.
Its derivative URL function returns an origin-relative
`/api/v1/derivatives/{id}` URL.

## Progressive events

Server-sent events are sufficient because updates travel from server to
browser while commands already use HTTP. Each selection runtime assigns a
monotonic event ID. Events use the existing path-free `WallUpdate` payloads.

The channel is bounded. A lagging subscriber receives `resyncRequired` rather
than an unbounded backlog. The browser then reloads its current authoritative
wall page. The HTTP adapter reconnects with the last observed event ID. The
server sends a comment heartbeat often enough to keep ordinary reverse proxies
from closing an idle stream.

SSE responses disable intermediary buffering and compression. The deployment
guide calls out proxy buffering and a long read timeout.

## Derivative delivery

Wall and viewer DTOs contain opaque derivative identifiers, not relative cache
paths. The derivative handler looks up the identifier in SQLite, validates that
the row belongs to managed cache state, rechecks cache containment, and opens
the file read-only.

Successful responses set the detected image content type, an immutable cache
policy, an ETag derived from immutable derivative identity, and
`X-Content-Type-Options: nosniff`. Missing or invalid cache entries fail closed.
The server never falls back to the original photo.

HTTP range support is not required for still-image derivatives in this slice.
It will be needed before hosted video playback.

## Errors and offline behavior

Folder-list errors distinguish an unavailable source, an invalid relative
path, and a permission failure without revealing native paths. The UI keeps the
last good directory listing visible where possible and offers Retry.

An unavailable photo mount does not erase catalogue rows. Saved selections,
wall geometry, metadata, and ready cached derivatives remain usable. Uncached
photos use the existing unavailable cue and do not open. The hosted interface
does not offer Locate Folder.

SSE disconnects do not blank the wall. The adapter reconnects and reloads if
the server reports missed state. A container restart uses the same SQLite and
cache volumes, so a browser can restore its selection without reindexing
unchanged media.

## Reverse proxy and public URLs

The hosted application does not need its public hostname. Static assets, API
calls, event streams, health checks, and derivative references use
origin-relative URLs. The browser resolves them against
`https://photos.docker.jenner.lan`.

The server avoids absolute redirects. It does not use `Host`,
`X-Forwarded-Host`, or `X-Forwarded-Proto` for filesystem or authorization
decisions. The reverse proxy should still forward the standard host, scheme,
and client-address headers for logs and future features.

The deployment guide will include this Nginx-compatible behavior:

```nginx
proxy_set_header Host $host;
proxy_set_header X-Forwarded-Host $host;
proxy_set_header X-Forwarded-Proto $scheme;
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
proxy_buffering off;
proxy_read_timeout 1h;
```

There is no `PHOTO_VIEWER_PUBLIC_URL` setting in this slice. A later feature
that emits absolute links, such as sharing or an OAuth callback, may add an
explicit canonical URL rather than trusting arbitrary forwarded hosts.

## Container contract

The repository gains a multi-stage `Containerfile` that uses standard OCI
instructions only. A Node stage builds the interface. A Rust stage builds the
server. The final Linux image contains the server binary, static interface
files, required runtime certificates, and an unprivileged application user.

Runtime configuration:

```text
PHOTO_VIEWER_SOURCE_ROOT=/photos
PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer
PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer
PHOTO_VIEWER_BIND=0.0.0.0:8080
```

The image exposes port 8080. `/photos` is a read-only bind mount. Data and cache
are persistent writable volumes. Startup rejects missing source configuration,
a non-directory source, and any overlap or symlink alias between source and
local state.

The image declares owned data and cache directories so Docker named volumes
inherit usable permissions. Operators using host bind mounts must grant the
documented container UID write access. The main process runs without root
privileges.

The repository includes Podman commands and a Compose-compatible deployment
example. The example mounts photos with `:ro`, maps the two persistent volumes,
sets restart policy, and publishes port 8080 only to the reverse proxy's
network or loopback address.

## Performance rules

- Folder browsing performs one `read_dir` for the requested directory and no
  recursive media scan.
- Wall pages remain bounded to 250 rows and use keyset cursors.
- SSE queues remain bounded and recover by resync.
- Identical scans and derivative work for the same folder and asset are
  deduplicated across browsers.
- The first visible wall rows keep their existing priority over near-viewport
  and background work.
- Static assets and immutable derivatives use browser cache headers.
- Scope and sort changes query SQLite. They do not walk the mounted share.

The SMB server and Wi-Fi determine cold source-read latency. The product must
show folder and wall progress rather than look frozen during those reads.

## Security and source safety

The server binds to the configured address and assumes the reverse proxy
controls network access. It adds a restrictive content security policy,
`nosniff`, referrer policy, and frame-ancestor policy suitable for the
same-origin application.

Request bodies and query strings have explicit size and count limits. Folder
and selection identifiers are validated before filesystem access. Derivative
identifiers are resolved through SQLite rather than treated as paths.

Automated source-safety coverage snapshots source bytes, sizes, and modification
times before and after folder browsing, scan, derivative generation, sorting,
scope changes, viewer-equivalent requests, container restart, and offline
reads. Production source operations are audited for writes, renames, and
deletes.

## Testing and acceptance

Rust tests cover folder containment, selection-explicit queries, concurrent
selection runtimes, scope isolation, bounded events, derivative containment,
offline behavior, API status and error mapping, static fallback, headers, and
health.

TypeScript tests cover the HTTP adapter, browser-local preferences, reconnect
and resync, error mapping, and capability differences. Existing component and
WebKit suites run unchanged. Browser tests add the hosted folder dialog and two
independent browser contexts selecting different folders.

The container acceptance run will:

1. Build the image with Podman.
2. Start it with a controlled read-only photo tree and persistent temporary
   data and cache volumes.
3. Wait for `/healthz`.
4. Browse and select two different folders in isolated browser contexts.
5. Exercise wall loading, thumbnails, ordering, subfolder scope, viewer,
   filmstrip, zoom, pan, and cached derivative URLs.
6. Confirm that neither browser changes the other.
7. Stop and restart the same container with the same volumes.
8. Confirm selection restoration and cached viewing.
9. Make the source unavailable and confirm offline catalogue behavior.
10. Recheck the source snapshot and traversal rejection.

The slice is ready for demonstration only when the native desktop regression
suite remains green, the hosted acceptance run passes, and the working tree is
clean. It will remain on its feature branch until user acceptance. No merge or
push is part of implementation approval.
