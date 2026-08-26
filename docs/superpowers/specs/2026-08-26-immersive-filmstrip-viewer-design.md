# Immersive filmstrip photo viewer

Date: 2026-08-26

Status: Approved in conversation, awaiting review of this written specification

## Summary

Build the next macOS vertical slice: clicking an available wall tile opens an edge-to-edge photo viewer without unmounting the wall. The viewer shows the best cached derivative immediately, refines it to the 4096-pixel screen preview without changing layout, and supports keyboard, pointer, filmstrip, and touch navigation.

The viewer includes a non-modal information drawer for a small, read-only metadata set. Controls and the bottom filmstrip stay out of the way until the user interacts. A visible back chevron returns to the exact wall position and briefly identifies the tile just viewed. The same React components and service boundary must work in the later hosted web portal and mobile PWA.

This slice deliberately stops before pan and zoom. Its gesture boundary reserves that behaviour: horizontal drag navigates only while the image is fit to the window. A later zoomed state will use drag for panning and suppress navigation swipes.

## Relationship to approved designs

This specification extends the approved [Photo Viewer system design](2026-08-21-photo-viewer-design.md), [macOS browse vertical slice](2026-08-24-macos-browse-vertical-slice-design.md), [progressive photo wall](2026-08-25-macos-progressive-photo-wall-design.md), and [thumbnail-first loading remediation](2026-08-26-thumbnail-first-loading-design.md).

It preserves these rules:

- Source media is read-only.
- SQLite stores metadata and derivative references, not image blobs.
- Generated derivatives live below the managed local cache root.
- React components depend only on the host-neutral `PhotoService` contract.
- Tauri details remain inside the desktop adapter.
- Cached catalog entries and derivatives remain useful while a root is offline.
- The active wall order comes from SQLite and keeps stable asset identities.
- Visible derivative work outranks speculative background work.

Where this document is more specific, it controls the immersive viewer slice. Pan, zoom, deep-zoom delivery, metadata editing, and video playback remain later work.

## Product outcome

A user can open any available photo from the justified wall and immediately see a credible full-window image. The viewer may begin with the ready wall thumbnail, but it must request and smoothly crossfade to the screen preview. Moving between photos must never expose an empty stage while any usable cached derivative exists.

The user can move through the wall's current order with the left and right arrow keys, visible previous and next controls, the filmstrip, or a horizontal swipe. Basic information is available through an explicit Info control. The interface keeps its controls quiet when unused and remains usable after a phone or tablet rotates.

Closing the viewer returns to the same wall, scroll position, and current tile. Opening and closing the viewer must not repeat the wall query or discard loaded thumbnails.

## Scope

### Included

- Edge-to-edge viewer overlay above the still-mounted wall
- Best-ready-derivative first frame
- Explicit visible-priority screen-preview requests
- Fixed-stage crossfade from wall thumbnail to screen preview
- Left and right keyboard navigation
- Visible previous and next controls
- Bottom filmstrip navigation
- Horizontal touch swipe navigation
- Auto-hiding controls for pointer and touch input
- Safe-area-aware back chevron that restores the wall position
- Read-only overlay information drawer
- Filename, capture date and time, resolved rating, dimensions, and media type
- Ordered navigation over the wall's loaded result window
- Incremental `loadMore` near a loaded boundary
- Adjacent-preview prefetch
- Responsive relayout on resize and mobile orientation change
- Cached and offline-safe viewing
- Desktop, tablet, and phone-width automated coverage
- A macOS Tauri demonstration with real indexed photos

### Deferred

- Pan, zoom, double-tap zoom, pinch zoom, and original-resolution delivery
- Deep-zoom tiles
- Keyword and extended metadata presentation
- Metadata or rating editing
- Viewer navigation over results not yet loaded into the wall controller
- A dedicated backend viewer-session or neighbour-token API
- Video playback controls and transcoding
- Slideshow automation
- Looping navigation from the last photo to the first

