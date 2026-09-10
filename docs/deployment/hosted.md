# Hosted deployment

The hosted image serves the web interface and API on port 8080. It runs as UID and GID 10001, reads photos from `/photos`, and writes only to its data and cache mounts. Every browser URL is origin-relative. A deployment at `https://photos.docker.jenner.lan` needs no hostname or public-URL environment variable.

The `/photos` mount is the highest folder the web interface can browse. Choose **Add folder** in the hosted interface to list its child directories, move through nested directories, and open any contained folder as the gallery source. The browser never receives the host path and cannot navigate above the mounted root. **Include subfolders** controls whether the gallery shows only photos directly inside the selected folder or also includes its descendants.

## Saved folders

Opening a folder adds a shortcut to the left sidebar. Its menu lets you rename
or remove the shortcut. Labels use natural alphabetical order, so Album 2
precedes Album 10. Hovering a row shows its full label and root-relative path.
Removing the active shortcut clears the gallery; it does not delete photos,
catalogue records, or cached images.

The hosted list lives in localStorage for each browser profile and site, under
an opaque identity for the configured catalogue root. It survives page reloads
and server restarts without accounts. Tabs share additions, labels and removals,
but keep independent active folders in sessionStorage. Removing a folder clears
any tab viewing it. Separate browser profiles have independent lists. Changing
the configured root identity selects a separate list. If browser storage is
blocked or full, Mote retains changes in memory and displays a persistence notice.

Desktop shortcuts instead live in the app profile's SQLite catalogue and show
native paths in their tooltips.

Unavailable entries stay visible with an amber warning and a Remove-only menu.
Selecting one requests an access check. Startup and focus also check saved
folders. Checks for the same folder share one running operation across clients
of the server process and reuse results for five seconds. A caller stops waiting
after five seconds; a slow filesystem operation keeps its slot until it finishes.
The coordinator allows four filesystem workers, 256 tracked keys and 64 waiters
per key. No periodic availability polling runs in the browser.

If an open folder becomes unavailable, cached images remain viewable with corner
warnings. Images without cached content cannot open. Switching away or reloading
requires a successful access check before that folder can reopen. Restoring
access clears the folder warning while preserving independent image errors.

## Build the image

Build with Podman:

```bash
podman build -t localhost/photo-viewer:dev -f Containerfile .
```

Docker uses the same file:

```bash
docker build -t photo-viewer:dev -f Containerfile .
```

The image starts `photo-server` as `10001:10001`. It contains no source photos, catalogue, or generated derivatives.

## Required host storage

The SQLite catalogue and generated image cache must use host bind mounts. Podman named volumes live inside the Podman VM on macOS, so they do not meet this requirement and can silently consume the VM disk.

Choose two separate host directories and expose their absolute paths as `PHOTO_VIEWER_DATA_PATH` and `PHOTO_VIEWER_CACHE_PATH`. The Compose file refuses to start if either variable is missing or empty. Do not put these directories below the photo source.

For a local test from the repository root, `./runtime/data` and `./runtime/cache` are suitable:

```bash
mkdir -p ./runtime/data ./runtime/cache
export PHOTO_VIEWER_DATA_PATH="$PWD/runtime/data"
export PHOTO_VIEWER_CACHE_PATH="$PWD/runtime/cache"
```

The container runs as UID and GID 10001 and needs read, write, and directory traversal permission on both directories. On a rootless Linux Podman host, set ownership in Podman's user namespace:

```bash
podman unshare chown -R 10001:10001 \
  "$PHOTO_VIEWER_DATA_PATH" \
  "$PHOTO_VIEWER_CACHE_PATH"
```

For rootful Podman or Docker on Linux, use `sudo chown -R 10001:10001` instead. Podman on macOS shares bind-mounted host directories through its VM. For disposable `./runtime` testing only, `chmod 0777 ./runtime/data ./runtime/cache` is a simple fallback if the container reports unwritable storage.

## Run with Podman or Docker

Mount the photo source read-only and bind both required storage paths:

```bash
podman run -d --name photo-viewer \
  -p 127.0.0.1:8080:8080 \
  -e PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
  -e PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
  -e PHOTO_VIEWER_SOURCE_ROOT=/photos \
  -e PHOTO_VIEWER_BIND=0.0.0.0:8080 \
  -v /srv/photos:/photos:ro,Z \
  -v "$PHOTO_VIEWER_DATA_PATH:/var/lib/photo-viewer:Z" \
  -v "$PHOTO_VIEWER_CACHE_PATH:/var/cache/photo-viewer:Z" \
  localhost/photo-viewer:dev
```

