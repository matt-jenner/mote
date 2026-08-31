# Cross-platform release packaging and shared engine design

## Status

Approved through the design conversation on 31 August 2026. This document defines the first test-release packaging for the desktop and hosted variants of Photo Viewer.

## Purpose

Photo Viewer will remain one product with several independently installed variants. The variants share the Rust photo engine and React interface, but each installation owns its own catalogue and derivative cache.

The first release must let invited testers download an unsigned desktop package for macOS, Fedora, or Windows. It must also let an operator run the hosted version with Docker or Podman against a configured read-only photo root.

## Architecture

The existing Rust crates form the shared engine:

- `photo-domain`
- `photo-core`
- `photo-metadata`
- `photo-catalog`
- `photo-cache`
- `photo-indexer`
- `photo-app-service`

`photo-app-service` is the public orchestration boundary used by product shells. Photo indexing, metadata precedence, catalogue queries, cache policy, derivative generation, offline behaviour, and read-only source guarantees stay below this boundary.

The desktop Tauri application links the engine into its native process. Tauri commands adapt native folder selection and events to the shared `PhotoService` interface.

The hosted `photo-server` binary links the same engine. It adapts the engine to HTTP, server-sent events, hosted folder selection, and static delivery of the shared web interface.

The React interface contains no catalogue or indexing implementation. It selects either the Tauri adapter or HTTP adapter at startup. Gallery and viewer components remain unaware of the transport.

The browser does not run the Rust engine through WebAssembly. Filesystem, SMB, SQLite, background indexing, and cache ownership remain desktop or server responsibilities.

## Installation-local state

Every installation creates its own SQLite catalogue, configuration, and derivative cache in its platform application-data locations.

- A macOS installation does not share its database with Windows, Linux, or the hosted service.
- A Windows or Linux installation indexes paths as seen by that machine.
- A hosted installation stores its catalogue and cache in persistent container volumes.
- Two installations may read the same photo share, but their catalogues and caches remain independent.
- No catalogue synchronisation or remote desktop-to-service mode is included.

The source photo folders remain read-only. Catalogues and derivatives are disposable local data that the engine can rebuild from the source.

## Release artifacts

The first test release publishes these artifacts:

| Variant | Architecture | Artifact |
| --- | --- | --- |
| macOS desktop | Universal Apple Silicon and Intel | Unsigned `.dmg` containing `Photo Viewer.app` |
| Fedora desktop | x86-64 | Unsigned `.rpm` |
| Windows desktop | x86-64 | Unsigned NSIS setup `.exe` |
| Hosted service | Linux AMD64 and ARM64 | Multi-architecture OCI image |

GitHub also provides the tagged source archive. Each release includes SHA-256 checksums for downloadable desktop artifacts and records the source commit used to build them.

Flatpak packaging is deferred. The engine and interface will continue using platform application-data paths and the existing folder-selection boundary so a later Flatpak can add portal and sandbox handling without changing gallery behaviour.

Native launchd, systemd, and Windows Service packages are also deferred. The hosted service is distributed as an OCI image in this release.

## Hosted deployment

The repository retains a standards-compatible `Containerfile` that both Docker and Podman can build. The release workflow also publishes the tested multi-architecture image to GitHub Container Registry.

The container uses these stable internal paths:

- `/photos` for source media, mounted read-only
- `/var/lib/photo-viewer` for the SQLite catalogue and durable application data
- `/var/cache/photo-viewer` for rebuildable derivative cache data

`deploy/compose.yaml` accepts the host photo path through `PHOTO_PATH` and mounts it at `/photos:ro`. A local build uses:

```bash
PHOTO_PATH=/mnt/archive/photos docker compose -f deploy/compose.yaml up -d --build
```

The Compose file also accepts `PHOTO_VIEWER_IMAGE` so an operator can select a versioned image from GitHub Container Registry instead of building locally. Private registry pulls require GitHub Container Registry authentication.

`PHOTO_VIEWER_SOURCE_ROOT` remains `/photos` inside the container. Public URLs remain origin-relative, so an HTTPS reverse proxy does not require a configured public hostname.

The data and cache volumes are never placed beneath the photo source. Routine container replacement preserves both volumes.

## GitHub release workflow

GitHub Actions builds releases from version tags such as `v0.2.0`. The workflow performs these stages:

1. Validate that the tag and application version agree.
2. Run the shared Rust, interface, hosted-server, and source-safety test gates.
3. Build the desktop artifact on its native operating-system runner.
4. Build and smoke-test the OCI image for AMD64 and ARM64.
5. Calculate checksums and assemble installation notes.
6. Create a GitHub pre-release and attach all desktop artifacts.
7. Publish the versioned OCI image to GitHub Container Registry.

The release remains unpublished if any required artifact or verification gate fails. This prevents testers receiving a partial release whose platforms came from different commits.

The repository and releases may remain private during testing. Testers receive GitHub collaborator access. The workflow does not publish artifacts to a separate public download service.

## Unsigned test distribution

The first packages are deliberately unsigned. Release notes state that they are private test builds and give concise instructions for the operating-system warning users will encounter.

- macOS testers may need to approve the app in Privacy and Security before first launch.
- Windows testers may see a SmartScreen warning.
- Fedora testers install the unsigned local RPM explicitly.

Signing, Apple notarisation, storefront submission, and automatic updates are outside this release. The packaging must leave room to add signing secrets to the CI jobs later without rebuilding the application architecture.

## Verification

Pull requests run tests and build checks without publishing artifacts. A tagged release repeats those checks before packaging.

The release gates cover:

- formatting and lint checks for Rust and TypeScript
- all Rust workspace unit and integration tests
- all interface unit and browser tests
- desktop interface production build
- hosted interface production build
- source-media write and delete safety tests
- platform package creation on its native runner
- container startup, health endpoint, folder browsing, wall loading, and derivative delivery against read-only fixture media
- confirmation that the catalogue and cache survive container recreation

The release workflow limits Rust build concurrency where appropriate. CI jobs clean up their exact containers, networks, volumes, and temporary browser state after testing.

## Acceptance criteria

The design is complete when:

- one version tag produces all three desktop packages from the same commit
- the macOS package runs on Apple Silicon and Intel hardware supported by the configured minimum macOS version
- the Windows setup executable installs and launches on x86-64 Windows
- the RPM installs and launches on supported x86-64 Fedora releases
- the published OCI image runs on AMD64 and ARM64 hosts
- Docker and Podman can also build the checked-in `Containerfile`
- an operator can select the hosted photo root by setting `PHOTO_PATH`
- the hosted source mount remains read-only
- every installation creates and uses only its own catalogue and cache
- release failure never publishes a partial set of desktop artifacts
- invited collaborators can download the unsigned packages from a GitHub pre-release

## Deferred work

- Flatpak and Flathub distribution
- Linux formats other than RPM
- application signing and notarisation
- public release hosting
- automatic application updates
- native background-service installers
- catalogue or cache synchronisation
- desktop clients connecting to a hosted service
