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

Choose `apps/interface/public/demo-photos` in Mote's system folder picker,
confirm that the photo wall and viewer load, quit Mote, and confirm the same
library works after relaunch. Repeat with a mounted NAS photo folder visible in
the system picker. Installed permissions must not contain `filesystems` or
`shared=network` entries.

## Refresh locked sources

After either lockfile changes, use an isolated checkout and virtual environment:

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
It has no manual trigger, so ordinary pushes and draft releases do not consume a
Flatpak build.
