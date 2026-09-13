# Exploratory release fixes design

Date: 2026-09-13

Status: approved in conversation; pending written-spec review

## Purpose

Resolve the issues found during exploratory testing of saved folders, progressive
indexing, the photo-wall toolbar, persistent picks, and desktop original copying.
The result is a release-candidate macOS build on `main` with predictable startup,
continuous indexing, compact controls and status, and a responsive, cancellable,
atomic copy flow.

This design supersedes conflicting interaction details in the saved-folder and
persistent-picks specifications. In particular, cached previews may be shown
while access is checked, copy progress appears only in the Picks drawer, and a
running copy replaces Clear Picks with Cancel.

## Agreed outcomes

### Welcome and toolbar

1. **Welcome action contrast.** The full-bright accent button on the initial
   welcome screen chooses black or white text according to which has the higher
   WCAG contrast against the configured accent, with a minimum 4.5:1 ratio.
   Keep that exact accent fill on hover and use a non-fill hover cue so the
   calculated contrast remains valid throughout the interaction; do not change
   other primary buttons.
2. **No artificial preview pause.** Selecting a newly added folder starts
   publishing cached or discovered items and indexing progress immediately.
   Folder access checks run without blanking the wall or imposing the current
   multi-second gate before useful content appears.
3. **No `Folder ready` status.** Remove the intermediate copy from the wall
   and first-run canvas. A selected folder moves directly to cached content,
   `Indexing`, an unavailable state, or its settled wall.
4. **One toolbar row.** At supported desktop widths, the wall controls and
   status remain on a single non-wrapping row. Constrained space is handled by
   concise status copy, icon controls, and intentional flex shrinking rather
   than a second toolbar row.
5. **Icon-only sorting.** Replace the `Oldest first` and `Newest first` text
   controls with their existing icons. Each control has an accessible name,
   visible selected state, and a tooltip on pointer hover and keyboard focus.
6. **Icon-only subfolder scope.** Replace the `Include subfolders` label with
   an icon toggle that exposes `aria-pressed`, a visible selected/unselected
   state, and a hover/focus tooltip.
7. **Concise indexing copy.** Report determinate progress as
   `Indexing - {indexed} of {total}`. If the total is not yet known, report only
   `Indexing`; do not add `photos`, `files`, `indexed`, or `Folder ready`.

### Folder restoration, cached previews, and indexing

8. **Deterministic desktop startup.** On every desktop launch with saved
   folders, select the first folder in the same natural, case-insensitive order
   shown in the sidebar instead of restoring an arbitrary or stale active ID.
   Select it immediately, before access validation, so its cached previews and
   eventual unavailable state remain visible. The first-run welcome is reserved
   for an actually empty saved-folder list.
9. **Cached wall first, validation second.** Hydrate the selected folder's
   requested cached catalog pages and usable derivatives without waiting for
   filesystem reconciliation. Cached derivatives remain visible while source
   access is validated; validation updates warnings in place and never replaces
   the wall with only a fixed first page.
10. **Root-first access checks.** Check the configured library root and then
    the selected folder, when it is a child, before any per-file access work.
    If either prerequisite cannot be opened, immediately project an unavailable
    warning onto every photo in the affected scope and skip all per-file checks;
    a library-root failure affects every folder in that library, while a child
    failure affects that saved folder. If both succeed, begin asynchronous
    per-file checks and apply only item-specific warnings.
11. **Per-folder indexing jobs.** Key indexing and derivative work by stable
    saved-folder identity rather than by the single active selection. Switching
    folders reprioritises the old job to background work instead of cancelling
    it, while the newly active folder receives foreground priority.
12. **Bounded priority.** Reserve the most responsive queue capacity for the
    active folder and process inactive saved folders at lower concurrency.
    Background work must continue and eventually finish without competing with
    active scrolling for every worker slot; duplicate requests for the same
    folder or asset coalesce.
13. **Scrolling cannot end indexing.** Loading more wall rows or encountering
    individual derivative warnings does not replace an active folder's
    indexing phase with `Some previews need attention`. Keep progress
    authoritative until indexing settles, merge metadata-settlement results by
    asset identity, and retain all already loaded pages and previews.

### Picks and desktop copy

14. **Only pick usable tiles.** Hide the add-to-picks control until a tile has
    a usable, interactive preview. An already picked item keeps its remove
    control even if its source or derivative later becomes unavailable.
15. **Fading add confirmation.** The complete `Added to picks` toast lifetime,
    including its final fade, is at most one second. Adding another photo
    immediately restarts both the visible interval and fade sequence;
    persistence latency must not delay that reset. Reduced-motion mode removes
    the animation but preserves the same lifetime and announcement.
