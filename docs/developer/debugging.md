# Debug Mote

Start with a reproducible source, a clean profile, and the narrowest failing
test. Do not diagnose against irreplaceable photos when the checked-in demo
folder can reproduce the problem.

## Use an isolated desktop profile

```bash
PHOTO_VIEWER_PROFILE=debug-session npm run desktop:dev
```

On macOS, named profile state lives below:

```text
~/Library/Application Support/io.github.matt-jenner.mote/profiles/<profile>/catalog.sqlite
~/Library/Caches/io.github.matt-jenner.mote/profiles/<profile>/
```

Choose a new profile name for a clean run. Do not delete or modify a user's
existing catalogue while investigating an unrelated failure.

## Narrow the failing layer

| Symptom | First check |
| --- | --- |
| Layout, control, or browser behaviour | `npm test` and `npm run test:browser` |
| Type error or stale contract | `npm run typecheck` |
| Rust service or catalogue behaviour | Run the relevant `cargo test -p <crate>` |
| Desktop packaging | `npm run test:build` and the platform build command |
| Linux packaging | `npm run test:flatpak` |
| Hosted storage or restart | `npm run test:deployment` and `./scripts/hosted-smoke.sh` |

Run the complete Rust suite only after the focused test passes:

```bash
npm run rust:verify
```

## Inspect the hosted service

Check health before reading a large log:

```bash
curl --fail --silent --show-error http://127.0.0.1:8080/healthz
podman logs photo-viewer
```

An `unhealthy` response usually means the catalogue or cache is unavailable or
unwritable. A `degraded` response points to a disconnected photo source or an
active source warning. Confirm the host paths and UID or GID permissions in the
[hosted guide](../hosted/README.md#storage-and-permissions).

## Test without HEIC

Use the supported flag instead of editing Cargo features by hand:

```bash
npm run desktop:build -- --no-heic
npm run desktop:build:windows -- --no-heic
npm run flatpak -- package --no-heic
./scripts/hosted-smoke.sh --no-heic
```

If only the enabled build fails, record the platform, architecture, compiler,
CMake version, and output from the native dependency verification script.

## Report a useful bug

Include:

- operating system, architecture, and Mote version or commit;
- desktop or hosted variant and the exact launch command;
- the smallest sequence that reproduces the problem;
- expected and actual behaviour;
- relevant terminal output, health response, or screenshot with private paths
  and filenames removed.

For source-specific bugs, state the file format and whether a copy of the file
can be shared privately. Do not attach personal photos to a public issue.