## Selected approach

Reuse the wall's ordered, loaded result window behind a small photo-sequence interface.

When the user opens a tile, the viewer receives its stable asset ID and the wall controller's current ordered assets. It navigates within that list and asks the existing wall controller to load another page as it approaches the loaded edge. The viewer does not request the entire collection and does not build a second copy of the wall query.

This is the right first slice because the wall already owns sorting, pagination, derivative references, and stable asset identities. A narrow sequence interface keeps the viewer independent of wall internals and leaves room for a later backend neighbour-token implementation without changing viewer components.

Two alternatives were rejected for this slice:

- Loading every matching asset before opening would make large collections slow and memory-heavy.
- A dedicated backend viewer session would handle arbitrary jumps and remote portals well, but it adds session lifetime, neighbour tokens, and query consistency before the product needs them.

## Viewer lifecycle and wall preservation

The wall remains mounted when the viewer opens. The overlay covers it visually and prevents wall interaction, but it does not destroy wall rows, derivative state, pagination state, or the active sort direction.

Opening captures a return anchor containing the selected asset ID and the wall's exact scroll position. Closing removes the overlay, restores that position after the wall has completed any required layout measurement, and briefly highlights the selected tile. The highlight must not move the wall.

An available still-image tile opens the viewer. A catalog entry whose source cannot be confirmed and which has no usable derivative keeps the approved subdued unavailable treatment and does not open this viewer. Desktop locate/reconnect is a follow-up; the hosted web interface must never offer a local folder picker for recovery.

### Final review ruling (2026-08-26)

An unavailable catalog item with no usable cached derivative does not open this
viewer because `PhotoService` has no locate-folder capability. Desktop
locate/reconnect is a named follow-up, and the hosted web interface must not
offer local folder selection. The cost is explicit: until that capability is
built, the desktop item has no in-viewer recovery path.

Browser or system Back may close the overlay as a secondary path. It is not the primary mobile return control. A safe-area-aware back chevron appears at the top left whenever viewer controls are visible. Tapping the dark background does not close the viewer.

## Ordered sequence and navigation

The sequence preserves the wall's current visible order, including oldest-first or newest-first sorting. Clicking an asset opens that same position. The viewer does not independently resort results.

Navigation methods are equivalent:

- Left and Right arrow keys move to the previous or next asset.
- Visible previous and next buttons perform the same move.
- Selecting a filmstrip thumbnail moves directly to that loaded asset.
- A deliberate horizontal swipe moves one asset in the swipe direction.

Navigation does not wrap. At the first or last known result, the unavailable direction is disabled. If the user approaches the end of the loaded result window while the wall has more results, the sequence asks the wall controller to load another page. The direction remains disabled until the additional result is available, and one input cannot skip across multiple assets.

The selected filmstrip thumbnail stays centred when enough neighbours exist. At the ends, the strip naturally aligns to its available content. Moving the current asset updates the stage, metadata, accessible position announcement, filmstrip selection, and preview priorities as one state transition.

## Stage and derivative refinement

The viewer uses a fixed stage that fits the current image within the available viewport while preserving its aspect ratio. The stage geometry comes from catalog dimensions and does not depend on image decode completion.

For the first frame, the viewer chooses the best ready derivative in this order:

1. Ready screen preview
2. Ready wall thumbnail
3. Stable representative-colour or neutral placeholder

The viewer then explicitly requests the current asset's `screenPreview` derivative with visible priority. This must bypass idle-only screen-preview prefetch. The service contract therefore identifies the requested derivative class rather than assuming every request is for a wall thumbnail.

Once decoded, the screen preview crossfades over the existing layer inside the same stage. It must not change the photo's fitted bounds, shift controls, or alter filmstrip geometry. Reduced-motion mode replaces the crossfade with an immediate layer change.

On every navigation, the service prioritises work in this order:

