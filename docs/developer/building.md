# Build Mote

Build desktop packages on their destination operating system. Mote's native
HEIC libraries and desktop bundles use platform toolchains and are not intended
for cross-compilation.

Run `npm ci` after cloning or pulling a lockfile change.

## HEIC build mode

Release builds include HEIC and HEIF decoding by default. Add one `--no-heic`
immediately after the npm separator to omit the native decoder while keeping
the other default features:

```bash
npm run desktop:build -- --no-heic
npm run desktop:build:windows -- --no-heic
npm run flatpak -- package --no-heic
./scripts/hosted-smoke.sh --no-heic
```

Enabled packages include the libheif and libde265 notices, LGPL text, verified
source pins, and replacement instructions. See the
[HEIC packaging reference](../../packaging/heic/README.md).

## Windows

Use 64-bit Windows 10 or 11. Install:

1. Visual Studio 2022 Build Tools with **Desktop development with C++**, x64
   MSVC tools, CMake, and a Windows SDK.
2. PowerShell 7 so `pwsh` is available.
3. Git, Rust 1.97.1, Node.js 24.18.0, and npm 11.16.0 or newer.

Open **x64 Native Tools Command Prompt for VS 2022**, move to the repository,
then start PowerShell 7. Verify the native tools:

```powershell
cl
dumpbin /?
cmake --version
rustc --version
node --version
npm --version
```

Build the installer:

```powershell
npm ci
npm run desktop:build:windows
```

The result is `dist/windows/Mote-<version>-windows-x64-setup.exe`. The builder
verifies the executable and decoder DLLs before replacing an existing package.
Windows packages are unsigned, so test SmartScreen approval and uninstall in a
clean virtual machine before a release.

## macOS

Install the Xcode Command Line Tools and both Rust targets:

```bash
xcode-select --install
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm ci
npm run desktop:build:universal
```

The result is `dist/macos/Mote.app`, with Apple Silicon and Intel executables.
Verify them with:

```bash
lipo -archs dist/macos/Mote.app/Contents/MacOS/photo-viewer-desktop
```

The output must include `x86_64` and `arm64`. The application has an ad-hoc
signature and is not notarized. Published releases use
`.github/workflows/build-macos.yml` to create and verify the disk image.

For a native app matching the current Mac only, run:

```bash
npm run desktop:build
```

## Linux

The supported package is an x86-64 Flatpak built with GNOME SDK 49. Install
Flatpak and Flatpak Builder, then follow the runtime setup in the
[Flatpak packaging reference](../../packaging/flatpak/README.md#host-setup).

Build, inspect, install, and run the package:

```bash
npm ci
npm run flatpak -- check
npm run flatpak -- package
npm run flatpak -- inspect
npm run flatpak -- install
npm run flatpak -- run
```

The result is `dist/flatpak/Mote-<version>-x86_64.flatpak`.

## Hosted image

Build with either supported engine:

```bash
podman build -t localhost/photo-viewer:dev -f Containerfile .
docker build -t photo-viewer:dev -f Containerfile .
```

Use [the hosted guide](../hosted/README.md) to mount photos and state. Run the
complete lifecycle check before changing deployment code:

```bash
./scripts/hosted-smoke.sh
CONTAINER_ENGINE=docker ./scripts/hosted-smoke.sh
```

## Cleanup

Build helpers remove their temporary Cargo and interface outputs. Direct Cargo
commands can leave large `target` directories. Remove known build outputs and
native download caches with:

```bash
npm run clean:build
```

The command does not touch source photos, application data, retained release
packages, or unrelated files in `dist`.

## Release packages

Set the version in `apps/desktop/src-tauri/tauri.conf.json`, then publish a
GitHub Release tagged with `v` and that exact version, such as `v0.1.0`.
Release workflows attach the Windows installer, macOS disk image, and Linux
Flatpak to the existing release. Ordinary pushes and draft releases do not run
the packaging workflows.
