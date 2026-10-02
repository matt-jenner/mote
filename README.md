# Mote

Mote is a fast, simple photo viewer for all those many nested photo folders you already have. 
Point it at a local directory, external drive, NAS mount, or server share and browse the
collection without importing, rearranging, or uploading the originals.

It was built to fit a particular need my wife had, looking through our photo archives to select
photos for photobooks or frames - it's also a really nice way to quickly show people your photos.

Mote builds a local catalogue and creates its own thumbnails and previews. Your
photo folders stay read-only, it's a totally non-destructive view only experience.

## Why use Mote?

- Keep your existing folder layout. No opinionated library folder structure or migration.
- Browse large collections while indexing continues in the background.
- You want to view ALL the photos in nested folders at once.
- View JPEG, PNG, WebP, TIFF, and HEIC or HEIF photos.
- Collect photos from several folders in Picks, then copy the originals on the
  desktop without overwriting existing files.
- Run the same interface as a desktop app or a private hosted service for browser based mobile
or desktop browsing.

Mote is a good fit for photo archives on disks and network shares. It is not a
cloud-sync service, editor, or replacement for your existing files.

## Choose a version

| Version | Best for | How it works |
| --- | --- | --- |
| Windows | Windows 10 or 11 | Native x64 installer with local folder access |
| macOS | Apple Silicon and Intel Macs | Universal app with the macOS folder picker |
| Linux | Linux desktops | Sandboxed x86-64 Flatpak with portal-based folder access |
| Hosted | A group sharing one photo archive | Web interface backed by a Podman or Docker container |

All four versions use the same gallery and viewer. Desktop versions let each
person choose folders from the machine. The hosted version exposes only the
read-only photo root selected by its administrator.

_NOTE: the hosted version is NOT suitable for direct exposure to the internet, local networks only!_

## Get started

1. Follow the [installation guide](docs/install/README.md) for your version.
2. Open Mote and add a photo folder.
3. Read the [user guide](docs/user-guide.md) for browsing, Picks, viewer
   controls, and offline folders.

The current release files are on
[GitHub Releases](https://github.com/matt-jenner/mote/releases). Windows and
macOS packages are not code-signed by a paid developer certificate, so both
platforms may ask you to confirm the first launch. The installation pages
explain what to expect.

## Documentation

- [Documentation home](docs/README.md)
- [Install Mote](docs/install/README.md)
- [User guide](docs/user-guide.md)
- [Host Mote with Podman or Docker](docs/hosted/README.md)
- [Developer guide](docs/developer/README.md)
- [Contributing and pull requests](docs/developer/contributing.md)

## Safety and privacy

Mote never writes, renames, or deletes files inside a configured photo source.
Its SQLite catalogue and generated previews live in separate data and cache
locations. The hosted version does not send native server paths to the browser,
and original-file downloads are disabled unless the administrator enables
them.

Mote is an early open-source project. Back up irreplaceable photos as you would
with any photo tool, and download packaged builds only from this repository's
official release page.

## Development

The [developer section](docs/developer/README.md) covers prerequisites, local
setup, platform builds, tests, debugging, architecture, and release packaging.
If you plan to send a change, start with the
[contributor guide](docs/developer/contributing.md).