On a host without SELinux, omit `,Z`. Docker uses the same host paths:

```bash
docker run -d --name photo-viewer \
  -p 127.0.0.1:8080:8080 \
  -e PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
  -e PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
  -e PHOTO_VIEWER_SOURCE_ROOT=/photos \
  -e PHOTO_VIEWER_BIND=0.0.0.0:8080 \
  -v /srv/photos:/photos:ro \
  -v "$PHOTO_VIEWER_DATA_PATH:/var/lib/photo-viewer" \
  -v "$PHOTO_VIEWER_CACHE_PATH:/var/cache/photo-viewer" \
  photo-viewer:dev
```

The `/photos` mount must remain read-only.

## Compose

`deploy/compose.yaml` binds the service to loopback, mounts `${PHOTO_PATH:-/srv/photos}` read-only, and bind-mounts the two required host storage directories. Its `Z` mount option gives each directory a private SELinux label on enforcing Linux hosts; Compose implementations on hosts without SELinux accept the same file.

Keep all three host paths in an environment file so every Compose command uses the same source and storage. For the local test directories above, create this ignored test file from the repository root:

```bash
cat > ./runtime/compose.env <<EOF
PHOTO_PATH="/mnt/archive/photos"
PHOTO_VIEWER_DATA_PATH="$PWD/runtime/data"
PHOTO_VIEWER_CACHE_PATH="$PWD/runtime/cache"
EOF
podman compose --env-file ./runtime/compose.env \
  -f deploy/compose.yaml up -d --build
```

Docker Compose uses the same file:

```bash
docker compose --env-file ./runtime/compose.env \
  -f deploy/compose.yaml up -d --build
```

For a permanent installation, keep the environment file in a protected host configuration directory rather than below `./runtime`. Use absolute paths in it, as shown above, so Compose provider differences cannot resolve a relative path below `deploy/`. `compose down` does not remove bind-mounted host data.

## Health and logs

Check the local endpoint:

```bash
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
podman logs photo-viewer
```

Health is `healthy` when the catalogue, cache, and source are available. It becomes `degraded` when a previously catalogued source goes offline or has an active warning. An unwritable catalogue or cache returns HTTP 503 with `unhealthy`. Health responses do not expose native paths or filenames.

## Reverse proxy

Start from `deploy/nginx.conf.example`. The example proxies to the loopback listener, forwards the original host and scheme as untrusted request metadata, disables buffering for event streams, and keeps long-lived streams open for one hour. Install the certificate and key paths shown in the example, then validate and reload Nginx:

```bash
sudo nginx -t
sudo nginx -s reload
```

The server never uses `Host`, `X-Forwarded-Host`, or `X-Forwarded-Proto` to construct links. Interface assets, API calls, event streams, and derivatives stay on the browser's current origin.

## Backup and restore

The catalogue is `$PHOTO_VIEWER_DATA_PATH/catalog.sqlite` on the host. Before any filesystem operation, require the variable explicitly so an unset path cannot become the host root. Stop the service before copying the data directory so the SQLite file and its sidecars form one consistent backup.

For a container created by the direct `podman run` command above, use this rootless Linux backup:

```bash
(
  set -eu
  : "${PHOTO_VIEWER_DATA_PATH:?export the absolute host data path first}"
  backup_root="$PWD/backups"
  backup_dir="$backup_root/photo-viewer-data-$(date -u +%Y%m%dT%H%M%SZ)"
  stopped=false
  restart_after_failure() {
    exit_status=$?
    trap - EXIT
    if [ "$stopped" = true ]; then podman start photo-viewer || true; fi
    exit "$exit_status"
  }
  trap restart_after_failure EXIT
  mkdir -p "$backup_root"
  mkdir "$backup_dir"
  podman stop photo-viewer
  stopped=true
  podman unshare cp -a "$PHOTO_VIEWER_DATA_PATH/." "$backup_dir/"
  podman start photo-viewer
  stopped=false
  printf 'catalogue backup: %s\n' "$backup_dir"
)
```

