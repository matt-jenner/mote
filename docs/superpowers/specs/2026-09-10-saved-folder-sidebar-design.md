# Saved folder sidebar design

Date: 2026-09-10

Status: approved in conversation, awaiting written-spec review

## Purpose

Let people keep a labelled list of folders in the left sidebar and switch
between their photo collections without reopening the folder picker.

Desktop saves the list across app restarts. Hosted web saves it per browser
profile and site in localStorage. This replaces the initial session-only web
requirement. No user accounts or server-side personal folder lists are needed.

Source media remains read-only. Renaming changes a shortcut label, and removal
deletes a shortcut. Neither operation renames or deletes folders, photos,
catalog records, or cached derivatives.

## Agreed interaction

- Show a flat list under Folders, with an Add folder button that uses the
  existing native picker or contained hosted folder browser.
- Successfully opening a folder adds an entry and displays its images.
  Cancelling or failing to open a new folder adds nothing.
- Opening an already saved folder activates its existing entry and preserves
  its custom label. A folder has one entry per saved list.
- Clicking an available entry loads its images and highlights it as active.
  Normal selection retains the existing gallery scope and sort preferences.
- Default the label to the folder name. For the hosted root, use the existing
  root breadcrumb display name.
- An entry menu provides Rename and Remove. Rename edits the label inline.
  Enter saves, Escape cancels, and leaving the editor cancels an uncommitted
  edit. Trim surrounding whitespace; saving an empty label removes the custom
  label and restores the folder name. Render labels as plain text.
- Sort by the displayed label, ignoring case and using natural numeric order:
  Album 2 precedes Album 10. Re-sort on save, retaining focus on the renamed
  entry. Different folders may share a label; use path and stable entry ID as
  deterministic tie-breakers.
- Hover and keyboard focus expose the full desktop display path or hosted
  root-relative path. Never expose the server's absolute filesystem path.
  Represent the hosted root itself as `/` in the tooltip.
- Removing an inactive entry leaves the current view unchanged. Removing the
  active entry closes any viewer, clears the wall and active selection, and
  shows the existing empty state. The user selects another entry or opens a
  new folder. Removal does not require confirmation.
- Narrow screens use the existing navigation drawer for the same list.
  Selecting a folder closes the drawer after successful activation. Long
  lists scroll and long labels truncate without hiding the entry menu.

## Unavailable folders and cached images

Unavailable entries remain saved, display a warning symbol, and use muted
styling. Their menu permits Remove only. They cannot be renamed or opened
until access succeeds.

The row remains keyboard focusable and accepts pointer or keyboard activation
as a request to recheck access. It must not be a native disabled button that
discards those events. Its accessible description explains that the folder is
unavailable and activation checks again. The warning tooltip includes the
path and a concise reason. A check shows progress without disabling removal.

If a click-triggered check succeeds, open the folder only while that selection
intent remains current. Automatic startup or focus checks update availability
without switching the user's current folder. A failed check leaves the entry
unavailable. Rapid activations do not launch overlapping checks.

If the active folder becomes unavailable during viewing, retain its current
wall and viewer state. Show a warning badge in a corner of each affected wall
thumbnail and an equivalent warning on the currently viewed image. The
tooltip for a usable cached image reads: "Source unavailable. Showing cached
image." Keep usable cached derivatives viewable under the existing zoom and
navigation rules. Items without usable cached content show a missing-image
placeholder and cannot open. Do not fetch original files as a fallback.

This continued viewing applies only to the folder already open when access
was lost. After switching away, returning requires a successful availability
check, even if cached images remain. Removing the active entry still clears
the view. On a fresh app or page load, an unavailable last-active folder does
not reopen from cache; the saved entry remains visible with its warning.

An individual missing or corrupt image does not make the entire folder
unavailable. Existing per-asset failure handling remains in effect.

## Storage and identity

Keep saved shortcuts separate from catalog libraries and folder groups. A
shortcut contains a stable entry ID, stable folder reference, default folder
name, path suitable for display, and an optional custom label. Availability
and in-progress checks are runtime state rather than durable promises of
access. A transient selection epoch must not be the shortcut's identity.

