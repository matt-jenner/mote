# Mote user guide

Mote opens photo folders without importing or reorganising them. The app stores
its catalogue, thumbnails, and screen previews separately from your originals.

## Add a folder

Select **Add folder** in the sidebar.

On Windows, macOS, and Linux, the system folder picker controls which folder
Mote can read. Mounted external drives and network shares work like local
folders as long as the operating system can read them.

On a hosted site, Mote shows folders below the root chosen by the server
administrator. It cannot browse above that root and does not reveal the
server's native path.

Opening a folder adds it to the sidebar. Use a saved folder's menu to rename or
remove that shortcut. Removing a shortcut does not delete photos, catalogue
records, or cached previews.

## Browse the photo wall

Mote displays thumbnails as soon as they are ready while indexing continues in
the background. Corrupt or unreadable photos receive their own warning and do
not stop healthy photos from loading.

Use the wall controls to:

- switch between oldest-first and newest-first order;
- include every nested folder or only the selected folder;
- retry a source after reconnecting a drive or share;
- open Picks.

Mote indexes video records but currently hides videos from the photo wall.

## Use the viewer

Open a wall tile to enter the viewer. Select the filmstrip, use the Previous and
Next buttons, swipe a fitted photo, or press the Left and Right arrow keys to
move through the loaded order.

Viewer controls:

| Action | Mouse, touch, or keyboard |
| --- | --- |
| Zoom in or out | Plus, Minus, `+`, `-`, or trackpad pinch |
| Return to fitted view | `0` or **Fit** |
| Toggle Fit and 100 percent | Double-click or double-tap the photo |
| Pan | Drag a zoomed photo |
| Show metadata | Open **Info** |
| Go back | Select **Back to photos** or press Escape |

Escape first closes Info, then resets zoom, then returns to the wall. Mote caps
zoom at the best generated preview's native pixel density. The viewer does not
load and enlarge an original file for extra zoom.

## Keep photos in Picks

Use a photo's pick control to add it to Picks. The list can contain photos from
several saved folders and keeps the order in which you added them.

On desktop, Picks live in the local catalogue and survive restarts. Select
**Copy originals** to copy the picked source files to another folder. Mote does
not overwrite existing files. It adds a numbered suffix when a name already
exists, and a cancelled batch keeps files that finished copying.

On hosted sites, Picks live in the browser's local storage for that site and
browser profile. Clearing site data removes them. An administrator can enable
**Download original** for one-file downloads, but it is off by default.

**Clear picks** removes the list and offers Undo for five seconds. Clearing the
list never deletes source files or completed copies.

## Work with disconnected folders

Mote keeps catalogue entries when a drive or network share goes offline.
Already generated thumbnails and previews remain available with a warning.
Photos without a cached image cannot open until the source returns.

Reconnect or remount the source at the same location, then select it again or
use Retry. Mote checks the source without discarding its existing catalogue.
Desktop folder relocation is not yet supported.

## What Mote writes

Mote treats configured photo sources as read-only. It never renames, moves, or
deletes files there. Desktop copies made through Picks go only to the separate
destination you choose.

Mote does write to its own locations:

- a SQLite catalogue that records folders, assets, metadata, and desktop Picks;
- a cache of generated JPEG thumbnails and screen previews;
- browser storage for hosted saved folders, preferences, and Picks.

If you run the hosted version, the administrator chooses the catalogue and
cache directories. See the [hosted storage guide](hosted/README.md#storage-and-permissions).
