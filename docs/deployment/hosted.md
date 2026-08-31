# Hosted deployment

The hosted image serves the web interface and API on port 8080. It runs as UID and GID 10001, reads photos from `/photos`, and writes only to its data and cache mounts. Every browser URL is origin-relative. A deployment at `https://photos.docker.jenner.lan` needs no hostname or public-URL environment variable.

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

## Run with Podman or Docker

Create retained volumes once, then mount the photo source read-only:

```bash
podman volume create photo-viewer-data
podman volume create photo-viewer-cache
podman run -d --name photo-viewer \
  -p 127.0.0.1:8080:8080 \
  -e PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
  -e PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
  -e PHOTO_VIEWER_SOURCE_ROOT=/photos \
  -e PHOTO_VIEWER_BIND=0.0.0.0:8080 \
  -v /srv/photos:/photos:ro,Z \
  -v photo-viewer-data:/var/lib/photo-viewer:U \
  -v photo-viewer-cache:/var/cache/photo-viewer:U \
  localhost/photo-viewer:dev
```

On a host without SELinux, omit `,Z`. The Docker form uses the same arguments with `docker`, omits `:U`, and relies on Docker's named-volume ownership initialization:

```bash
docker volume create photo-viewer-data
docker volume create photo-viewer-cache
docker run -d --name photo-viewer \
  -p 127.0.0.1:8080:8080 \
  -e PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
  -e PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
  -e PHOTO_VIEWER_SOURCE_ROOT=/photos \
  -e PHOTO_VIEWER_BIND=0.0.0.0:8080 \
  -v /srv/photos:/photos:ro \
  -v photo-viewer-data:/var/lib/photo-viewer \
  -v photo-viewer-cache:/var/cache/photo-viewer \
  photo-viewer:dev
```

The `/photos` mount must remain read-only. Do not put `/var/lib/photo-viewer` or `/var/cache/photo-viewer` below the source tree. For bind-mounted data or cache directories, grant UID 10001 read, write, and directory traversal permission before starting the container. Named volumes avoid host-path permission drift.

## Compose

`deploy/compose.yaml` binds the service to loopback, mounts `${PHOTO_PATH:-/srv/photos}` read-only, and retains the catalogue and derivative cache in named volumes:

```bash
PHOTO_PATH=/mnt/archive/photos podman compose -f deploy/compose.yaml up -d --build
```

Docker Compose uses the same file:

```bash
PHOTO_PATH=/mnt/archive/photos docker compose -f deploy/compose.yaml up -d --build
```

The Compose project owns `photo-viewer-data` and `photo-viewer-cache`. `compose down` keeps them. Do not add `--volumes` during routine restarts or upgrades.

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

The catalogue is `/var/lib/photo-viewer/catalog.sqlite`. Stop the service before copying it so the SQLite file and its sidecars form one consistent backup:

```bash
podman stop photo-viewer
podman run --rm \
  -v photo-viewer-data:/data:ro \
  -v "$PWD/backups:/backup" \
  docker.io/library/debian:bookworm-slim \
  cp /data/catalog.sqlite /backup/catalog.sqlite
podman start photo-viewer
```

Use `docker` instead of `podman` for Docker-managed volumes. Protect the backup like the photo catalogue it describes. To restore, stop the service and copy the file into a fresh data volume owned by UID and GID 10001.

The cache volume contains rebuildable JPEG derivatives. Backing it up reduces regeneration work but is optional. If it is lost or cleared, keep the data volume and source mount. The server reconciles missing cache rows at startup and regenerates derivatives when a wall or viewer requests them.

## Restart and upgrade

A normal restart keeps both volumes:

```bash
podman restart photo-viewer
```

For an upgrade, build or pull the new image, stop and remove only the container, then recreate it with the same source mount and named volumes. With Compose:

```bash
podman compose -f deploy/compose.yaml build --pull
podman compose -f deploy/compose.yaml up -d
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
```

Keep a current catalogue backup before an upgrade. Never delete the data volume as part of image replacement. Cached wall and viewer derivatives remain usable across process restarts and temporary source outages.

## Acceptance smoke test

`scripts/hosted-smoke.sh` builds `localhost/photo-viewer:dev`, creates only resources prefixed `photo-viewer-smoke-`, and runs the complete browser restart and offline lifecycle:

```bash
./scripts/hosted-smoke.sh
```

The script needs Podman, curl, Node.js, npm, and the Playwright Chromium headless shell. It never prunes Podman and removes only its exact container, network, volumes, and temporary browser state.
