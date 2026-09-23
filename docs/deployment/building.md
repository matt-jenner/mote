# Build Mote for Windows, macOS, and Linux

Mote uses the same locked Node, Rust, and HEIC source versions on every desktop
platform. Build on the operating system you intend to package. The native HEIC
libraries and desktop bundles use platform toolchains that are not suitable for
cross-compilation.

## Shared requirements

Use a reviewed checkout with these tools available:

- Git
- Node.js 24.18.0 and npm 11.16.0 or newer
- Rust 1.97.1 through rustup
- CMake and a platform C/C++ compiler

Install the JavaScript dependencies once after cloning or pulling:

```text
npm ci
```

HEIC and HEIF decoding is enabled by default. Each platform command accepts one
`--no-heic` immediately after the npm separator when a decoder-free package is
needed. Release workflows always build with HEIC enabled.

## Windows

### Prepare the build shell

Use Windows 10 or 11 x64. Install:

1. Visual Studio 2022 Build Tools with **Desktop development with C++**, MSVC
   x64 tools, CMake, and a Windows SDK.
2. PowerShell 7 so the `pwsh` command is available.
3. Rust 1.97.1, Node.js 24.18.0, npm 11.16.0, and Git.

Open **x64 Native Tools Command Prompt for VS 2022**, change to the repository,
then start PowerShell 7 with `pwsh`. Confirm the native tools before building:

```powershell
cl
dumpbin /?
cmake --version
rustc --version
node --version
npm --version
```

### Build the installer

```powershell
npm ci
npm run desktop:build:windows
```

The builder downloads and verifies the pinned libde265 and libheif sources,
builds x64 DLLs with MSVC, rejects encoder implementations and unexpected
runtime dependencies, and creates:

```text
dist/windows/Mote-0.1.0-windows-x64-setup.exe
```

The installer includes `heif.dll`, `libde265.dll`, and the LGPL compliance and
rebuild files. To verify the decoder-free branch without changing other default
features:

```powershell
npm run desktop:build:windows -- --no-heic
```

Windows packages are unsigned. SmartScreen may show **Windows protected your
PC**. For a build you created from your own reviewed checkout, choose **More
info**, confirm the app name is Mote, then choose **Run anyway**.

### VM acceptance check

1. Run the HEIC-enabled installer and launch Mote from the Start menu.
2. Add a folder containing JPEG and iPhone HEIC or HEIF photos. Confirm every
   supported file is counted and receives a thumbnail.
3. Open an HEIC photo, quit Mote, relaunch it, and confirm the folder and cached
   image still work.
4. Uninstall Mote from **Settings > Apps > Installed apps** and confirm the app
   installation directory is removed. User catalog and cache data may remain so
   an uninstall does not silently destroy a library index.

### Windows cleanup

The builder keeps only the accepted installer, the accepted native decoder
prefix, and two checksum-named source archives. It removes the temporary Cargo
target and generated interface on success or failure. To remove the reusable
native prefix and downloads after testing:

```powershell
Remove-Item -Recurse -Force build/heic-native
```

Delete a retained installer separately only when it is no longer needed:

```powershell
Remove-Item -Force dist/windows/Mote-*-windows-x64-setup.exe
```

## macOS

Install the Xcode Command Line Tools, then add both Rust targets for a universal
Apple Silicon and Intel build:

```bash
xcode-select --install
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm ci
npm run desktop:build:universal
```

The output is `dist/macos/Mote.app`. The build stages pinned universal HEIC
libraries, verifies both slices, ad-hoc signs the nested libraries and app, and
removes its Cargo target. Check the executable architectures with:

```bash
lipo -archs dist/macos/Mote.app/Contents/MacOS/photo-viewer-desktop
```

Use `npm run desktop:build:universal -- --no-heic` for the decoder-free branch.
The repository [README](../../README.md#run-the-macos-desktop-app) includes the
local DMG command. Published releases use the verified
[macOS release workflow](../../.github/workflows/build-macos.yml) and attach
`Mote-<version>-macOS.dmg`.

Remove reusable build downloads and known intermediates without touching the
retained app or DMG:

```bash
npm run clean:build
```

## Linux

The supported desktop package is an x86-64 Flatpak built with GNOME SDK 49.
Install Flatpak and Flatpak Builder using your distribution's package manager,
then add Flathub and the locked runtimes listed in the
[Flatpak guide](../../packaging/flatpak/README.md#host-setup).

Build and inspect the bundle:

```bash
npm ci
npm run flatpak -- check
npm run flatpak -- package
npm run flatpak -- inspect
```

The current output is `dist/flatpak/Mote-0.1.0-x86_64.flatpak`. Build the
decoder-free branch with `npm run flatpak -- package --no-heic`. Install and run
the accepted bundle with:

```bash
npm run flatpak -- install
npm run flatpak -- run
```

The Flatpak helper removes temporary build directories on success, failure, or
interruption and retains only the latest bundle. `npm run clean:build` removes
known repository intermediates, native downloads, and stale smoke-test assets.
It does not remove the retained `.flatpak` bundle, application data, cache data,
or source photos.

## Tagged releases

Set the version in `apps/desktop/src-tauri/tauri.conf.json`, then publish a
GitHub Release tagged with `v` plus that exact version, such as `v0.1.0`. The
release workflows build Windows, macOS, and Linux packages and attach them to
the existing release. Draft releases and ordinary pushes do not start package
builds. Manual Windows and macOS checks keep their artifacts for one day.

Repository Actions are intentionally left disabled until a deliberate remote
test or release is ready. Enable Actions immediately before that test, then use
manual dispatch for the platform being checked.
