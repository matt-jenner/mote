# HEIF decoder fixtures

These are committed synthetic test data from Pillow-Heif, not photographs
from a user's private library. They are BSD-3-Clause, copyright 2021–2023
Pillow-Heif contributors; retain `LICENSE-Pillow-Heif.txt` when redistributing.
Mote's code keeps its own licence. No CC-BY-SA fixtures were imported.

The immutable upstream revision is
`efdbf036572e32c3202639b5516f842183242ec3` of
<https://github.com/bigcat88/pillow_heif>. The original licence is at
<https://github.com/bigcat88/pillow_heif/blob/efdbf036572e32c3202639b5516f842183242ec3/LICENSE.txt>.
`fixtures.json` records each source URL, final SHA-256, bit depth, display
dimensions, expected corners (top-left, top-right, bottom-left, bottom-right),
and derivation. No fixture has a capture timestamp. The `iphone-*` names
describe the supported encoding patterns, not a claimed capture device.

The 8-bit file is the unchanged upstream `RGB_8__29x100.heif`. The 10-bit
fixture starts from `RGB_10__29x100.heif` and has two distinct 64×100 tile
items that reference the same compressed bytes, assembled into a 93×100
primary grid. The extra tile is hidden and is not a second display image.
These fixtures intentionally remain small (under 20 KB combined).

`derive-fixtures.mjs` performs only container edits; it never invokes an
encoder or changes the HEVC bitstreams. Run it with a path to the original
upstream 10-bit file to reproduce all derived files, or with no argument
to reproduce only the rotation, P3, and truncation files from the committed
8-bit source. Downloads used for regeneration should be placed in a temporary
directory and removed afterwards. Do not pass the already-derived 10-bit
file as input. There are no generated build caches alongside these fixtures.

The P3 fixture assigns Display P3 primaries and sRGB transfer characteristics
to the source RGB values via NCLX (12/13/6, full range). Expected sRGB values
were calculated independently using the D65 P3-to-sRGB matrix and the IEC
61966-2-1 transfer function; the interior sample `[66,65,195]` becomes
`[66,65,203]`, which distinguishes conversion from simply tagging RGB bytes.
Other expected corners were measured with the direct native API before the
Mote backend existed. Rotation positions are the source corners permuted
90 degrees counter-clockwise.

## Known pinned-library edge case

The positive rotation fixture uses an even 28-pixel crop. libheif 1.23.4
mis-colours the upper corners when rotating the same YCbCr 4:2:0 grid at
odd width 29: its untransformed corners are `[0,6,252]`, `[2,252,253]`,
`[250,5,0]`, `[252,251,1]`; irot=1 returns upper corners `[75,200,255]`
and `[255,199,76]` instead of the rotated cyan and yellow. The untransformed
decode and the even-width transform are correct. This is an upstream
chroma/rotation limitation, not corrected by applying EXIF orientation.
The decoder currently inherits it; a native patch or separately managed
transform pipeline is needed to fix odd cropped grids. Common even-dimension
iPhone-style transforms remain covered. Do not widen the test tolerance to
hide this case.
