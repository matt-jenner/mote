# Linux Flatpak Packaging Design

## Status

Approved through the design conversation on 8 September 2026. This document
replaces the Fedora RPM portion of the earlier cross-platform release packaging
design for Mote's first Linux test package. GitHub Actions integration and
Flathub publication remain deferred.

## Purpose

Mote will ship one distribution-independent Linux desktop package rather than
maintaining packages for individual distributions. The first deliverable is an
x86-64 `.flatpak` bundle that can be installed locally on the current Omarchy
workstation, copied to Fedora, and installed there.

The Flatpak must preserve Mote's read-only source-media guarantee. Users grant
access to individual photo folders through the trusted desktop file chooser;
the application does not receive blanket access to the host filesystem.

## Application identity

Use `io.github.matt_jenner.mote` as the Flatpak/Flathub application ID. The
Flatpak manifest, desktop entry, AppStream metadata, and Linux icons use this
exact identifier. Flathub demangles the underscore to the GitHub owner
`matt-jenner`. Use `io.github.matt-jenner.mote` as Tauri's desktop/macOS
identifier because Tauri rejects underscores.

No migration from the old application-data paths is included. There are no
development profiles that need preserving, and Mote has not shipped a signed
public desktop release under either new identifier. Documentation and tests that
name the old paths are updated to the appropriate platform identifier.

The installed Linux executable is named `mote`.

## Build architecture

Use `flatpak-builder` to compile Mote from locked source inside the Flatpak
build environment. Do not create or unpack an intermediate RPM or Debian
package.

The manifest uses:

- GNOME Platform 49 as the runtime
- GNOME SDK 49 as the build SDK
- `org.freedesktop.Sdk.Extension.node24` branch `25.08`
- `org.freedesktop.Sdk.Extension.rust-stable` branch `25.08`, which supplies
  Rust 1.97.1

The build consumes the root npm lockfile and the desktop Rust lockfile. Generated
Flatpak source declarations make npm and Cargo dependencies available to build
commands without network access. The checked-in source tree supplies the React
interface, Tauri shell, shared Rust crates, brand assets, and metadata.

The build installs the release binary directly into `/app/bin/mote`, along with
the desktop entry, AppStream metadata, and existing Mote icons under names that
match the Flatpak application ID.

Only x86-64 is an acceptance target for this phase. The manifest and packaging
scripts must not embed x86-64 paths or assumptions that would prevent a later
ARM64 CI build; generated dependency sources may use architecture filters when
their upstream artifacts require them.

## Sandbox and folder access

The Flatpak receives only these static runtime permissions:

- Wayland display access
- fallback X11 display access
- GPU rendering access
- shared IPC required by the webview stack

It receives no `home`, `host`, arbitrary-path, or network permission. Mote does
not write source media, and the manifest does not grant source-folder write
access.

Mote's existing Tauri folder dialog is presented through the desktop file
chooser portal inside Flatpak. The host picker can display local folders and
mounted network shares without exposing them to Mote. Selecting a directory
creates a persistent portal grant for that directory. The returned portal path
is passed unchanged to `AppService`, scanned recursively according to the
selected gallery scope, and stored in Mote's installation-local catalogue.

Application data and SQLite state remain under Flatpak's private data area,
and generated derivatives remain under its private cache area below
`~/.var/app/io.github.matt_jenner.mote/`. No catalogue or derivative data is
stored inside a selected photo source.

## Runtime behaviour and failures

On first launch, Mote creates its catalogue and cache through Tauri's standard
application-data path APIs. On subsequent launches, a persistent folder grant
allows the stored portal path to be reopened without another prompt.

If a local or network source is unmounted, moved, unavailable, or has its portal
grant revoked, the existing offline-source behaviour applies. Mote retains the
catalogue and ready cached derivatives, marks the source unavailable, and does
not modify source media. The user can choose the folder again after access is
restored. The package does not fall back to broad filesystem permissions.