16. **Immediate destination picker.** `Choose destination` opens the native
    folder dialog directly, without a preparatory tooltip or synchronous batch
    scan. Resolve and validate the immutable copy batch after the user chooses
    a destination, reporting any unavailable originals through the normal
    result model.
17. **Drawer-owned copy progress.** Remove copy progress from the top toolbar.
    The open Picks drawer or sheet shows a determinate progress bar across its
    available width, with compact right-aligned `X of N` text, and continues to
    reflect the operation if the panel is closed and reopened.
18. **Atomic, reader-tolerant files.** Copy each original to a uniquely named
    temporary file in the destination directory, flush and close it, then
    atomically rename it to the collision-resolved final filename. Finder,
    Preview, or another reader can therefore observe only a complete final
    file, never a partially written one.
19. **Destination disappearance.** Validate the destination before starting
    and before each file, and classify directory-not-found failures that occur
    during a copy or final rename as `The destination folder no longer exists.`
    Stop the remaining batch, remove the current temporary file, retain all
    completed final files, and keep the picks.
20. **Explicit cancellation.** While copying, replace Clear Picks with Cancel.
    Cancellation is cooperative and checked during the current file as well as
    between files; it removes only the current temporary file, retains every
    completed final file, stops the remaining batch, and restores the drawer to
    the exact pre-attempt copy state with the picks unchanged. While native
    cleanup drains, keep the Cancel control disabled as `Cancelling...` and do
    not expose another copy attempt.
21. **Toast-only cancellation message.** After cancellation, show a quick
    fading `Copy cancelled` toast using the same at-most-one-second lifetime;
    do not persist that phrase or a cancelled result in the drawer.
22. **Clearing removes stale copy state.** After a completed or partially
    completed operation, Clear Picks also clears copy results, row errors, and
    completion copy such as `Copied 6 originals`. The empty drawer contains no
    history from the cleared selection.

## Architecture

### Interface state

Keep folder indexing state separate from the currently rendered selection.
The interface stores per-folder summaries keyed by saved-folder ID, while the
wall subscribes to the selected folder and projects its phase, counts, pages,
and warnings. A selection change detaches only the view subscription; it does
not destroy the folder's job.

Startup selection uses one shared ordering function with the sidebar. Desktop
bootstrap returns the saved list and selected cached wall without waiting for
an access check and without using a transient `Checking` state to clear the
selection. It then schedules one background verification. A confirmed
inaccessible result updates warnings, while timeout, checking, invalid, and
indeterminate failures leave the selection and cached state intact as
unverified. Root and per-file validation events update folder and asset
availability independently of catalog hydration.

Wall reducer updates are monotonic by asset ID. `metadataSettled` and later
page results patch or append matching items and preserve items that are absent
from that event. Warning totals may be shown after indexing, but a warning
cannot force the indexing phase to settle early.

The welcome button gets a local style modifier. The existing accent-token
calculation remains the single source for adaptive black/white foreground; the
modifier prevents hover from changing the fill after that foreground has been
calculated.

### Indexing scheduler

Extend the application service from one cancellable active scan to a registry
of saved-folder jobs. Each job owns its scan generation, derivative queue,
progress, and terminal result. Selection changes update priorities atomically:
the selected folder becomes foreground and prior work becomes background.

Foreground requests use the responsive worker budget. Background jobs use a
smaller bounded budget and round-robin scheduling so every saved folder makes
progress. An asset request generated by visible scrolling is promoted within
the active folder but does not cancel the folder scan or reset its generation.
Job and asset keys coalesce duplicate work, and stale events are rejected by
folder ID plus generation rather than by the current selection alone.

### Copy operation

Split destination selection from batch preparation. The native command first
opens the picker; after a directory is chosen, the service resolves the current
immutable pick IDs, validates the destination, reserves collision-safe final
names, and starts one operation identified by a copy-operation ID.

The copy service publishes progress and owns a cancellation token. Each file is
streamed in chunks to a destination-local temporary filename that cannot clash
with user files. Cancellation is checked between chunks. Success requires
flush/close followed by an atomic same-directory no-replace publication to the
reserved final name. A competing filename causes collision resolution and a
retry, never an overwrite. Every error path attempts to remove the temporary
file but never removes a completed final file.

The UI treats `idle`, `running`, `success`, and `partial failure` as drawer
states. It snapshots the pre-attempt state before a run. Explicit cancellation
restores that exact snapshot after native cleanup and emits the ephemeral toast;
dismissing the destination picker is silent. A cancelled run does not update
the remembered destination, even if an earlier file completed. Clear Picks
resets the selection and all non-running copy presentation state; it is
unavailable while a run is active because Cancel occupies that action.

