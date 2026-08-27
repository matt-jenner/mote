# Viewer zoom and pan

Date: 2026-08-27

Status: Approved in conversation, awaiting review of this written specification

## Summary

Add a fast, cache-only zoom and pan slice to the immersive photo viewer. The
feature uses the existing wall thumbnail and 4096-pixel screen preview. It does
not read the original image on demand, request deep-zoom tiles, or add another
backend contract.

The interaction should feel native on macOS, Windows, Linux, and the hosted
mobile PWA. Pinch, modified wheel input, keyboard shortcuts, double-click or
double-tap, drag panning, and visible fallback controls all manipulate one
shared transform. A compact navigator appears only while zoomed and shows the
visible region of the whole photo.

Changing photos resets the view to fit. Resizing or rotating preserves the
current zoom and the image point at the centre. Progressive replacement of a
thumbnail with the screen preview never changes the transform.

## Relationship to approved designs

This specification extends the approved [Photo Viewer system
design](2026-08-21-photo-viewer-design.md) and [immersive filmstrip photo
viewer](2026-08-26-immersive-filmstrip-viewer-design.md).

It preserves these rules:

- Source media is read-only.
- SQLite and the managed derivative cache are the only application write
  locations.
- React components depend on the host-neutral `PhotoService` contract.
- Cached derivatives remain useful while a source is unavailable.
- Preview refinement preserves geometry and interaction state.
- Fit-state horizontal touch swipes navigate; zoomed drags do not.
- Controls remain safe-area aware, keyboard accessible, and quiet when unused.
- The wall remains mounted behind the viewer and returns at its exact position.

Where this specification is more specific, it controls moderate viewer zoom
and pan. Original-resolution delivery and deep zoom remain later work.

## Product outcome

A user can inspect detail in the currently displayed photo without waiting for
a network share or original source file. Zoom stays smooth and centred on the
point under the pointer, pinch midpoint, keyboard focus, or double-tap. Panning
never exposes empty space beyond the valid image bounds.

The user can always recover to fit with a visible reset control, a shortcut,
or double-click or double-tap. The navigator makes the current location clear
without permanently occupying the viewer. Navigating to another photo returns
to the existing predictable fit state.

## Scope

### Included

- Continuous zoom from fit to the native resolution of the best decoded cache
  derivative
- Pointer-centred modified-wheel zoom
- Trackpad pinch zoom
- Touch pinch zoom around the pinch midpoint
- Mouse and one-finger drag panning while zoomed
- Ordinary wheel or trackpad panning while zoomed
- Double-click and double-tap toggling between fit and native 100 percent
- `+`, `-`, and `0` keyboard shortcuts
- Visible minus, Fit or percentage, and plus controls
- A contextual navigator with a viewport rectangle
- Desktop dragging of the navigator viewport
- Fit-state swipe navigation and zoomed pan-only gesture arbitration
- Zoom and focal-point preservation through resize and rotation
- Transform preservation when a better cached preview replaces a thumbnail
- Reset to fit when the current asset changes
- Safe-area, reduced-motion, keyboard, screen-reader, and touch-target coverage
- macOS exploratory demonstration using cached indexed photos

### Deferred

- Original-file reads initiated by zoom
- Zoom beyond the decoded derivative's native resolution
- Deep-zoom tile generation or delivery
- RAW full-resolution decode initiated by zoom
- Persisting zoom between photos
- A locked comparison zoom across several photos
- Momentum, elastic overscroll, or spring-back physics
- Navigator dragging on mobile
- Video zoom or playback
- Metadata editing

## Selected approach

Keep the existing decoded `<img>` layers and apply a shared CSS transform
derived from a small, browser-independent geometry controller.

The controller stores view intent in image space rather than screen offsets.
The stage, controls, gesture adapter, and navigator consume its derived
geometry. This retains the proven progressive-preview and accessibility
structure while isolating calculations that require exact tests.

Two alternatives were rejected:

- A scrollable enlarged container makes pointer-centred zoom, pinch, rotation,
  and exact focal-point preservation depend on browser scrolling differences.
- A canvas renderer would prepare for deep zoom but replace working image
  semantics and crossfade behaviour before the product needs that cost.

## Transform model

The transform state contains:

