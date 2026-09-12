# Persistent photo picks design

Date: 2026-09-12

Status: approved in conversation; ready for implementation planning

## Purpose

Let people collect photos into one persistent pick list while browsing several
saved folders, review that subset in the immersive viewer, and act on the
original files.

Desktop Mote copies the selected originals into a destination chosen by the
user. Hosted Mote keeps the same picking and review experience, stores the list
only in the current browser, and can expose a download link for each original.
Hosted original downloads are disabled by default and require an environment
setting plus a container restart to enable them.

The pick list is one working collection named **Picks**. Named lists, sharing,
server-side user accounts, and server-side pick storage are outside this
version.

## Agreed interaction model

- Picks persist across folder changes and app restarts.
- Desktop persistence is local to the Mote installation. Hosted persistence is
  local to the browser profile and site. The server does not store a user's
  picks.
- Adding, copying, or downloading never clears the list automatically.
- `Clear picks` acts immediately and offers a short-lived `Undo` action. It
  does not open a confirmation dialog.
- Removing a saved-folder shortcut does not remove picks from that folder.
  A pick remains until the user removes it or clears the list.
- If a source or original becomes unavailable, retain the pick with an
  unavailable state so the user can review a cached derivative when one exists
  or remove the stale pick.

## Approved visual direction

Use the selected docked-drawer direction shown in the
[desktop reference](assets/2026-09-12-pick-list-desktop-selected.png). The
[mobile browsing reference](assets/2026-09-12-pick-list-mobile-browse.png) and
[mobile sheet reference](assets/2026-09-12-pick-list-mobile-sheet.png) show the
responsive interaction.

These generated references define placement and hierarchy. They do not require
changes to the existing photo arrangement, warning state, source names, asset
filenames, toolbar controls, or image content. Keep the current Mote wordmark,
fonts, design tokens, justified wall, source rail, viewer, light and dark
themes, motion rules, and accessibility conventions.

### Wall selection

- Each photo tile has a separate pick toggle at its top-left corner. Existing
  warning badges keep their top-right position.
- Picked photos always show the selected control and a restrained green
  outline. On precise pointer devices, the unselected control may appear on
  hover and keyboard focus. It remains visible on touch devices.
- The pick toggle and the tile's open action are separate controls. Activating
  one must not trigger the other.
- Adding or removing a pick updates every visible instance immediately,
  including the wall, drawer, mobile sheet, filmstrip, and viewer.
- Adding a photo does not open the drawer. Show a brief confirmation such as
  `Added to picks`, with a quiet `View` action.

### Desktop drawer

- Add a persistent `Picks` button with the current count to the right side of
  the library toolbar. It remains available while switching saved folders.
- The button toggles a docked drawer on the right. The open button state is
  visually selected. The drawer also closes with its X or Escape.
- Closing the drawer never changes the pick list. Start each app launch with
  the drawer closed, even when persisted picks exist.
- Use a target width near 320px. Opening and closing the drawer reflows the
  justified wall rather than covering photos.
- Each drawer row contains a derivative thumbnail, filename, saved-folder
  label, and a remove control. A row whose source is unavailable uses the
  existing warning treatment without hiding cached imagery.
- Selecting a row thumbnail opens review mode at that item. The drawer header
  also offers `Review picks`.
- Keep action controls in a sticky footer. Desktop shows `Copy N originals...`
  as the primary action. `Clear picks` stays visually secondary and separate
  from the close control.
- An empty drawer explains how to add photos and does not show copy, review, or
  clear actions.

### Mobile and narrow screens

- Below the existing 900px navigation breakpoint, replace the right drawer
  with a bottom sheet. Do not place a desktop sidebar beside the wall.
- A 56px Picks bar above the safe area shows the count and opens the sheet. It
  remains available while browsing and does not cover the final photo row.
- The sheet rises to roughly 84 percent of the viewport. It has a drag handle,
  title, count, close button, scrollable pick rows, and a sticky action area.
- Close the sheet with its X, a downward swipe, backdrop activation, Escape,
  or the platform back action. Return focus to the Picks bar.
- Keep touch targets at least 44px and keep the copy or hosted action area above
  the bottom safe area.
- The sheet traps keyboard focus while open. The page behind it is inert.

## Review mode

- The wall viewer has an `Add to picks` or `Picked` toggle beside its existing
  controls. It remains usable when viewer chrome is revealed by keyboard,
  pointer, or touch input.
- `Review picks` opens the existing immersive viewer with a pick-only sequence.
  Previous, Next, keyboard navigation, swipe navigation, and the filmstrip move
  only through the current picks, including picks from different folders.
- Show `Picks` and the position, such as `3 of 6`, in the viewer chrome.
- Removing the current item advances to the next pick, or the previous pick
  when it was the last item. Removing the only remaining pick closes review to
  the empty drawer or sheet.
- Closing review restores the previously open drawer or sheet and the browsing
  folder's scroll position.
- Cached derivatives remain reviewable when their source is unavailable. An
  item without a usable derivative shows the existing unavailable state and
  remains removable.

## Desktop copy flow

1. `Copy N originals...` opens the native destination-folder picker every time.
2. When the operating system permits it, start the picker at the last
   destination used by a successful copy. Cancelling does not change that
   remembered destination.
3. Reject a destination that is the same as, or contained within, a configured
   photo source. Mote must continue treating every configured source as
   read-only.
4. Copy each original directly into the chosen destination. Do not reproduce
   source-folder structure and do not alter source bytes or metadata.
