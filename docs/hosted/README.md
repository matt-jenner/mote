# Host Mote with Podman or Docker

The hosted version serves Mote's web interface and API on port 8080. It reads a
photo archive mounted at `/photos` and writes its catalogue and generated
previews to two separate mounts. The container runs as UID and GID 10001.

The browser can explore folders below `/photos`, but it cannot browse above
that root or see the host path. Mount the narrowest useful root instead of an
entire filesystem.

## Before you start

Install Podman or Docker with Compose support. You also need a local checkout
of this repository to build the current image.

Choose three absolute host paths:

- `PHOTO_PATH` is the mounted root of the photo archive;
- `PHOTO_VIEWER_DATA_PATH` stores the SQLite catalogue;
- `PHOTO_VIEWER_CACHE_PATH` stores generated thumbnails and previews.

Keep the data and cache directories outside the photo archive. Mote must have
read access to the archive and read-write access to both state directories.

## Mount the root share folder

Mount a NAS or remote share on the container host first. Mote does not mount
SMB, NFS, or another remote protocol itself. For example, if the host mounts a
share at `/mnt/photos`, use that exact path for `PHOTO_PATH`.

The Compose file maps the host root to `/photos` with a read-only mount:

```text
/mnt/photos on the host -> /photos in the container -> folder browser in Mote
```

The selected root controls what every hosted user can browse. If you mount
`/mnt/photos/family`, users cannot move up to `/mnt/photos`. Keep the share
mounted at the same path across restarts so catalogue identities remain stable.

Test host access before starting Mote:

```bash
find /mnt/photos -maxdepth 1 -type d -print
```

If that command cannot traverse the folders as the service host account, the
container will not be able to index them either.

## Storage and permissions

Create separate host directories for persistent state:

```bash
sudo mkdir -p /var/lib/mote/data /var/lib/mote/cache
```

For rootful Docker or Podman on Linux, give UID and GID 10001 ownership:

```bash
sudo chown -R 10001:10001 /var/lib/mote/data /var/lib/mote/cache
```

For rootless Podman, apply ownership inside its user namespace:

```bash
podman unshare chown -R 10001:10001 \
  /var/lib/mote/data /var/lib/mote/cache
```

The photo root needs read and directory traversal permission only. Do not make
the archive writable to solve a data or cache permission error. Check the two
state mounts instead.

On SELinux hosts, the checked-in Compose file uses `:Z` for private labels. If
you use direct Docker or Podman commands, add `:Z` to all three bind mounts on
an enforcing host. Omit it on hosts without SELinux.

Podman named volumes on macOS live inside the Podman virtual machine. Use host
bind mounts for data and cache so their disk use is visible and manageable.

## Start with Compose

Create a protected environment file outside the repository, for example
`/etc/mote/compose.env`:

```dotenv
PHOTO_PATH=/mnt/photos
PHOTO_VIEWER_DATA_PATH=/var/lib/mote/data
PHOTO_VIEWER_CACHE_PATH=/var/lib/mote/cache
MOTE_ACCENT_COLOR="#7C3AED"
```

Build and start with Podman:

```bash
podman compose --env-file /etc/mote/compose.env \
  -f deploy/compose.yaml up -d --build
```

Docker Compose uses the same file:

```bash
docker compose --env-file /etc/mote/compose.env \
  -f deploy/compose.yaml up -d --build
```

Mote listens on `127.0.0.1:8080` by default. Open
`http://127.0.0.1:8080`, or place a reverse proxy in front of it. The
[Nginx example](../../deploy/nginx.conf.example) includes event-stream settings
and keeps the application on one origin. Mote uses origin-relative URLs, so a
reverse-proxied deployment does not need a public hostname setting.

## Hosted settings

The Compose file accepts these optional settings:

| Variable | Default | Effect |
| --- | --- | --- |
| `MOTE_ACCENT_COLOR` | Mote green | Six-digit CSS hex colour sent to browsers at startup |
| `MOTE_HEIC` | `enabled` | Set to `disabled` when building an image without HEIC support |
| `PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS` | off | Set to `true` to show original download links in Picks |

`PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS` is supported by the server but is not
present in `deploy/compose.yaml`. Add it to the service environment when you
intend to allow downloads. Restart the container after changing any setting.

## Check health and logs

```bash
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
podman compose --env-file /etc/mote/compose.env \
  -f deploy/compose.yaml logs photo-viewer
```

Use `docker compose` in the second command when running Docker.

Health is `healthy` when the catalogue, cache, and source are available. A
disconnected source or active source warning reports `degraded`. An unwritable
catalogue or cache returns HTTP 503 with `unhealthy`.

Common failures:

| Symptom | Check |
| --- | --- |
| `PHOTO_VIEWER_DATA_PATH` or cache variable error | Both variables must contain absolute host directories |
| Permission denied for catalogue or cache | UID and GID 10001 need write and traversal access |
| Empty folder browser | Confirm `PHOTO_PATH`, host mount state, read access, and SELinux labels |
| Photos vanish after a restart | Remount the share at the same host path and check `/healthz` |

## Back up and upgrade

The catalogue is `catalog.sqlite` inside `PHOTO_VIEWER_DATA_PATH`. Stop the
service before copying the data directory so the SQLite database and sidecar
files form one consistent backup. This example creates a unique destination
and restarts the service if the copy fails:

```bash
set -eu
compose_env=/etc/mote/compose.env
backup_dir="/var/backups/mote-data-$(date -u +%Y%m%dT%H%M%SZ)"
stopped=false
restart_after_failure() {
  status=$?
  trap - EXIT
  if [ "$stopped" = true ]; then
    podman compose --env-file "$compose_env" \
      -f deploy/compose.yaml start photo-viewer || true
  fi
  exit "$status"
}
trap restart_after_failure EXIT
sudo mkdir -p /var/backups
sudo mkdir "$backup_dir"
podman compose --env-file "$compose_env" \
  -f deploy/compose.yaml stop photo-viewer
stopped=true
sudo cp -a /var/lib/mote/data/. "$backup_dir/"
podman compose --env-file "$compose_env" \
  -f deploy/compose.yaml start photo-viewer
stopped=false
trap - EXIT
printf 'catalogue backup: %s\n' "$backup_dir"
```

Adapt ownership and the Compose command to your container engine. Protect the
backup because the catalogue describes the photo archive. The cache is
rebuildable, though backing it up avoids regenerating previews.

To upgrade, keep the photo, data, and cache paths unchanged, then rebuild and
recreate only the container:

```bash
podman compose --env-file /etc/mote/compose.env \
  -f deploy/compose.yaml build --pull
podman compose --env-file /etc/mote/compose.env \
  -f deploy/compose.yaml up -d
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
```

Do not add `--volumes` to `compose down`. Catalogue and cache mounts contain
persistent host data.

## Test a hosted build

The repository smoke test builds an isolated image, mounts disposable source
and state directories, checks restart and offline behaviour, then removes only
its named test resources:

```bash
./scripts/hosted-smoke.sh
CONTAINER_ENGINE=docker ./scripts/hosted-smoke.sh
```

The test requires the chosen container engine, curl, Node.js, npm, and the
Playwright Chromium browser. See the
[developer guide](../developer/README.md) for local prerequisites.