- `mode`: `fit` or `zoomed`
- scale relative to the fitted image rectangle
- a focal point expressed as normalized image coordinates from 0 to 1
- the current drawable viewport dimensions
- the current decoded derivative's natural dimensions
- a revision identifying the current asset

Pure functions derive:

- fitted image bounds
- maximum zoom scale
- transformed image bounds
- clamped pan or focal position
- CSS translation and scale
- navigator viewport rectangle

Fit is scale 1. The minimum scale is always fit. The maximum scale is the
ratio between the best decoded derivative's natural dimensions and its fitted
CSS dimensions, clamped to at least 1. This represents one derivative pixel per
CSS pixel and prevents the viewer from enlarging cached pixels beyond their
native resolution.

If the available derivative is smaller than the viewport, the maximum remains
fit. The plus control and zoom shortcuts disable when no enlargement is
available.

Zoom steps use a multiplicative factor of 1.25. Continuous wheel and pinch
input may choose intermediate values within the same bounds.

## Focal-point zoom and panning

Zooming preserves the image point beneath the pointer or pinch midpoint. The
controller converts that screen point into normalized image coordinates before
changing scale, then derives the new translation so the point stays fixed.

Keyboard and plus-button zoom use the centre of the drawable image area as the
anchor. The Fit or percentage control resets to fit. Pressing `0` performs the
same reset.

While zoomed, drag changes the focal point and is clamped so the transformed
image continues to cover the drawable stage. No drag reveals empty canvas
beyond an image edge. The first slice uses firm clamping rather than momentum
or elastic overscroll.

At fit, one-finger horizontal touch movement keeps its existing navigation
meaning. While zoomed, all one-finger image drags pan and never navigate, even
at an image edge. Previous and next buttons, keyboard arrows, and the filmstrip
remain available.

Mouse clicks and drags do not enter touch navigation. When zoomed, the image
uses a `grab` cursor and changes to `grabbing` during an active pan.

## Input model

### Desktop and pointer devices

- Trackpad pinch and modified wheel input zoom around the pointer.
- `Ctrl` or `Cmd` plus wheel is captured only over the open viewer stage.
- Ordinary wheel or trackpad deltas pan while zoomed and do nothing to the
  photo at fit.
- Mouse drag pans only while zoomed.
- Double-click toggles between fit and native 100 percent around the clicked
  point.
- `+` and `-` change scale by a 1.25 factor.
- `0` resets to fit.
- Arrow keys continue to navigate photos and reset the newly selected photo
  to fit.

### Touch devices

- Pinch zooms around the midpoint of the two active primary touches.
- One-finger drag pans while zoomed.
- One-finger horizontal swipe navigates only at fit.
- Double-tap toggles between fit and native 100 percent around the tapped
  point.
- Single tap retains its control-visibility behaviour.

Single-tap dispatch is delayed only long enough to distinguish it from a
double-tap. A completed double-tap must not briefly toggle the controls before
zooming. Touches that begin on buttons, the filmstrip, the information drawer,
or the navigator controls do not begin a stage gesture.

Browser page zoom and document scrolling are prevented only for gestures that
the open viewer consumes. The hosted PWA must not disable page behaviour
outside the viewer.

## Zoom controls

A compact zoom cluster appears with the normal viewer chrome:

- Minus button
- Central Fit or percentage button
- Plus button

Every target is at least 44 by 44 CSS pixels. Disabled limits use native
button state and remain understandable without colour. The percentage is
relative to the decoded derivative's native display scale; fit is labelled
`Fit` rather than as a misleading percentage.

The cluster stays clear of the filmstrip, safe-area insets, and information
drawer. When the drawer is open, the cluster shifts into the remaining usable
stage instead of sitting underneath the drawer.

Keyboard zoom announces the resulting Fit or percentage value through a
polite live region without announcing every intermediate pinch frame.

## Navigator

The navigator appears only while the viewer is zoomed. It shows:

- the best currently decoded full-photo layer
- the aspect-correct full image bounds
- a high-contrast viewport rectangle derived from the current transform

The navigator appears during zoom or pan and whenever normal viewer controls
are visible. It fades with the controls after inactivity, but active
manipulation keeps it visible. Resetting to fit removes it.

On desktop, dragging inside the navigator moves the main focal point. Clicking
a location recentres the main view there. Navigator interaction uses the same
clamping functions as stage panning.

