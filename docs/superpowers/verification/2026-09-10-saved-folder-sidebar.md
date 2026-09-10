# Saved folder sidebar verification

Date: 2026-09-10
Branch: `codex/saved-folder-sidebar`
Design: [approved specification](../specs/2026-09-10-saved-folder-sidebar-design.md)

## Implemented

- Flat naturally sorted shortcuts, default folder names, inline labels, duplicate
  reuse, native/root-relative tooltips, and active/inactive removal behaviour.
- SQLite desktop persistence and migration from the previous active folder.
- Root-namespaced browser storage, independent tab selection, shared list edits,
  legacy migration, storage failure handling and browser profile isolation.
- Shared access checks with five-second result reuse and caller deadlines,
  bounded workers/keys/waiters, and retained in-flight slots after timeout.
- Selection intent and native snapshot revisions prevent delayed work from
  restoring removed entries or superseded views.
- Unavailable rows, manual rechecks, cached-image warning badges, and empty
  startup when the previous folder cannot be accessed.
- Wall and viewer queries include the decoder-supported JPEG, PNG, TIFF and
  WebP formats. RAW/DNG, HEIF and AVIF files, plus supported files with a
  terminal thumbnail decode failure, are omitted from both sequences.
- The wall status ignores retained warnings for filtered assets while still
  reporting warnings attached to visible items.
- Option 3 sidebar geometry, labelled narrow-screen drawer, keyboard menus,
  rename focus, sticky folder header and menus that open upwards near the bottom.

The hosted access route lives in `api/gallery.rs`, browser storage lives under
`folders/`, and the row/menu are contained in `SavedFolderList.tsx`. Warning
presentation is shared through `SourceAvailabilityContext.tsx`. These consolidate
related pieces from the plan's illustrative file breakdown.

## Automated evidence

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --workspace --all-features --quiet` | 519 passed |
| Desktop `cargo fmt -- --check` and `cargo check` | Passed |
| `npm run check` | Passed |
| `npm run typecheck` | Passed |
| `npm test` | 191 passed in 20 files |
| Vitest browser, motion and contrast projects | 191 passed in 6 files |
| Final sidebar browser suite after adding long-list regression | 18 passed; includes one additional test |
| Interface production build and hosted build | Passed |
| Hosted Playwright before restart | 3 passed |
| Hosted Playwright after restart | 1 passed; 2 phase-specific skips |
| Hosted Playwright offline | 1 passed; 2 phase-specific skips |
| `git diff --check` | Passed |

Hosted verification used the same A/B/nested-folder fixture layout as
`scripts/hosted-smoke.sh`, served by the native `photo-server` binary on a
loopback port. It exercised two browser contexts and two tabs, label sorting,
refresh persistence, cross-tab changes and active removal, selection isolation,
server restart, stable cached derivatives, and unavailable startup. SHA-256
hashes and modification times of all seven fixture photos were unchanged after
each phase. The container build and container mount wrapper were not rerun.

The first repeat of the Rust suite exposed an existing test-ordering race:
automatic prefetch could satisfy a gate before the explicit request registered.
The regression fixture now holds automatic collection enqueue and bounds the
request wait. The focused test and complete workspace suite then passed.

Read-only review identified and verified fixes for native response ordering,
late browser restoration, removal during selection, root identity changes,
transient migration/storage failures, and the shared root/child check deadline.

## Visual and interaction checks

The in-app browser displayed the real hosted application with copied demo
photos. Desktop light/dark at 1440px, tablet at 768px and phone at 390px were
inspected. Screenshots were emitted in the task conversation. The sidebar,
52px rows, drawer, inline rename, phone menu, tooltips and focus restoration were
checked against option 3 while retaining the existing justified photo wall.

A disposable folder was made inaccessible while open: the cached wall remained
usable with amber corner badges. Reloading cleared the active view and retained
the unavailable shortcut; its menu offered Remove only. Removing it kept the
remaining shortcut and moved focus there. Browser tests also cover viewer
navigation, warning hit targets, uncached placeholders and accessibility.

This validation ran on macOS. The release `Mote.app` bundle was built and
launched; Windows and Linux desktop UI were not launched.
