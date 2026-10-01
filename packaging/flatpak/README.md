# Mote Flatpak

Mote is built from locked source inside GNOME SDK 49. The installed Flatpak app
uses the ID `io.github.matt_jenner.mote` and receives photo-folder access only
through the desktop file chooser portal. Flatpak uses the underscore because
Flathub demangles it to the GitHub owner `matt-jenner`; Tauri rejects
underscores, so its macOS identifier is `io.github.matt-jenner.mote`.

## Host setup

On Omarchy or Arch Linux:

```bash
omarchy pkg add flatpak flatpak-builder
```

On Fedora:

```bash
sudo dnf install flatpak flatpak-builder
```

Add Flathub and install the build/runtime references for the current user:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive -y flathub \
  org.gnome.Platform//49 \
  org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.node24//25.08 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
```

The repository helper deliberately does not install packages, add remotes, or
install runtimes.

## Build and install

```bash
npm run flatpak -- check
npm run flatpak -- package
npm run flatpak -- install
npm run flatpak -- inspect
npm run flatpak -- run
```

For version 0.1.0 on the acceptance architecture, the bundle is written as
`dist/flatpak/Mote-0.1.0-x86_64.flatpak`. The helper derives later filenames
from the Tauri version and `flatpak --default-arch`.
Packaging keeps its builder directory and repository in a temporary Mote-owned
location and removes them when the command succeeds, fails, or is interrupted.
The previous bundle remains in place until its replacement is complete, and
only the latest versioned bundle is retained.
x86-64 is the current acceptance architecture; ARM builds and Flathub publication are deferred.
Build commands have no network access; `flatpak-builder` fetches only the
URL-and-checksum sources declared in the manifest before entering the build
sandbox.

## Install the bundle on Fedora

Copy the `.flatpak` file to Fedora, then run:

```bash
flatpak install --user ./Mote-0.1.0-x86_64.flatpak
flatpak run io.github.matt_jenner.mote
flatpak info --show-permissions io.github.matt_jenner.mote
```

The bundle records Flathub as its runtime source. Flatpak may offer to download
GNOME Platform 49 if Fedora does not already have it.

## Portal acceptance test

Run `npm run test:photos`, then choose `runtime/test-photos/demo-photos` in
Mote's system folder picker,
confirm that the photo wall and viewer load, quit Mote, and confirm the same
library works after relaunch. Repeat with a mounted NAS photo folder visible in
the system picker. Installed permissions must not contain `filesystems` or
`shared=network` entries.

## Refresh locked sources

The enabled manifest builds pinned libde265 followed by libheif. The libheif
source patch removes its built-in mask encoder, and a post-install runtime
probe requires exactly the libde265 decoder and zero encoders. Only versioned
shared libraries survive cleanup. Package creation runs loader and filesystem
inspection before replacing a bundle.

`npm run flatpak -- package --no-heic` renders a manifest with neither native
module and passes `--no-default-features --features mote-defaults` to Tauri.
The concrete JSON manifest lives in the managed temporary build directory and
is removed on success, failure, or interruption. YAML does not rely on host
environment expansion. The checked-in enabled manifest is the template.

Enabled builds install `THIRD_PARTY_NOTICES.md`, the full LGPL text, the two
dependency records, `HEIC-REBUILD.md`, and the literal `decode-only.cmake`
patch under
`/app/share/licenses/io.github.matt_jenner.mote/`. Because the compliance files
belong to the libheif module, `--no-heic` omits them with both native modules.
The hosted image installs the same set under `/usr/share/licenses/mote/` only
when HEIC is enabled. See the [native decoder source and replacement
guide](../heic/README.md) for source pins, patches, build switches, inspection,
and library replacement instructions.

After a Cargo lock or native pin change, refresh offline:

```bash
bash scripts/update-flatpak-sources.sh --offline
```

The refresh unions the root and desktop Cargo locks, verifies existing npm
archive entries against `package-lock.json`, and records all three lockfile
digests plus `packaging/heic/versions.env`. It refuses an incomplete npm cache.
Candidate sources are validated before the generated directory is replaced;
failure restores the old directory. `npm run clean:build` reaps stale source
refresh staging, Flatpak work directories, native builds and smoke runtime
directories while preserving committed locks, source media and release files.

If npm dependencies change, regenerate npm sources in an isolated checkout and
virtual environment:

```bash
generator_root="$(mktemp -d /tmp/mote-flatpak-generators.XXXXXX)"
git clone https://github.com/flatpak/flatpak-builder-tools.git "$generator_root/tools"
python3 -m venv "$generator_root/venv"
"$generator_root/venv/bin/pip" install "$generator_root/tools/node" tomlkit aiohttp
PATH="$generator_root/venv/bin:$PATH" scripts/update-flatpak-sources.sh "$generator_root/tools"
npm run test:flatpak
```

Commit both generated JSON files and `source-lock.json` with the lockfile change.

## Tagged releases

Publishing a GitHub Release whose tag exactly matches `v` plus the version in
`apps/desktop/src-tauri/tauri.conf.json` runs
`.github/workflows/release-flatpak.yml`. The workflow validates the tag before
installing the Flatpak runtimes, builds the bundle with the same non-interactive
helper used locally, and attaches the `.flatpak` to the existing GitHub Release.
It first builds and inspects the disabled variant in the runner's temporary
directory. Hosted CI runs enabled and disabled smoke tests on AMD64 and ARM64.
It has no manual trigger, so ordinary pushes and draft releases do not consume a
Flatpak build.
