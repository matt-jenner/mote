# Develop Mote

Mote combines a Rust workspace, a React and TypeScript interface, a Tauri
desktop shell, and a Rust web server. Native HEIC support also builds pinned C
and C++ dependencies.

## Prerequisites

Install these shared tools:

- Git;
- Node.js 24.18.0 or newer;
- npm 11.16.0 or newer;
- Rust 1.97.1 through rustup;
- Rustfmt and Clippy;
- CMake and the platform C or C++ compiler.

The checked-in `rust-toolchain.toml` selects the Rust version and components.
Package builds need additional platform tools listed in the
[build guide](building.md).

## Set up the checkout

```bash
git clone https://github.com/matt-jenner/mote.git
cd mote
npm ci
npm run test:photos
```

Install the browser used by interface tests:

```bash
# macOS
npm exec playwright install webkit

# Windows and Linux
npm exec playwright install chromium
```

On Arch Linux, tests use `/usr/bin/chromium` when it exists. Set
`MOTE_CHROMIUM_PATH` to choose a different system Chromium executable.

## Run Mote locally

Start the desktop application:

```bash
npm run desktop:dev
```

Use a named profile when testing a change that needs clean catalogue and cache
state:

```bash
PHOTO_VIEWER_PROFILE=clean-demo npm run desktop:dev
```

Choose `runtime/test-photos/demo-photos` in the folder picker for a generated
local source. Mote must treat this directory as read-only. Git ignores the
generated images, and production builds do not contain them.

Start only the hosted interface during frontend work:

```bash
npm run web:dev
```

The complete hosted application needs `photo-server` or the container image.
Use the [hosted guide](../hosted/README.md) for that workflow.

### Run the server directly

Build the hosted interface first. Then run `photo-server` with explicit source,
web, data, and cache directories:

```bash
npm run web:build
mkdir -p runtime/dev-data runtime/dev-cache
PHOTO_VIEWER_DATA_DIR="$PWD/runtime/dev-data" \
PHOTO_VIEWER_CACHE_DIR="$PWD/runtime/dev-cache" \
PHOTO_VIEWER_SOURCE_ROOT="$PWD/runtime/test-photos/demo-photos" \
PHOTO_VIEWER_WEB_ROOT="$PWD/apps/interface/dist" \
cargo run --release -p photo-server
```

PowerShell uses the same four paths:

```powershell
npm run web:build
New-Item -ItemType Directory -Force -Path runtime/dev-data, runtime/dev-cache
$env:PHOTO_VIEWER_DATA_DIR = (Resolve-Path "runtime/dev-data").Path
$env:PHOTO_VIEWER_CACHE_DIR = (Resolve-Path "runtime/dev-cache").Path
$env:PHOTO_VIEWER_SOURCE_ROOT = (Resolve-Path "runtime/test-photos/demo-photos").Path
$env:PHOTO_VIEWER_WEB_ROOT = (Resolve-Path "apps/interface/dist").Path
cargo run --release -p photo-server
```

The server binds to `127.0.0.1:8080` by default. Set `PHOTO_VIEWER_BIND` only
when another loopback address or port is required.

## Run checks

The focused interface checks are:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
```

Run the Rust verification suite with:

```bash
npm run rust:verify
```

The managed command uses a temporary Cargo target and removes it after the run.
CI tests HEIC-enabled packaging contracts, hosted containers on AMD64 and
ARM64, and Rust on Ubuntu, macOS, and Windows. The `--no-heic` option remains
available for local builds.

## Next references

- [Build desktop and hosted packages](building.md)
- [Debug Mote](debugging.md)
- [Repository architecture](architecture.md)
- [Contributor requirements](contributing.md)
- [Machine and agent instructions](../../AGENTS.md)
