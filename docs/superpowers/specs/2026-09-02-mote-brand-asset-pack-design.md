# Mote brand asset pack design

Date: 2026-09-02
Status: approved for implementation planning

## Purpose

Create a complete, reusable Mote identity pack under `docs/brand/`. The pack must support desktop packaging on macOS, Windows, and Linux, the hosted web and PWA service, light and dark presentation, and printable documentation.

The same symbol must remain recognisable on every platform. Platform variants may change canvas treatment, padding, masking, file format, and tiny-size optical details. They must not redraw the underlying mark into a different logo.

## Approved identity

- Product name: Mote
- Highlight: `#45A06B`
- Graphite: `#171A1F`
- Cool white: `#F7F8FA`
- Mineral grey: `#B9C1C9`
- Wordmark typeface: Fredoka Regular 400
- Wordmark width axis: 96%
- Brand line: `A simple space for your photos.`
- Source visual: `/Users/jennerm/.codex/generated_images/01a05ee9-0e4c-73a2-83a8-0426df8935d5/exec-90b97972-cd49-44a1-b932-1ec9d4c16d7a.png`

The master wordmark must be converted to vector outlines so it renders consistently without requiring a font installation. The original, unmodified Fredoka variable font must also be included with its SIL Open Font License for UI, web, and documentation use.

## Visual system

### Core symbol

The symbol uses three offset rectangular photo frames and a small central square. The frames overlap to suggest a compact stack of photographs without adding a literal camera, mountain, or aperture glyph.

The master uses a `1024 x 1024` coordinate system. It has enough internal spacing to remain distinct at 16 pixels. Corners and stroke endings follow the rounded Fredoka letterforms.

The full-colour symbol has these layers:

1. A rear graphite frame, offset toward the upper right.
2. A cool-white middle frame, offset toward the upper centre.
3. A green foreground frame, offset toward the lower left.
4. A small green central square.

The small-size master keeps the same geometry. At 16 and 24 pixels, it increases the frame stroke and central square by one optical pixel. No layer may be removed.

### Light and dark variants

Light-background assets use graphite text and a graphite rear frame. Dark-background assets use cool-white text and a cool-white rear frame. The green foreground remains `#45A06B` in both modes. Mineral grey is used only for secondary outlines and neutral samples.

Full-colour app icons keep a graphite tile in both modes so the application remains recognisable across launchers. Appearance-specific sources may lift the dark tile to `#22272D` or use a cool-white tile, but the mark geometry and green foreground remain unchanged.

Monochrome exports use solid graphite for light surfaces and solid white for dark surfaces. They must retain transparent backgrounds unless the target format requires an opaque canvas.

## Asset structure

```text
docs/brand/
├── README.md
├── source/
│   ├── mote-symbol-master.svg
│   ├── mote-wordmark-outlined.svg
│   └── mote-lockup-master.svg
├── svg/
│   ├── mote-symbol-light.svg
│   ├── mote-symbol-dark.svg
│   ├── mote-symbol-monochrome-dark.svg
│   ├── mote-symbol-monochrome-light.svg
│   ├── mote-wordmark-dark.svg
│   ├── mote-wordmark-light.svg
│   ├── mote-lockup-horizontal-light.svg
│   ├── mote-lockup-horizontal-dark.svg
│   ├── mote-lockup-stacked-light.svg
│   └── mote-lockup-stacked-dark.svg
├── fonts/
│   ├── Fredoka-Variable.ttf
│   ├── Fredoka-Variable.woff2
│   ├── OFL.txt
│   └── README.md
├── icons/
│   ├── macos/
│   │   ├── Mote.icns
│   │   ├── icon-1024.png
│   │   ├── icon-1024-dark.png
│   │   └── Mote.iconset/
│   ├── windows/
│   │   ├── Mote.ico
│   │   └── png/
│   ├── linux/
│   │   └── hicolor/
│   └── web/
│       ├── favicon.svg
│       ├── favicon.ico
│       ├── icon-192.png
│       ├── icon-512.png
│       ├── maskable-192.png
│       ├── maskable-512.png
│       ├── apple-touch-icon-180.png
│       ├── service-monochrome.svg
│       └── manifest-icons.json
├── print/
│   ├── mote-lockup-light.svg
│   ├── mote-lockup-dark.svg
│   ├── mote-lockup-light-3000.png
│   ├── mote-lockup-dark-3000.png
│   └── mote-brand-sheet-a4.pdf
└── previews/
    ├── mote-asset-contact-sheet.png
    └── mote-small-size-check.png
```

Existing font-test captures may remain in `docs/brand/`, but the README must distinguish them from production assets.

## Platform packages

### macOS

The source canvas is `1024 x 1024`. The main mark stays within the central safe region. The icon uses the graphite tile and a restrained outer shadow appropriate to the macOS Dock. The `.iconset` contains 16, 32, 128, 256, 512, and 1024 pixel representations with the required `@2x` filenames. `Mote.icns` is built from that set.