On mobile, the navigator is display-only. It does not capture panning gestures
or create a small competing touch target. It remains readable without
obscuring the main subject and respects every safe-area inset.

If the information drawer opens, the navigator repositions within the visible
stage. It never appears underneath the drawer.

## Progressive preview refinement

Zoom uses the best decoded cached layer already available in the stage. A wall
thumbnail may provide the first zoom interaction while the screen preview is
still decoding. Its natural dimensions set a temporary maximum scale.

When the 4096-pixel screen preview becomes ready:

1. It crossfades within the existing transformed stage.
2. Scale, normalized focal point, and pan position remain unchanged.
3. The maximum zoom range expands if the new layer has more pixels.
4. The navigator replaces its image without moving its viewport rectangle.

If the current scale would exceed the native limit of a replacement layer, the
controller clamps once to the new valid maximum while preserving the focal
point. Normal thumbnail-to-preview refinement should increase, not reduce, the
limit.

If screen-preview generation fails, the thumbnail remains zoomable up to its
own native limit. The existing `Larger preview unavailable` message remains in
the information drawer. Zoom does not initiate an original source read.

Ready cached derivatives continue to support zoom after a source becomes
unavailable.

## Navigation, resize, and rotation

Every asset selection resets transform state to fit before the new image is
presented. This applies to keyboard, buttons, filmstrip selection, and fit-state
swipe navigation. Zoom and pan are never carried between photos in this slice.

Resize and rotation preserve:

- current asset and decoded layers
- current scale, subject to the recalculated native maximum
- the normalized image point at the centre of the drawable stage
- information drawer state
- control visibility state

The controller recalculates fitted bounds, pan limits, CSS transform, zoom
controls, and navigator geometry from one coalesced viewport revision. It does
not dispatch viewer navigation, preview reset, or another wall query.

If preserving the exact focal point would reveal empty canvas after a resize,
the controller clamps to the nearest valid focal point.

## Control visibility and interaction scheduling

Zooming, panning, wheel input, navigator movement, and zoom-button use all
report interaction through the existing wall interaction API. Current and
nearby derivatives retain their high priority while library-wide idle work may
yield during manipulation.

Active manipulation keeps the zoom controls, navigator, and required viewer
chrome visible. When interaction and control focus end, the existing pointer or
touch inactivity timer resumes.

Hiding controls never removes a focused zoom control. When all chrome is
hidden, the viewer dialog continues to contain keyboard focus. The information
drawer retains its existing exception: Info and drawer Close remain available
while it is open.

## Accessibility and reduced motion

- Zoom controls have explicit names and expose disabled limits.
- The central control announces Fit or the current percentage.
- The navigator is decorative for screen readers; zoom state is conveyed by
  controls and the live region.
- Keyboard operation does not depend on pointer location.
- Focus remains stable through transform, refinement, resize, and rotation.
- High-contrast mode keeps the navigator viewport rectangle visible.
- Reduced-motion mode removes animated transform settling and navigator fade.
  Direct manipulation still follows the pointer or touch synchronously.
- Touch targets remain at least 44 by 44 CSS pixels in portrait and landscape.

The viewer does not trap assistive technology inside the navigator and does
not announce every pan coordinate.

## Error handling and invariants

- Invalid natural dimensions leave the viewer at fit and disable zoom.
- Non-finite scale, focal, or pointer inputs are ignored and never reach CSS.
- Pointer cancellation, lost capture, viewer close, asset navigation, and
  viewport revision end active gestures safely.
- A stale gesture or preview completion from an older asset revision cannot
  alter the current transform.
- Removing the current asset closes the viewer through the existing safe path.
- No zoom operation reads, writes, renames, rates, tags, or deletes source
  media.
- No native path enters the shared service, DOM, logs, or accessibility text.

## Components and boundaries

The feature adds or extends these focused units:

- `viewerTransform`: pure fit, zoom, focal-point, clamping, resize, and
  navigator geometry
- `useViewerTransform`: owns current transform state and asset-revision reset
- `useViewerGestures`: arbitrates fit swipe, zoomed pan, pinch, double-tap,
  pointer cancellation, and lost capture