### Desktop

Add a saved-folder repository and schema migration in photo-catalog. Store
entries in the existing SQLite database, naturally scoped to the app profile.
Use the catalog's library and relative-folder identity to deduplicate saved
folders and reopen them without invoking the picker. Preserve native path
encoding for filesystem operations and use a separate display string for UI.
Keep existing source-containment and overlapping-source rules.

Reuse the desktop active-selection mechanism. Removing its saved entry and
clearing active selection must be atomic in storage. A service transition
must invalidate pending selection work and detach the old wall subscription,
so scan events cannot restore removed content.

On first migration, seed an entry for the previously active folder if present.
Do not turn every historical catalog root into a visible shortcut. Removing
and later reopening a folder creates a default-labelled entry while reusing
the catalog and caches where the existing engine permits it.
Gate startup selection restoration and its scan startup on the access result;
the existing automatic desktop reconciliation must not bypass this check.

### Hosted web

Extend browser preferences with a versioned saved-list format. Store entries
in localStorage, keyed by the site's configured-root identity. The server
exposes an opaque root identity stable across normal restarts, tied to the
configured catalog library. A different identity selects a separate list;
old root-relative shortcuts must not automatically target that new root.

The configured-root identity is not a content fingerprint. Replacing the
contents of a mount at the same configured path without changing its catalog
identity remains the same root, as it does in the existing application.

Use the server-resolved folder identity to deduplicate successful selections.
Retain validated root-relative paths for reopening and tooltips. Never store
or return absolute server paths in the browser entry format.

Subscribe to storage changes so tabs share additions, renames, and removals.
Persist entries under separate keys within the root namespace so simultaneous
edits to different entries cannot overwrite an entire saved list. For competing
edits to the same entry, the last stored mutation wins; apply storage events
by reading the current committed value. Metadata and migration have their own
versioned keys.
Each tab keeps its own active selection and pending selection intent.
Selecting in one tab must not navigate another tab. Removing an entry shared
by the tabs clears any tab currently displaying that entry. Other active
folders remain unaffected.

Keep per-tab active selection in sessionStorage and a browser-level last-opened
entry as the fallback for a new tab. Refresh restores that tab's selection;
a new tab restores the browser's last-opened entry if accessible. Clearing
the active selection records an explicit empty state rather than restoring
a previous fallback on refresh.

Migrate the existing saved selection and breadcrumbs into one entry when its
root identity and folder can be established. Migration runs once and retains
appearance, gallery scope, and sort preferences. Validate stored values and
tolerate malformed data. If storage is unavailable or a write fails, keep the
current tab usable in memory and report that changes cannot be saved.

Browser preferences are not written to shared desktop app_state or to a
server-side personal list. Clearing browser site data removes these shortcuts.

## Shared availability checks

Add a focused access-check coordinator in the Rust application layer, used
by desktop and hosted adapters. The hosted server owns one shared instance
for all requests in its process. It records each folder's in-progress task,
last completed result, completion time, and result generation.

All availability triggers use this coordinator: startup, app/window focus,
saved-entry activation, and hosted selection validation. Coalesce repeated
focus events. Do not perform a second independent access probe immediately
after obtaining a fresh coordinated result. Normal operations still enforce
their required containment and authorization constraints.

The coordinator applies these rules atomically per stable folder identity:

1. If a probe is running, join it rather than launching another filesystem
   operation. Different browsers receive the same completion result.
2. If a result completed less than five seconds ago, return that result.
   The cooldown applies to success and failure and cannot be bypassed by a
   client refresh flag.
3. Otherwise, claim the in-progress slot before starting the probe, then
   publish its result and completion time to waiting callers.

Use monotonic time for cooldowns. Identity lookup for an existing saved folder
must not require a fresh filesystem canonicalization before checking the
in-progress slot. Hosted keys include the configured-root identity; labels,
browser IDs, and transient selection IDs do not create separate check slots.

An access probe checks that the target resolves to an allowed directory and
can be read. It does not recursively enumerate photos, start an indexing
scan, or generate derivatives. A successful activation then follows the
normal selection and shared gallery runtime path.

