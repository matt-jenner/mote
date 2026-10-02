# Fast remount recovery design

Date: 2026-10-02

Status: behavior approved in conversation; pending written-spec review

## Purpose

When a saved photo folder becomes readable again, Mote must remove its source
warnings within seconds. Recovery must not wait for image decoding, metadata
extraction, or a complete re-index.

The current recovery path starts the normal scan. On a library of about 5,000
photos that has taken 80 minutes, even when none of the files changed. This
design separates proof that the folder is mounted from the slower work needed
for new or changed files.

Source media remains read-only.

## Success criteria

1. A successful root access check updates the saved folder and catalog from
   `root_offline` to `available` within one second on a responsive local or USB
   volume. The interface removes source-unavailable warnings as soon as it
   receives that result.
2. Reconciliation of about 5,000 known files normally completes within five
   seconds on a responsive local or USB volume when their path, size, and
   modification time are unchanged.
3. Unchanged files are not opened or decoded. Mote reads directory entries and
   file metadata only.
4. New and changed files continue through the existing shape, colour, metadata,
   HEIF, and derivative pipeline.
5. Files that disappeared become `missing` only after a successful scan
   generation completes. Existing `missing` and `unreadable` states are never
   cleared merely because the root returned.

The timing figures are operating targets, not wall-clock test assertions. A
slow network share can take longer because filesystem metadata calls can block,
but it must still avoid opening unchanged image content.

## Recovery flow

### 1. Prove folder access

The saved-folder access check opens the canonical configured root and, for a
child saved folder, the selected child directory. A successful check is enough
to say that the source is mounted. It must not wait for a scan.

On success, the application service performs one catalog transaction:

- Set the affected library root to `available`.
- Change affected assets from `root_offline` to `available`.
- Leave `missing`, `unreadable`, and other item-specific states unchanged.
- If only a child folder recovered, limit the asset update to that folder. Do
  not clear warnings for an unavailable sibling folder.

The service then publishes the saved-folder availability change. The active
wall can remove its source warning immediately, and the next catalog query
returns recovered asset availability.

If the root disappears between this check and reconciliation, the scanner uses
the existing source-unavailable path to mark the affected scope offline again.

### 2. Reconcile known files without reading image content

Before starting the scan, the application service loads a compact snapshot of
known assets in the affected scope. Each entry contains:

- Stable asset identity and source-relative path.
- File size and modification time from the last successful scan.
- Relevant sidecar modification time, where one exists.
- Whether required cached shape and metadata are already settled.
- Whether a one-time forced refresh applies, such as the HEIF decoder
  transition.

The scanner walks the directory and builds the same signature from filesystem
metadata. For a known asset whose signature matches and whose cached data is
settled, it emits the normal discovered and generation-membership events, then
counts the item as processed. It does not open the image, probe its dimensions,
decode representative colour, read EXIF or XMP content, or run HEIF decoding.

A new file, changed signature, missing cached result, or forced refresh follows
the current full processing pipeline. This preserves the existing work needed
to build or repair catalog data and derivatives.

The independent inventory check may keep its second directory traversal. Both
passes must use directory entries and filesystem metadata only for unchanged
files. Retaining the second pass protects the generation-completion check from
partial discovery without returning to content-level work.

### 3. Finish the generation

Unchanged assets still join the active generation, update `last_seen_at`, and
finish with `available` state. The existing generation fence rejects stale
events. After a successful completion, catalog assets that were not discovered
in that scope become `missing` through the existing completion rules.

Cancellation, a newer generation, an inventory mismatch, or a root access
failure prevents missing-file settlement just as it does today.

## Interface warning rules

The folder access result is authoritative for a folder-wide source warning.
When the current saved-folder context is `available`, a stale
`sourceUnavailable` warning on an already loaded asset must not fall through to
the generic photo-warning badge. That fallback currently produces the lingering
exclamation symbol after a remount.

Photo tiles, the immersive viewer, and pick rows must apply the same rule:

- Show the source warning when the current folder context is unavailable.
- Hide a stale `sourceUnavailable` asset warning when the current folder context
  is explicitly available.
- Continue showing real item warnings, including missing originals, unreadable
  files, and derivative failures.

This immediate projection avoids waiting for a page reload. Catalog recovery
still updates the durable state so reopening the folder produces the same
result.

## Data and API changes

No schema migration is required. Existing asset signature, availability,
shape, metadata, and scan-generation fields remain the source of truth.

Add catalog operations for scoped root recovery and for reading the known-asset
snapshot. Extend the scan request with that snapshot or an equivalent lookup
owned by the scan job. Keep platform-specific path comparison inside the
existing asset identity rules rather than adding string-only path matching.

The fast path must be observable in tests through dependency call counts. It
must not rely on elapsed-time tests, which would be unreliable on shared CI
runners.

## Error and race handling

- A successful access result applies only to its saved-folder identity and
  check generation. A late result cannot clear a newer failure.
- A library-root recovery may clear `root_offline` across that library. A child
  recovery clears only its linked assets unless the root check also proves the
  whole library available.
- If `stat` fails for an individual entry, process it through the existing
  per-item error handling. Do not make the whole root unavailable unless the
  root itself can no longer be opened.
- A matching size and modification time is the normal unchanged-file contract.
  Explicit refresh operations bypass it.
- The scanner never writes to, renames, or deletes source files or sidecars.

## Verification

### Catalog and application service

- Recovering a root changes only `root_offline` rows to `available` and leaves
  `missing` and `unreadable` rows untouched.
- Recovering a child folder does not clear an offline sibling.
- A successful saved-folder recheck publishes `available` and updates catalog
  state before the reconciliation scan finishes.
- A stale success result cannot override a newer unavailable result.

### Indexer

- An unchanged known asset emits discovery and generation membership with zero
  image-probe, metadata-reader, or decoder calls.
- New and changed assets each use the full processing path.
- Missing cached shape or metadata and forced HEIF refresh bypass the fast path.
- A successful generation still marks undiscovered catalog assets missing.
- Cancellation and inventory mismatch do not settle missing files.

### Interface

- An available folder plus a stale `sourceUnavailable` asset warning shows no
  generic exclamation badge.
- An unavailable folder still shows the source warning on tiles and viewers.
- Missing, unreadable, and derivative warnings remain visible when the folder is
  available.

### Performance check

Add a repeatable benchmark or diagnostic test fixture with at least 5,000 known
unchanged entries. Report filesystem metadata operations and content-reader
calls. The expected result is zero content-reader calls and a local development
run near the five-second target. Keep this diagnostic outside the required CI
pass/fail suite.

## Out of scope

- A live filesystem watcher or continuous change feed.
- Content hashing of every photo during recovery.
- Changes to derivative cache formats or eviction.
- Hiding item-specific warnings.
- General hosted-source recovery changes beyond reusing the same safe fast path
  where the current saved-folder service already applies.