- `ViewerStage`: applies one derived transform to every decoded image layer
- `ViewerZoomControls`: renders minus, Fit or percentage, and plus controls
- `ViewerNavigator`: renders the contextual overview and desktop drag input
- `PhotoViewerOverlay`: connects shortcuts, control visibility, interaction
  reporting, drawer layout, and asset navigation

The geometry module imports no React, browser, Tauri, filesystem, or service
code. The React layer imports no Tauri API. No Rust or `PhotoService` contract
change is expected for this cache-only slice.

## Data flow

1. The viewer opens at fit with catalog dimensions and the best decoded layer.
2. The stage supplies drawable bounds and decoded natural dimensions to the
   transform controller.
3. An input adapter converts wheel, pinch, double-tap, drag, button, or keyboard
   input into one transform action.
4. Pure geometry derives scale, focal point, pan bounds, CSS transform, and
   navigator rectangle.
5. The stage applies the same transform to thumbnail and screen-preview layers.
6. The navigator and controls render derived state.
7. Preview refinement updates the native zoom ceiling without replacing view
   intent.
8. Navigation increments the asset revision and resets transform state to fit.
9. Resize or rotation recalculates geometry while preserving the normalized
   centre point.

## Testing strategy

### Unit tests

- Fit and native-scale calculations for landscape, portrait, square, small,
  and malformed dimensions
- Pointer-centred zoom invariance
- Pinch midpoint invariance
- Pan clamping at every edge
- Resize and rotation focal-point preservation
- Native-limit expansion after preview refinement
- Navigator viewport and click-to-centre geometry
- Asset-revision reset
- Non-finite input rejection

### Browser tests

- Modified-wheel and pinch zoom
- Mouse and touch panning
- Double-click and double-tap without stray single-tap chrome changes
- `+`, `-`, `0`, buttons, focus, disabled limits, and live status
- Fit swipe navigation versus zoomed pan-only behaviour
- Navigation reset through keyboard, buttons, and filmstrip
- Thumbnail-to-preview crossfade with an unchanged transform
- Preview failure and offline cached zoom
- Navigator appearance, geometry, desktop drag, fade, and drawer avoidance
- Resize and portrait or landscape rotation preservation
- Safe areas, 44-pixel targets, high contrast, reduced motion, and Axe
- No document scroll or browser zoom leakage from consumed viewer gestures

### Native demonstration

On macOS, use the existing indexed `wall-demo` profile to demonstrate:

1. Immediate fit view from a cached thumbnail
2. Double-click to native 100 percent and drag pan
3. Trackpad pinch and ordinary trackpad pan
4. Plus, minus, Fit, and keyboard shortcuts
5. Navigator movement and desktop viewport dragging
6. Preview refinement without a jump
7. Next-photo reset to fit
8. Narrow and wide resize focal preservation
9. Cached zoom after the source becomes unavailable, if safely reproducible

Leave the app running for user exploratory acceptance. Do not merge until the
user accepts the slice.

## Acceptance criteria

1. Zoom never requires a live source read once a usable derivative is cached.
2. The viewer supports fit through native cached resolution and never enlarges
   beyond that native limit.
3. Zoom preserves the point beneath the pointer or pinch midpoint.
4. Zoomed drag pans and never navigates, including at an image edge.
5. Fit-state horizontal touch swipe continues to navigate one photo.
6. Navigation resets the new photo to fit.
7. Resize and rotation preserve zoom and the centred image point, subject only
   to valid clamping.
8. Preview refinement preserves scale, focal point, pan, and stage geometry.
9. The navigator appears only while zoomed and accurately represents the
   visible region.
10. Every zoom path has keyboard or visible-control equivalence.
11. Controls, navigator, drawer, and filmstrip remain reachable and
    safe-area-correct on desktop and mobile.
12. Reduced-motion and high-contrast modes remain usable.
13. Cached zoom works while the source is unavailable.
14. Source fixture hashes remain unchanged after verification.
15. The complete interface, browser, Rust, desktop, lint, build, benchmark, and
    macOS bundle gates pass before user acceptance.

## Follow-up work

- Original-resolution and deep-zoom derivative delivery
- Locked zoom for comparing several photos
- More advanced trackpad momentum or elastic pan physics
- Desktop locate and reconnect for unavailable uncached assets
- Video playback and poster-frame zoom policy