1. Current asset screen preview at visible priority
2. Immediate previous and next asset screen previews at near-viewport priority
3. The next two neighbours in each direction when the app is idle

Navigation increments a preview request generation. A completion from an older generation may populate shared derivative state but must never replace the current stage image.

## Loading and failure behaviour

If a screen preview is already cached, it should appear within the existing 300-millisecond cached-preview target. If it is not ready, the wall thumbnail remains visible until refinement completes. The stage must not flash black or return to a colour block during navigation, sorting, resize, or rotation.

If generation of the larger preview fails, the viewer keeps the wall thumbnail and shows the restrained message `Larger preview unavailable` in the information drawer. Navigation remains available. A retry may occur through the normal derivative request lifecycle.

If no usable derivative exists but catalog dimensions are known, the stage shows the stable representative colour or neutral fill with the approved warning treatment. One damaged asset never blocks neighbours.

If the source share goes offline while the viewer is open, ready cached thumbnails and previews continue to work. Uncached assets retain their sequence positions and use unavailable treatment. The service reports path-free errors to shared UI code.

SQLite and the managed cache remain the only writable locations. The viewer, derivative generator, recovery flow, and tests must not write, rename, rate, tag, or delete source media.

## Information drawer

The Info control opens a right-side overlay drawer. It sits over the image rather than reducing or moving the stage. The drawer starts closed each time the viewer opens, remains open while the user navigates, and updates in place for the newly selected photo. Returning to the wall resets it to closed.

The first slice shows:

- Filename
- Resolved capture date and time
- Resolved rating
- Display dimensions after orientation
- File or media type

The catalog query must include resolved rating in the shared `WallAsset` data. Opening the drawer must not trigger a filesystem read or a separate metadata extraction request.

The drawer is non-modal. Keyboard and swipe navigation continue while it is open. On desktop it is about 320 pixels wide. On a phone it uses about 88 percent of the usable width so a strip of the photo remains visible. Content may scroll vertically without a vertical movement being interpreted as photo navigation.

Escape closes the drawer first. If it is already closed, Escape returns to the wall.

## Controls, filmstrip, and gestures

On pointer devices, pointer movement shows the primary controls. Moving into the bottom reveal zone also shows the filmstrip. Controls fade after about 2.5 seconds without interaction. The stage remains visible and uncluttered.

On touch devices, one tap on the image toggles the controls and filmstrip. They fade after about 3.5 seconds without interaction. Opening metadata requires a conscious tap on the Info control. An ordinary image tap never opens the drawer.

A horizontal swipe changes photo when horizontal travel clearly exceeds vertical travel and passes the navigation threshold. A swipe that begins inside the drawer may navigate horizontally, while dominant vertical movement scrolls the drawer. Taps on controls or filmstrip items do not start stage navigation.

This slice has a fit-to-window interaction state only. The gesture controller still accepts a view-state policy so later work can define `zoomed` behaviour. In that state, drag will pan and horizontal navigation swipes will be suppressed until the photo returns to fit.

The metadata drawer never fades while open. Its close and Info controls remain available even when the rest of the viewer controls hide.

## Responsive layout and device rotation

The viewer uses the live visual viewport and safe-area insets rather than assuming a fixed window size. Desktop resize and phone or tablet orientation change use the same relayout path.

When dimensions change, the viewer must:

1. Preserve the current asset, loaded sequence, metadata drawer state, and control visibility state.
2. Cancel any in-progress swipe presentation without navigating.
3. Recalculate the fitted image rectangle from catalog dimensions.
4. Recalculate filmstrip capacity and re-centre the selected thumbnail.
5. Recalculate drawer width and available drawer height.
6. Keep top controls inside the new safe area and the filmstrip above the bottom safe area.

The current derivative layers stay mounted during this relayout. Rotation must not replace a loaded preview with a thumbnail or placeholder, start another wall query, reset the user to the first photo, or create a geometry jump after the new viewport stabilises.