## Error handling and race rules

- Folder checks carry folder identity and generation. Late results can update
  their own saved folder but cannot switch the current view or overwrite a
  newer generation.
- A confirmed configured-root failure is authoritative for that check
  generation: mark every asset in that library unavailable and issue no child
  or file probes. A confirmed selected-child failure marks that folder and also
  skips file probes. A later successful prerequisite check clears the matching
  folder-wide projection before per-file results are applied.
- Individual file or derivative failures remain per-asset warnings and do not
  cancel indexing or hide other cached previews.
- Destination validation distinguishes a missing directory from permissions,
  source-containment rejection, and individual source failures. Only a missing
  destination uses the exact approved message.
- Cancellation and destination loss win over scheduling the next file. If they
  race with final rename, a successfully completed rename counts as completed;
  only a still-temporary file is removed.
- Explicit in-progress cancellation and destination-picker dismissal use
  distinct outcomes so only the former emits `Copy cancelled`.
- Only one desktop copy operation may run at a time. A stale progress or
  completion event with an old operation ID cannot modify a newer run.

## Parallel delivery and integration

Implementation uses isolated Git worktrees and feature branches, one workspace
per independently owned workstream:

1. **Library continuity:** deterministic top-folder selection, cached hydration,
   root-first access, asynchronous per-file warnings, keyed foreground/background
   jobs, scroll-safe generations, and merge-preserving reducer behavior.
2. **Toolbar and welcome UX:** local adaptive welcome button, single-row toolbar,
   icon toggles, and concise status.
3. **Picks and desktop copy:** pick-control visibility, resettable toast, immediate
   picker, drawer progress, atomic temporary files, destination loss,
   cancellation, and copy-state clearing.

Each workstream starts from the committed integration specification, uses
tests-first changes, and receives requirements and code-quality review before
integration. The integration owner merges workstreams sequentially, resolves
cross-stream conflicts, runs the full verification matrix, builds the macOS
application, performs a smoke test, and only then merges the verified
integration branch into `main`.

## Verification

### Automated

- Add browser tests for light and dark custom accents, including `#777777`,
  verifying at least 4.5:1 contrast at rest and hover only on the welcome
  action.
- Add toolbar geometry tests at supported desktop breakpoints, plus accessible
  names, `aria-pressed`, selected styling, and hover/focus tooltips for the
  icon-only controls.
- Add startup tests with no folders, several naturally sorted folders, stale
  active IDs, an inaccessible top entry, and all entries inaccessible. The
  displayed top entry remains selected while its cached wall is flagged.
- Add service and reducer tests proving cached rows appear before access
  completion, confirmed library or selected-folder failure skips downstream
  probes, indeterminate checks preserve cached state, successful prerequisites
  start per-file work, and settlement/page events never discard loaded items.
- Add scheduler tests proving switching A to B does not cancel A, B receives
  foreground priority, A continues at bounded lower concurrency, duplicate
  jobs coalesce, and scroll-driven promotion cannot end indexing.
- Add tile and Picks tests for preview-gated add controls, removal of stale
  picks, one-second resettable fading toasts, reduced motion, drawer-only
  progress, Cancel/Clear replacement, and clearing stale result state.
- Add native copy tests for immediate picker ordering, collision handling,
  exact-byte results, readers observing only finalized files, destination
  deletion before and during files, cancellation during a large file,
  temporary-file cleanup, retained completed files, exact pre-attempt state
  restoration, silent picker dismissal, idempotent cancellation, and stale
  operation events.
- Run the existing interface, Rust workspace, desktop adapter, packaging, and
  source-safety suites to catch regressions in hosted behavior and read-only
  source guarantees.

### Manual macOS smoke test

Build and ad hoc sign a fresh arm64 Mote application from the integrated
`main`. Launch it with two previously indexed saved folders and verify the
topmost folder, immediate cached previews, background progress after switching,
single-row toolbar, accessible icon states, smooth scrolling, and warning
projection after simulated source loss. Exercise add-to-picks feedback, normal
copy, destination browsing during copy, cancellation, and destination deletion;
inspect the destination for complete finalized files and no temporary debris.

## Out of scope

- Changing global accent behavior or the appearance of unrelated primary buttons
- Reordering saved folders beyond the existing natural display order
- User-configurable indexing priority or concurrency controls
- Persisting access-check results as durable truth across launches
- Resuming an interrupted copy after app termination
- Rolling back files that completed before cancellation or another copy error
- Copy history after picks are cleared
- Hosted bulk-copy or ZIP behavior
