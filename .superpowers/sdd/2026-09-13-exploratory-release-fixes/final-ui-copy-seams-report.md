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
