# Final UI and copy seam report

## Status

Complete on `codex/release-final-ui-copy-seams`. The change is limited to the wall reducer, Picks copy controller and tests, and the desktop copy command. Native atomic publication and AppService shutdown code were not touched.

## Root causes

- A current settled first-page response rebuilt the wall from an empty list, while metadata settlement rejected an equal generation and always merged a newer terminal generation. This could either drop loaded later pages or retain photos deleted by a newer complete scan.
- The Picks controller passed its pre-picker ID list for every copy, and the desktop command did not emit initial progress until every batch was prepared. Native full copies therefore bypassed the post-picker picks snapshot and the drawer stayed in its non-cancellable choosing state during preparation.
- Copy failures used the same one-second toast lifetime as add and cancel confirmations.

## RED evidence

The new interface tests failed before production changes with five expected assertions: same-generation metadata was not patched, the current settled first page dropped its later page, a newer complete generation retained the absent asset, full copy received explicit stale IDs instead of `null`, and both partial and native-error toasts disappeared after one second.

The first sandboxed browser run could not bind its loopback port (`EPERM`). The approved browser run exercised the rendered AppShell and Picks panel with the real controller path.

## Implementation

- Current-generation settled first pages now patch the loaded sequence and preserve loaded later pages and their conservative total. A terminal metadata page from a strictly newer generation replaces the sequence and accepts its lower authoritative total; equal-generation metadata remains a patch.
- Full copies pass `null` so the native command snapshots current picks after destination selection. Retry still sends only the explicit failed IDs.
- The desktop command registers the operation, snapshots IDs, emits `0 / total`, and checks cancellation before and between preparation chunks. The drawer moves to copying on that event and exposes Cancel before preparation.
- Partial results and native copy, cancel, or Show-folder failures remain visible for five seconds with the existing 200 ms fade. Add and successful cancel confirmations remain one second.

## Verification

- Interface unit focus: 3 files, 117 tests passed.
- Picks browser focus: 1 file, 45 tests passed.
- Desktop command focus: 14 tests passed.
- Interface typecheck: passed.
- Biome on six touched interface files: passed.
- Desktop `cargo fmt --check`: passed.
- Desktop Clippy passed with `-D warnings` after allowing the pre-existing `large_enum_variant` warning in untouched `dto.rs` on the command line. The unadjusted Clippy invocation stopped only on that existing warning.
- `git diff --check`: passed.

## Concerns

Cancellation is observed before the first preparation call, between 250-item chunks, and after each preparation call. `prepare_original_copy` itself has no cancellation parameter, so a cancellation that arrives during one in-flight preparation call takes effect immediately after that call returns and never proceeds to copying.

## Fix round 1

Re-review found that newer-generation replacement still depended on `nextCursor` being null. `usePhotoWall` reads settlement through a 100-item page, so an authoritative generation with more than 100 photos has a non-null next cursor even though the scan generation itself is complete.

The regression uses 150 loaded old assets and a newer generation with a 100-item first page, logical total 120, and `new-page-2`. Before the fix, all 150 old assets remained, the total stayed 150, and pagination stayed exhausted. After the fix, the wall contains exactly the new first page, reports total 120, points at `new-page-2`, and marks pagination nonterminal. The same-generation settlement regression still preserves its loaded later page and conservative total.

A second RED regression reproduced the review Minor: a native copy error during active Clear/Undo updated the live announcement but disappeared when Undo expired because no toast had been queued. Copy failures now deliberately defer behind the Undo toast, retain the original Undo deadline and action, then display for five seconds with the existing final fade.

Fix-round verification:

- Focused wall, Picks controller, and Tauri service unit tests: 3 files, 118 tests passed.
- Interface typecheck: passed.
- Biome on the four fix-round interface files: passed.
- `git diff --check`: passed.

## Fix round 2

The full browser suite exposed a second settlement edge: generation 2 streamed 21 photos after the 100 already displayed, then its authoritative query returned only the first 100 with a next-page cursor. Replacing the wall with that page correctly removed stale generation-1 assets, but also removed the 21 assets already observed in generation 2.

The reducer now records asset IDs delivered by `catalogBatch` for the current generation. When a newer generation settles, it rebuilds the wall from the authoritative first page plus only that generation's streamed remainder, clears the tracking set, accepts the new total and cursor, and drops every unobserved prior-generation asset. Unchanged first-page asset objects are reused so settlement does not needlessly restart rendered tile preview state.

The new reducer regression failed RED with 100 items instead of 121. It now proves that a paged generation-2 settlement retains its 21 streamed assets, removes 50 stale generation-1 assets, reports total 121, resets pagination to the new cursor, and preserves identity for unchanged first-page assets. `PhotoViewer.browser.test.tsx` was not changed; its existing “keeps catalog-loaded photos when a later settlement page is shorter” acceptance test remains the integrated contract.

Fix-round-2 verification:

- Focused wall reducer unit test: 59 tests passed.
- Existing focused PhotoViewer browser acceptance test: 1 passed, 92 skipped.
- Interface typecheck: passed.
- Biome on the two changed wall files: passed.
- `git diff --check`: passed.

## Fix round 3

Re-review identified an event-order race: a generation-6 catalog batch can arrive before a delayed generation-5 metadata settlement. The older settlement previously replaced `streamedAssetGeneration` with 5 and cleared generation 6's tracked IDs, so the later paged generation-6 settlement could no longer retain its already-streamed remainder.

The RED reducer test interleaves those events deterministically and initially observed generation 5 where generation 6 was required. Settlement now treats an equal-or-newer tracked stream as protected wall content, and only clears stream tracking when the settlement is for that generation or a newer one. The test proves generation 6's 21 assets and tracking survive generation 5, then generation 6 retains those assets while pruning 50 stale assets and adopting its authoritative total and cursor.

Fix-round-3 verification:

- Focused wall reducer unit test: 60 tests passed.
- Focused PhotoViewer browser run: 2 tests passed, including the unchanged “keeps catalog-loaded photos when a later settlement page is shorter” expectation.
- Interface typecheck: passed.
- Biome on the two changed wall files: passed.
- `git diff --check`: passed.