Build and packaging commands fail with a clear diagnostic when `flatpak`,
`flatpak-builder`, a required runtime, or a required SDK extension is absent.
They do not install host packages or modify system Flatpak configuration.

## Packaging interface and outputs

Provide a small non-interactive packaging interface for these operations:

- validate generated dependency declarations and desktop metadata
- build and export Mote to a repository-local Flatpak repository
- create a predictably named x86-64 `.flatpak` bundle
- install or update that bundle for the current user
- run the installed application
- inspect the installed application metadata and permissions

Build directories, repositories, and bundles are generated artifacts and are
ignored by Git. The final bundle remains easy to locate and copy to another
machine.

The commands use paths relative to the repository and have no dependency on the
developer's home-directory layout. This makes the same interface suitable for
a later Ubuntu GitHub Actions runner. No GitHub Actions workflow is added or
changed in this phase.

## Desktop integration

Install a desktop entry with the exact ID
`io.github.matt_jenner.mote.desktop`, display name `Mote`, executable `mote`,
and an appropriate graphics/photo-viewer category. Install AppStream metadata
whose component and launchable IDs match the Flatpak and desktop IDs. Reuse the
approved scalable and raster Linux icons from `docs/brand/icons/linux/hicolor`;
do not create new brand artwork.

The metadata is usable by local Linux software centres and structured for a
future Flathub submission. Flathub submission, signing, public release hosting,
and automatic updates are outside this phase.

## Verification

Automated repository tests verify:

- Tauri config uses `io.github.matt-jenner.mote`, while the Flatpak manifest,
  desktop entry, AppStream metadata, and installed icon names use
  `io.github.matt_jenner.mote`
- the manifest contains no blanket filesystem, network, or source-write
  permission
- the manifest uses the specified runtime and SDK extensions
- desktop and AppStream metadata pass the available platform validators
- dependency declarations agree with the committed lockfiles

The local package verification sequence:

1. Run the existing Rust formatting, lint, unit, integration, browser, desktop,
   and source-safety checks relevant to the changed files.
2. Build Mote in `flatpak-builder` without build-command network access.
3. Export the application to a repository-local Flatpak repository.
4. Create the x86-64 `.flatpak` bundle.
5. Install or update it for the current user.
6. Inspect the installed ID, runtime, metadata, and static sandbox permissions.
7. Launch the installed Mote application.

Manual acceptance testing on both the current workstation and Fedora:

1. Select the checked-in demo-photo directory through the system folder picker.
2. Confirm that the wall, derivatives, and immersive viewer work.
3. Exit and relaunch Mote, then confirm that the grant and library still work.
4. Select a mounted NAS photo directory and confirm that it scans and remains
   available after relaunch.
5. Confirm through installed permissions that Mote has not received blanket
   host filesystem or network access.

Portal UI automation is intentionally not a CI acceptance gate because the
trusted picker crosses the application sandbox boundary. A future GitHub
Actions workflow will build and validate the package using the same
non-interactive commands, while real portal behaviour remains a desktop test.

## Acceptance criteria

The phase is complete when:

- the project builds from locked source with `flatpak-builder`
- the generated x86-64 `.flatpak` bundle installs on the current workstation
  and Fedora
- Mote launches from both the command line and desktop application menu
- a folder chosen through the portal can be scanned without blanket filesystem
  permission
- the chosen folder remains accessible after restarting Mote
- a mounted NAS folder visible to the host picker can be selected and scanned
- source files remain byte-for-byte and metadata-time unchanged during normal
  use and source-safety verification
- the catalogue and derivatives remain inside Flatpak-owned data and cache
  paths
- offline and revoked sources retain catalogue records and ready derivatives
- the packaging commands are non-interactive and suitable for later GitHub
  Actions use
- the existing CI workflow is unchanged

## Deferred work

- GitHub Actions Flatpak build and artifact upload
- ARM64 Flatpak builds
- Flathub submission and review
- signing and hosted Flatpak repositories
- automatic updates
- RPM, Debian, AppImage, or other distribution-specific packages
- migration from `app.photoviewer.desktop` local data paths