The implementation listens to both viewport resize and orientation-related visual viewport changes, then settles measurements through a single coalesced update. It must not use orientation as a content decision. Narrow and wide layouts derive from measured usable space so split-screen tablets and resized desktop windows behave correctly.

## Components and boundaries

The React implementation uses focused host-neutral units:

- `PhotoViewerOverlay` owns overlay lifecycle, return focus, and return-to-wall behaviour.
- `ViewerStage` owns fixed photo geometry and progressive derivative layers.
- `ViewerFilmstrip` renders loaded neighbours and keeps the current selection visible.
- `PhotoInfoDrawer` renders the approved read-only catalog metadata.
- `usePhotoViewer` or an equivalent reducer owns current asset ID, navigation, control visibility, drawer state, and request generations.
- `useViewerGestures` translates pointer and touch input into taps or navigation intent according to the current view-state policy.
- `PhotoSequence` exposes ordered assets, current position, boundary state, and `loadMore` without exposing wall implementation details.
- A viewport hook coalesces desktop resize, visual viewport changes, and device rotation into stable usable dimensions.

The viewer controller imports no Tauri API, reads no source path, and performs no direct network request. It talks only to `PhotoService` and the sequence interface. The Tauri adapter maps explicit derivative-class requests and catalog DTOs to the Rust application service. A later hosted adapter can implement the same operations over HTTP.

## State and data flow

Opening a wall tile follows this sequence:

1. The wall captures its selected asset ID and return scroll anchor.
2. The overlay opens above the mounted wall and receives the current ordered sequence.
3. The stage lays out from catalog dimensions and paints the best ready derivative.
4. The viewer requests the current screen preview at visible priority and adjacent previews at near-viewport priority.
5. A ready screen preview decodes and refines the fixed stage if the request generation still matches.
6. Navigation changes the stable current asset ID and repeats the priority update without clearing the stage to an empty state.
7. Approaching a loaded boundary asks the wall controller for its next SQLite page.
8. Closing restores the wall anchor and focus, then briefly highlights the selected tile.

The viewer state contains only UI and navigation state. Shared catalog and derivative readiness remain in the existing service-backed asset store so wall and viewer can benefit from each other's completed work.

## Accessibility

Opening the viewer moves focus into the overlay while remembering the originating tile. Closing returns focus to that tile when it remains mounted, or to the wall container if virtualization later removes it.

The overlay exposes an accessible viewer name. The current filename and position are announced when navigation settles, for example `DSC0123.jpg, photo 12 of 107 loaded`. The wording says `loaded` when more catalog results may exist so it does not report a false collection total.

Buttons have clear accessible names, visible focus treatment, and at least 44 by 44 pixel touch targets. The current filmstrip item uses `aria-current`. Disabled directions expose their unavailable state. The information drawer is labelled but does not trap focus because it is non-modal.

Reduced-motion settings disable drawer sliding, image crossfades, filmstrip scroll animation, and return-tile animation. Hidden controls cannot retain keyboard focus. Safe-area insets protect all interactive controls on notched devices in both orientations.

## Performance requirements

The existing targets remain in force:

- Ready cached screen previews should display within 300 milliseconds.
- Viewer navigation and transitions target 60 frames per second.
- A derivative refinement must not change stage geometry.
- Interaction immediately lowers unrelated background work.
- Shared UI performs no synchronous source read or image decode.

This slice adds these requirements:

- The first viewer frame uses an existing derivative or placeholder without waiting for the screen preview.
- Opening or closing the overlay does not rebuild the wall query.
- Navigation does not wait for adjacent prefetch to complete.
- Only a bounded neighbour window receives viewer preview priority.
- Resize and orientation changes use one coalesced measurement update and retain decoded derivative layers.
- The filmstrip renders only its useful visible and overscan window rather than every loaded result when the loaded wall page becomes large.