Run filesystem work away from async request and UI threads. Bound concurrency
across folders. Allow the caller to stop waiting after five seconds and show
an access-check timeout. A timeout is not permission to start a second probe:
if blocking filesystem work has not actually ended, retain its in-progress
slot until it does. Subsequent callers receive an in-progress response rather
than building an unbounded waiter queue. A late completion updates the shared
result but does not auto-open a folder for a timed-out selection intent.

Use transient server memory for this coordination. Restarting the process
clears the cooldown and running state. Multi-process server coordination is
outside this feature's deployment scope.

Distinguish folder missing, unreadable, and root unavailable from network or
server request failure. A client that cannot reach the server cannot conclude
that every underlying folder is missing. Show an access-unverified warning
and allow retry/removal while preserving any already open cached view.
Mark a library root offline only when the root itself is known to be offline;
an unavailable child folder must not disable accessible siblings.

## Components and data flow

React continues to depend on PhotoService rather than transport-specific
imports. Extend that contract with saved-entry listing and change events,
rename/removal, activation/clearing, and availability checks. Bootstrap exposes
the saved list and restoration state. Keep hosted-only root identity in the
HTTP boundary and browser preferences as appropriate.

The main implementation boundaries are:

- NavigationRail and a focused saved-entry component render the list, inline
  label editor, menu, tooltips, and availability state in both desktop and
  drawer layouts.
- useAppController owns selection intents and invalidates them when the user
  chooses another folder, removes an entry, or clears the view.
- The Tauri service adapter and desktop commands call the saved-folder
  repository and coordinated activation through photo-app-service.
- The HTTP adapter owns browser entry persistence and uses server folder
  identity and access-check responses. The server stores shared access state,
  not custom labels or a person's active selection.
- PhotoWall and PhotoViewerOverlay receive active-source access state to
  render warnings while keeping usable cached content visible.

Do not hold a catalog mutex or a global selection lock during slow filesystem
work. Serialize only the short admission and state-commit steps. Preserve the
existing per-selection scan and derivative coordination.

Availability responses can update a still-existing entry, but only a response
matching the current selection intent can change the active wall. Ignore old
generations, removed entries, responses for another configured root, and
events from a previous active selection. A shared server result must never
change which folder another browser is viewing.

## Verification

Use focused unit and integration tests for the new behaviour, followed by
the repository's existing interface and Rust checks during implementation.

- Entry persistence, desktop profile isolation, browser root namespaces,
  migration from the old active selection, malformed storage, and failed
  persistence writes.
- Duplicate folder selection, preserved custom labels, whitespace reset,
  duplicate display labels, natural case-insensitive order, and stable ties.
- Active and inactive removal, preserved source files/catalog/cache,
  reopening after removal, and restored active-selection behaviour.
- Cross-tab list updates with independent selections, including removal of
  an entry active in another tab and a concurrent edit to a different entry.
- Startup/focus/activation checks, success and failure cooldowns, simultaneous
  requests from separate clients causing exactly one filesystem probe,
  independent folders, and cooldown expiry using a controlled clock.
- Slow filesystem work, bounded admission, timeout without duplicate probes,
  recovery after completion, and no locks held across the slow operation.
- Missing child versus offline root, root-relative path validation, containment,
  and no absolute hosted path leakage.
- Switching or removing entries during checks and image loading, with no late
  response changing the current folder or resurrecting a removed entry.
- Cached wall and immersive viewer behaviour when the active folder goes
  offline, missing-content placeholders, recovery, and blocked return after
  switching away.
- Browser tests for rename keyboard controls, menu and tooltip accessibility,
  focus after reordering/removal, narrow-screen drawer selection, long labels,
  and scrollable lists.

No source implementation changes are part of this design-document step.

## Scope limits

This feature does not add accounts, cross-device synchronization, nested
sidebar trees, manual ordering, filesystem rename/delete, locate/relink,
per-folder viewing preferences, continuous background polling, or distributed
coordination across server processes. Existing image indexing, cache eviction,
and source-safety policies continue to apply.