For a Compose deployment, load the same trusted environment file used to create the service and address the service through Compose:

```bash
(
  set -eu
  compose_env="./runtime/compose.env"
  set -a
  . "$compose_env"
  set +a
  : "${PHOTO_VIEWER_DATA_PATH:?compose environment must set the data path}"
  backup_root="$PWD/backups"
  backup_dir="$backup_root/photo-viewer-data-$(date -u +%Y%m%dT%H%M%SZ)"
  stopped=false
  restart_after_failure() {
    exit_status=$?
    trap - EXIT
    if [ "$stopped" = true ]; then
      podman compose --env-file "$compose_env" \
        -f deploy/compose.yaml start photo-viewer || true
    fi
    exit "$exit_status"
  }
  trap restart_after_failure EXIT
  mkdir -p "$backup_root"
  mkdir "$backup_dir"
  podman compose --env-file "$compose_env" \
    -f deploy/compose.yaml stop photo-viewer
  stopped=true
  podman unshare cp -a "$PHOTO_VIEWER_DATA_PATH/." "$backup_dir/"
  podman compose --env-file "$compose_env" \
    -f deploy/compose.yaml start photo-viewer
  stopped=false
  printf 'catalogue backup: %s\n' "$backup_dir"
)
```

Replace `./runtime/compose.env` with the permanent environment-file path used by your installation. On Podman for macOS, or Docker where your host account owns the directory, use `cp -R "$PHOTO_VIEWER_DATA_PATH/." "$backup_dir/"` after the same variable guard. A rootful Linux deployment can use `sudo cp -a`. Protect the backup like the photo catalogue it describes. To restore, stop the service with the matching direct-run or Compose command, require and verify `PHOTO_VIEWER_DATA_PATH` again, copy the saved directory contents into it, restore UID and GID 10001 access, and then start the service through the same route.

`PHOTO_VIEWER_CACHE_PATH` contains rebuildable JPEG derivatives. Backing it up reduces regeneration work but is optional. If it is lost or cleared, keep the data directory and source mount. The server reconciles missing cache rows at startup and regenerates derivatives when a wall or viewer requests them.

### Migrating existing Podman named volumes

If an earlier deployment used `photo-viewer-data` and `photo-viewer-cache` named volumes, stop the service and copy both volumes into the new host directories before starting this version. Compose often prefixes volume names with its project name, so identify the exact names first with `podman volume ls`. Keep the old volumes until the new deployment reports healthy and the gallery has been checked.

## Restart and upgrade

A normal restart keeps both host storage directories. For the direct-run container:

```bash
podman restart photo-viewer
```

For Compose:

```bash
podman compose --env-file ./runtime/compose.env \
  -f deploy/compose.yaml restart photo-viewer
```

For an upgrade, build or pull the new image, stop and remove only the container, then recreate it with the same source mount and host storage paths. With Compose:

```bash
podman compose --env-file ./runtime/compose.env \
  -f deploy/compose.yaml build --pull
podman compose --env-file ./runtime/compose.env \
  -f deploy/compose.yaml up -d
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
```

Replace `./runtime/compose.env` with the permanent environment-file path used by your installation.

Keep a current catalogue backup before an upgrade. Never delete the host data directory as part of image replacement. Cached wall and viewer derivatives remain usable across process restarts and temporary source outages.

## Acceptance smoke test

`scripts/hosted-smoke.sh` builds `localhost/photo-viewer:dev`, creates only resources prefixed `photo-viewer-smoke-`, and runs the complete browser restart and offline lifecycle with Podman by default:

```bash
./scripts/hosted-smoke.sh
```

Run the same smoke test with Docker by selecting it as the container engine:

```bash
CONTAINER_ENGINE=docker ./scripts/hosted-smoke.sh
```

The script needs the selected container engine, curl, Node.js, npm, and the Playwright Chromium headless shell. It verifies that the image defaults to UID and GID 10001, then runs the test container as the host user so its temporary bind mounts remain writable under standard rootful Docker and rootless or machine-hosted Podman. Docker daemons using rootless mode or `userns-remap` are rejected because their user-namespace mapping cannot safely use these host-owned test directories. The script bind-mounts test-only directories below `./runtime`, confirms the catalogue and cache files appear there, and removes only its exact container, network, host test directories, and temporary browser state.