5. Never overwrite a destination file. Resolve collisions within the batch and
   against existing files with a numeric suffix before the extension, such as
   `IMG_2048 (2).jpg`.
6. Show determinate progress in the drawer and on the Picks toolbar button.
   Closing the drawer does not cancel the copy.
7. On success, report `Copied N originals` and offer `Show folder`. Keep all
   picks selected.
8. On partial success, report `Copied X of N` and mark the failed rows with a
   concise reason. Successful copies remain in place, failed picks remain
   selected, and a retry acts only on the failed items after the user chooses a
   destination again.

Only one copy operation may run at a time. Removing or clearing picks while a
copy is running changes the saved list but not the immutable batch already in
progress.

## Hosted behaviour

- Store the ordered pick identifiers in localStorage, scoped by site and hosted
  source-root identity. Tabs under the same browser profile and site share list
  edits through the browser storage event. Other browser profiles do not.
- The server bootstrap advertises whether original downloads are available.
  The interface must not infer availability from a failed request.
- Use `PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS` as the hosted environment setting.
  Missing, empty, `0`, or `false` keeps downloads disabled. Enabling requires a
  server or container restart.
- When downloads are enabled, each drawer or sheet row exposes one `Download
  original` link. This version does not trigger several browser downloads at
  once and does not create a ZIP archive.
- When downloads are disabled, omit all download links and download buttons.
  Picking, removing, clearing, drawer browsing, and immersive review continue
  to work. A short note may explain that this site does not offer original
  downloads.
- The server enforces the setting for every original-file request. Hiding the
  controls is not the security boundary.
- The download route accepts a catalog asset identifier, resolves it on the
  server, verifies that the canonical path stays within the configured hosted
  source root, and returns the original as an attachment. It never accepts an
  arbitrary filesystem path and never exposes an absolute source path.
- Missing, moved, unreadable, and out-of-root files return a safe unavailable
  response without leaking filesystem details.

## State and service boundaries

Use one shared pick-list model in the interface, independent of the current
wall selection. It owns ordered membership, count, add, remove, clear, undo,
and availability projection. The wall, drawer or sheet, and viewer consume that
model rather than keeping separate selection state.

Desktop backs the model with the local application store and resolves original
paths through native commands. Hosted Mote backs it with browser storage and
asks the server only for asset summaries, derivatives, and optional original
downloads. The current active folder must not change when Mote resolves or
reviews picks from another folder.

Stable catalog asset identifiers are the persisted membership keys. Persist no
absolute source paths in browser storage. Preserve insertion order so the
review sequence matches the order in which photos were picked.

## Accessibility and keyboard behaviour

- Every pick toggle exposes `Add {filename} to picks` or `Remove {filename}
  from picks` and uses `aria-pressed` for its state.
- Announce count changes and clear undo through a polite live region. Do not
  announce the full drawer after every selection.
- The Picks toolbar button exposes its count in its accessible name and uses
  `aria-expanded` and `aria-controls`.
- Drawer rows, remove controls, review, copy, download links, and clear are
  reachable in a predictable tab order.
- Opening and closing the drawer, sheet, or review mode restores focus to the
  control that launched it when that control still exists.
- Copy progress and completion use a live status without stealing focus.
- Reduced-motion mode removes drawer, sheet, selection, and toast animation
  while preserving state changes and announcements.

## Error handling

- A storage failure keeps the current in-memory picks usable and reports that
  they may not survive a restart.
- A removed or unavailable original remains visible in Picks with a warning
  and can be removed. Copy and download controls for that row are unavailable.
- A derivative failure does not remove an otherwise valid pick or prevent its
  original from being copied or downloaded.
- Closing Mote during a desktop copy may interrupt the unfinished batch. Files
  already copied remain valid, and a later retry must never overwrite them.
- A destination becoming unavailable produces a partial result rather than
  clearing picks or rolling back files that were copied successfully.

## Verification

- Unit-test ordered membership, duplicate adds, removal, immediate clear and
  undo, persistence recovery, stale entries, and cross-folder review order.
- Test desktop persistence across restarts and hosted persistence across reloads,
  tabs, sites, and separate browser profiles.
- Test native copy with temporary source and destination trees, same-name files,
  case-sensitive and case-insensitive collisions, partial failures, cancellation,
  retries, and remembered destinations.
- Hash source files and compare their metadata before and after copy tests.
  Verify that every source-root destination is rejected.
- Test the hosted capability with the environment setting absent, false, and
  true. Direct download requests must remain blocked while disabled.
- Test download path containment, encoded traversal attempts, symlink escapes,
  missing assets, attachment filenames, and filenames with non-ASCII characters.
- Browser-test wall and viewer toggles, drawer reflow, pick-only navigation,
  immediate clear and undo, focus restoration, reduced motion, live regions,
  mobile sheet dismissal, safe-area spacing, and download-disabled review.
- Visually inspect desktop light and dark themes at 1440px, the existing 768px
  breakpoint, and a 390px phone viewport. Verify that pick controls do not
  collide with warning badges or hide the tile open action.

## Out of scope

- Named or multiple pick lists
- Reordering picks by drag and drop
- Server-side pick storage, accounts, sync, or sharing
- Bulk hosted downloads or ZIP generation
- Moving, deleting, renaming, or editing source photos
- Copying into a configured photo source
- Recreating source-folder structure in the destination
- Automatic clearing after copy or download
