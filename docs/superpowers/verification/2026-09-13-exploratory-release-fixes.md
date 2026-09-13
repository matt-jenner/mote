# Exploratory release fixes verification

Date: 2026-09-13

Integration branch: `codex/exploratory-release-fixes`

Verified range: `6d20a4c..e4b2e40` plus integration merge commits

## Automated verification

| Area | Command | Result |
| --- | --- | --- |
| Formatting and lint | `npm run check` | Pass; 108 files checked |
| TypeScript | `npm run typecheck` | Pass |
| Interface unit | `npm test` | Pass; 24 files, 268 tests |
| Interface browser | `npm run test:browser` | Pass; 9 files, 257 tests |
| Rust workspace and desktop | `npm run rust:verify` | Pass; formatting, Clippy with warnings denied, workspace/native tests, doc tests, and 10,000-photo benchmark smoke |
| Brand assets | `npm run brand:test` | Pass; 30 tests |
| Hosted deployment | `npm run test:deployment` | Pass; 9 tests |
| Flatpak packaging | `npm run test:flatpak` | Pass; 21 tests |
| Hosted web build | `npm run web:build` | Pass; 1,909 modules transformed |
| Native macOS build | `npm run desktop:build` | Pass; release `Mote.app` produced |

The first hosted deployment run was blocked from the local Podman socket by the
workspace sandbox. The unchanged command passed 9/9 when rerun with local
socket permission. The browser suite retained its known React `act(...)`
warning in `ViewerStage`; it did not fail or hide an assertion failure.

## macOS artifact

The generated bundle was ad hoc signed after Tauri's build, then passed
`codesign --verify --deep --strict --verbose=2`. Its identifier is
`io.github.matt-jenner.mote`, its executable is a Mach-O 64-bit arm64 binary,
and the executable SHA-256 is
`603e8efa5aa5ac471c9a9f48ea3d19087032232ac7578b907ce44e08540d4e6c`.

The release feature set emits one non-failing dead-code warning for
`GalleryEngine::cancel_runtime_scan`, which is exercised by the test feature
set. This is not a runtime failure or a rejected Clippy warning.

## Live macOS smoke test

The signed app launched with the two existing saved folders `2024` and `2025`.
It selected the naturally first folder (`2024`) instead of the welcome screen,
showed cached photos immediately, and kept the toolbar to one row with icon-only
scope and sort controls.

Switching to `2025` showed cached previews immediately. Its indexing count
advanced from 41 to 96 while the folder was inactive, proving background
continuity; after returning and scrolling, it advanced from 96 to 130 without
changing to a warning state. The active `2024` folder also advanced while
selected.

Adding a loaded photo to Picks showed `Added to picks`, and the toast faded by
the one-second boundary. The destination picker opened directly. A copy into an
isolated Downloads folder produced one complete 2,250,989-byte file with mode
`0600` and no `.mote-copy-*` debris. Clearing Picks returned the drawer to its
empty state and removed the previous copy result.

The isolated smoke-test folder was moved to Trash after inspection. The
cancellation and destination-deletion paths were not forced against the user's
live photo library; deterministic native tests cover cancellation at the first
and later files, partial cleanup, retained completed files, destination loss,
path replacement, competing readers, and competing publication.

## Reviewed limitations

- Atomic publication rejects deterministic path replacement, but a narrow
  adversarial path-substitution window remains between identity validation and
  the final path-based hard-link system call.
- macOS, Linux, Redox, and Windows have exclusive no-replace publication;
  unsupported targets retain the safe hard-link fallback, and filesystems that
  reject exclusive publication fail without exposing a partial final file.
- One in-flight original-copy preparation call is bounded to 250 IDs but is not
  pre-emptible; cancellation is checked before and after preparation, between
  chunks, and before copying.
- The inventory filesystem walk is not fully scheduler-priority-bounded, while
  derivative, shape, and metadata work is priority-bounded.
- A dropped asynchronous copy command during read-only preparation can retain
  preparation work briefly; terminal and cancellation paths wait for owned work.
- Ordinary-motion toast fading is covered by controller/CSS tests and live
  smoke, while the dedicated browser assertion targets reduced motion.
- Startup race coverage uses a bounded absence assertion rather than a separate
  completion signal.

These limitations were judged non-blocking by the independent workstream
reviews and do not relax source read-only guarantees or final-file atomicity.

Final combined review also caught and fixed three release-blocking integration
seams before this verification: stale photos surviving a newer settled scan,
current-generation streamed previews being dropped by paged settlement, and an
older settlement clearing newer-generation stream tracking. Deterministic
reducer coverage now exercises both the paged and out-of-order generation paths.