The dark appearance source uses `#22272D` for the tile so its edge remains visible against dark launch surfaces. The packaged `.icns` remains the default full-colour icon because the current Tauri configuration accepts a single `.icns`. The appearance-specific 1024-pixel source is retained for a future asset-catalog or Icon Composer workflow.

### Windows

`Mote.ico` contains 16, 24, 32, 48, 64, 128, and 256 pixel images. The 16 and 24 pixel images use the small-size optical master. All entries use 32-bit colour with alpha.

The Windows PNG directory contains each individual source size. Transparent outer corners are retained. The same graphite tile and central mark appear at every size.

### Linux

The Linux package follows the freedesktop `hicolor` layout with PNG files at 16, 24, 32, 48, 64, 128, 256, and 512 pixels under `apps`, plus a scalable SVG. The full-colour icon remains identical to the Windows artwork.

Theme-aware monochrome SVGs are supplied as `mote-symbolic.svg` and `mote-symbolic-dark.svg`. These are optional integration assets and do not replace the full-colour application icon.

### Hosted web and PWA

The web package includes:

- An SVG favicon with light and dark appearance rules.
- A multi-resolution favicon containing 16, 32, and 48 pixel images.
- Standard 192 and 512 pixel PWA icons.
- Maskable 192 and 512 pixel icons whose essential mark stays inside the central safe zone.
- A 180 pixel Apple touch icon.
- A monochrome SVG for browser or installed-app contexts that require a single-colour service mark.
- A manifest fragment declaring size, MIME type, and `purpose` for each PWA asset.

The hosted-service icon uses the same Mote symbol. It does not add server, cloud, globe, or network imagery.

## Font packaging

The pack includes the original Fredoka variable TTF and a WOFF2 conversion of the same variable font. The binary retains its original family name and axes. The pack does not create or distribute a renamed font derivative.

`OFL.txt` accompanies both files. `fonts/README.md` records the upstream Google Fonts path, licence, the selected wordmark settings, and sample CSS using `font-variation-settings: "wght" 400, "wdth" 96`.

## Print and documentation assets

The printable horizontal lockups use outlined wordmark paths. They require no installed fonts. Light and dark SVG versions use transparent backgrounds.

The 3000-pixel PNG exports provide convenient raster images for README files, release pages, and presentations. They include transparency and retain sRGB colours.

The A4 PDF brand sheet contains:

- Primary horizontal lockup on a light surface.
- Reversed horizontal lockup on graphite.
- Standalone symbol.
- Approved palette with colour values.
- Fredoka wordmark specification.
- Minimum-size and clear-space notes.

The PDF uses vector artwork where possible and embeds the required font information or outlined paths.

## Documentation

`docs/brand/README.md` is the entry point. It lists every deliverable and explains which asset to use for desktop packaging, Linux installation, PWA manifests, favicons, dark surfaces, light surfaces, monochrome contexts, and print.

It also records these usage rules:

- Do not recolour the green foreground.
- Do not change the order or offset of the frames.
- Do not add text inside an app icon.
- Do not use the dark wordmark on graphite or the light wordmark on white.
- Preserve the clear space defined by the central-square width.
- Use the outlined SVG wordmark for portable documents.
- Use the bundled font for editable text, with the licence retained.

## Generation architecture

The source SVG files are human-readable and remain the visual source of truth. A deterministic script under `docs/brand/tools/` generates platform PNGs, ICO, ICNS, print PNGs, contact sheets, and the PDF from those vectors and the bundled font.

Generated outputs must not be edited by hand. Any visual change starts in the source SVG or generation constants, then regenerates the full pack. This prevents platform assets from drifting apart.

The generator writes only inside `docs/brand/`. It must not overwrite `apps/desktop/src-tauri/icons/` or hosted application files. Integration into shipping builds is a separate reviewed change.

## Failure handling

Generation stops with a nonzero exit when a required font, source SVG, renderer, or output size is missing. It must not leave a partly updated platform package presented as complete.

The script writes generated files to a temporary staging directory first. It replaces final outputs only after all validation checks pass. Existing assets remain available if generation fails.

## Validation

Validation covers file structure and visible output:

- Parse every SVG as XML.
- Confirm portable wordmark and lockup SVGs contain outlined paths and no `<text>` elements.
- Confirm each raster image has the declared dimensions and colour mode.
- Confirm every Windows ICO size can be reopened.
- Confirm the macOS ICNS can be unpacked into its expected representations.
- Confirm the Linux directory names and icon sizes match.
- Confirm the PWA manifest paths, sizes, purposes, and MIME types match real files.
- Confirm the font files open, retain the Fredoka family name, and include weight and width axes.
- Confirm the SIL OFL file accompanies the font binaries.
- Render the A4 PDF to PNG, inspect it, and verify page size and text extraction.
- Inspect a contact sheet containing light, dark, monochrome, maskable, and 16, 32, and 64 pixel assets.
- Run repository formatting checks and `git diff --check`.

## Non-goals

- Renaming the product in Tauri configuration.
- Replacing the currently shipping desktop icon.
- Adding the PWA manifest to the hosted interface.
- Creating mobile-store, tvOS, watchOS, or visionOS packages.
- Registering trademarks or providing legal clearance for the Mote name or symbol.
