# Task 2 report: cached thumbnail completion fallback

## Status

Complete. Round 1 follow-up is implemented and verified.

## Confirmed root cause

`PhotoTile` only changed the image layer to visible from React's `onLoad` callback. In WebKit, a cached custom-protocol image can already be complete with a positive natural width while no usable `load` callback reaches that mount. The thumbnail and cache data were healthy; the missing state transition was in the image element's loaded/paint state.

## Red command and observed failure

Command:

```text
npm run test:browser --workspace @photo-viewer/interface -- --testNamePattern='reveals a cached image when its load event does not reach React'
```

Before the production change, the real WebKit regression loaded a warmed image while capture-phase `load` propagation was suppressed. The image was complete with positive natural width, but the user-visible opacity assertion failed:

```text
AssertionError: expected '0' to be '1'
```

## Implementation

- Added a ref to the current image element.
- Retained the normal `onLoad` path while accepting only the current image/current URL.
- Kept the immediate cached-image check, then await the current image's `decode()` promise as a bounded non-polling completion signal. The decode completion is fenced by effect cancellation, current image identity, current derivative URL, and `complete === true` with `naturalWidth > 0`.
- Keyed the image by derivative URL so stale completions from a previous URL cannot reveal a replacement.
- Kept error, unavailable-file fallback, warning, colour placeholder, and fade behavior unchanged.
- Added deterministic WebKit coverage for cached completion without React `load`, delayed completion after the old immediate/one-frame window, zero-natural-width completion with error delivery suppressed, and stale URL-A decode completion after URL-B replacement.

## Fix-round timing evidence

The delayed-completion regression holds `complete === false` and `naturalWidth === 0` for 75 ms while suppressing capture-phase `load` delivery. Opacity remains `0` after the old immediate/one-frame window. After the test resolves the image's decode promise and exposes positive natural width, the fenced decode path changes opacity to `1`.

The zero-width regression exposes `complete === true` with `naturalWidth === 0` while suppressing `error`; the image remains at opacity `0`. The stale-completion regression resolves URL A's decode after URL B has replaced it; URL B remains at opacity `0` until URL B's own decode resolves.

## Verification

- `npm run test --workspace @photo-viewer/interface`: 4 files, 66 tests passed.
- `npm run test:browser --workspace @photo-viewer/interface`: 2 files, 49 tests passed.
- `npm run typecheck --workspace @photo-viewer/interface`: passed.
- `npm run check`: 36 files checked, no issues.
- `npm run build --workspace @photo-viewer/interface`: passed.

## Commit

`adf0dc2` (`fix(interface): await fenced thumbnail decode`).

## Concerns

No known concerns. The native app was not launched and no catalog, derivative cache, or source photo files were touched.