## Testing

### TypeScript unit tests

- Sequence navigation preserves order, stops at boundaries, and requests another page near a loaded edge.
- Viewer state opens at the selected asset ID and restores the correct return anchor.
- Information drawer state persists across navigation and resets after closing the viewer.
- Escape closes the drawer before closing the viewer.
- Preview request generations reject stale stage replacement.
- Derivative selection prefers screen preview, then wall thumbnail, then placeholder.
- Current and neighbouring assets receive the correct priorities and derivative class.
- Gesture classification distinguishes taps, horizontal swipes, drawer scrolls, and control interaction.
- A resize or orientation change cancels active swipe presentation without changing asset.
- Viewport relayout preserves the current asset, derivative layer, drawer state, and centred filmstrip selection.

### Browser and component tests

- Clicking a tile opens the corresponding photo without unmounting or re-querying the wall.
- Left and Right keys, buttons, and filmstrip clicks navigate consistently.
- Touch swipe navigates and a touch tap only toggles controls.
- Info opens only through its explicit control on mobile.
- Controls fade at the approved pointer and touch delays and remain usable through keyboard focus.
- The drawer overlays rather than resizes the stage and remains open during navigation.
- Closing restores exact wall scroll and briefly identifies the current tile.
- Cached refinement crossfades without geometry movement or a blank frame.
- Screen-preview failure leaves the thumbnail and navigation usable.
- Portrait-to-landscape and landscape-to-portrait rotation retain the same photo, preview quality, drawer state, and filmstrip selection.
- Desktop, tablet, and phone viewport sizes keep controls inside safe areas.
- Reduced-motion mode removes nonessential transitions.
- Automated accessibility checks report no serious violations in the viewer flow.

### Rust and desktop adapter tests

- Derivative requests carry an explicit wall-thumbnail or screen-preview class through DTO, command, and service layers.
- Visible screen-preview requests bypass idle-only prefetch while preserving bounded worker limits.
- Resolved rating serialises through the shared `WallAsset` contract.
- Cached preview delivery works while the source root is offline.
- Errors exposed to interface code do not leak native source paths.
- Source-operation audits confirm that production viewer and derivative paths remain read-only.

### macOS demonstration

The feature checkpoint uses the packaged or development Tauri app with a real indexed folder. It demonstrates:

1. Opening a wall tile and seeing an immediate cached image
2. Refinement to the larger screen preview without movement
3. Left and right keyboard navigation
4. Previous and next controls and filmstrip selection
5. Touch-sized horizontal swipe behaviour in WebKit
6. Explicit information drawer opening and metadata updates during navigation
7. Control fading and bottom filmstrip reveal
8. Portrait and landscape relayout without losing the photo or preview
9. Returning to the exact wall position and selected tile
10. Continued cached viewing after the source share becomes unavailable

The demonstration is the acceptance checkpoint for this vertical slice. Pan and zoom begin only after this viewer flow has been exercised and accepted.

## Acceptance criteria

- Any available wall tile opens the same asset in the wall's current order.
- A ready derivative paints immediately, and the screen preview refines it without a blank frame or geometry change.
- Keyboard, visible controls, filmstrip, and touch swipe navigate the same bounded sequence.
- Navigation does not wrap and can extend the loaded window through existing pagination.
- The filmstrip keeps the current asset visible and centred where possible.
- Controls hide and return according to the approved pointer and touch behaviour.
- Mobile metadata requires an explicit Info tap.
- The overlay drawer shows the approved catalog-backed fields and remains open across navigation.
- The back chevron restores the exact wall position and current tile without relying on browser Back.
- Mobile rotation and desktop resize preserve photo, derivative quality, navigation position, drawer state, and usable safe-area layout.
- Offline cached viewing and source read-only guarantees remain intact.
- The automated suites, format checks, static analysis, production build, and macOS demonstration pass before the slice is offered for merge.
