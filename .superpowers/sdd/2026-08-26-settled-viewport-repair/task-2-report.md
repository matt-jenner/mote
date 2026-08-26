# Task 2 report: cached thumbnail completion fallback

## Status

Complete. The bounded `PhotoTile` repair and focused browser regressions are implemented.

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
- After mount and derivative URL replacement, checks the current image on the immediate layout pass and the next animation frame; `complete === true` and `naturalWidth > 0` mark that exact image loaded.
- Keyed the image by derivative URL so stale completions from a previous URL cannot reveal a replacement.
- Kept error, unavailable-file fallback, warning, colour placeholder, and fade behavior unchanged.
- Added WebKit browser coverage for cached completion without React `load`, zero-natural-width failure, and replacement reset behavior.

## Verification

- `npm run test --workspace @photo-viewer/interface`: 4 files, 66 tests passed.
- `npm run test:browser --workspace @photo-viewer/interface`: 2 files, 47 tests passed.
- `npm run typecheck --workspace @photo-viewer/interface`: passed.
- `npm run check`: 36 files checked, no issues.
- `npm run build --workspace @photo-viewer/interface`: passed.

## Commit

`c9a1b11` (`fix(interface): reveal cached wall thumbnails`).

## Concerns

No known concerns. The native app was not launched and no catalog, derivative cache, or source photo files were touched.
