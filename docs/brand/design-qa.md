# Mote Fredoka test page design QA

- Source visual truth: `/Users/jennerm/.codex/generated_images/01a05ee9-0e4c-73a2-83a8-0426df8935d5/exec-90b97972-cd49-44a1-b932-1ec9d4c16d7a.png`
- Implementation screenshot: `docs/brand/fredoka-test-page-viewport.jpg`
- Combined comparison: `docs/brand/fredoka-source-comparison.jpg`
- Viewport: 1280 x 720 CSS pixels
- Source pixels: 1486 x 1060
- Implementation pixels: 1280 x 720
- Density normalization: both images were reviewed at 1x. The combined comparison scales each complete image into an equal-width panel.
- State: Fredoka Regular 400, width axis 96%, light test-page theme.

## Full-view comparison evidence

The test page preserves the source palette: graphite `#171A1F`, cool white `#F7F8FA`, mineral grey `#B9C1C9`, and green `#45A06B`. The main wordmark remains the dominant element and is shown on both light and dark backgrounds. The page is a type test rather than a recreation of the complete icon board, so the source icons are intentionally absent.

## Focused region comparison evidence

The combined comparison puts the source wordmark and the Fredoka rendering in one browser capture. Fredoka Regular preserves the source's rounded stroke endings, open counters, circular `o`, and softened `t`. Fredoka's `e` is rounder and more consistent than the earlier candidates. Its `M` is slightly wider at the shoulders, but this is a minor difference and the page exposes weight and width controls for optical tuning.

## Required fidelity surfaces

- Fonts and typography: the browser reports Fredoka loaded at weight 400. Display text uses `-0.035em` tracking. Interface labels and controls retain the system font.
- Spacing and layout rhythm: the 1180-pixel desktop canvas uses consistent section dividers, 18-pixel cards, and paired light and dark lockups without document overflow.
- Colors and visual tokens: all four approved brand colors match the source values. Text and controls retain clear contrast.
- Image quality and asset fidelity: the test page contains no substituted icon artwork. The bundled variable font renders as live type at each sample size.
- Copy and content: the page uses the approved line `A simple space for your photos.`

## Primary interactions tested

- Weight control changed the live wordmark from 400 to 450 and updated its output value.
- Width control changed the live wordmark from 100% to 96% and updated its output value.
- The selected default is weight 400 and width 96%.
- `document.fonts.check("400 72px Fredoka")` returned `true`.

## Comparison history

1. Initial implementation, P2: a green full stop was added to the wordmark even though it was absent from the selected source.
2. Fix: removed the full stop from both light and dark lockups and the in-context sidebar mark.
3. Post-fix evidence: the rendered lockups contain only `Mote`, matching the source content. No P0, P1, or P2 findings remain.

## Follow-up polish

No P3 typography decisions remain for this test page. The selected width is 96%.

## Platform asset pack QA

- Source masters: `docs/brand/source/mote-symbol-master.svg`, `mote-wordmark-outlined.svg`, and `mote-lockup-master.svg`.
- Appearance comparison: `docs/brand/previews/mote-asset-contact-sheet.png`.
- Small-size comparison: `docs/brand/previews/mote-small-size-check.png`.
- Print reference: `docs/brand/print/mote-brand-sheet-a4.pdf`.
- Rendered print evidence: `docs/brand/previews/mote-brand-sheet-a4-1.png`.

The contact sheet confirms that the graphite wordmark reads on the light surface and the cool-white wordmark reads on graphite. Green remains `#45A06B` in both modes. Full-colour desktop and PWA icons use a cool-white rear frame and green foreground frame on the same graphite tile.

The approved symbol contains two equal `416 x 416` frames: a graphite or cool-white rear frame and a green foreground frame. The rear frame opens at its exposed top-right corner and the foreground frame opens at its exposed bottom-left corner. These crop-line endings break the continuous interlocking-square silhouette. The combined painted bounds remain centred on both axes of the master canvas. The centre dot sits wholly inside the shared open area and remains clear of both strokes at normal and small-icon weights.

The small-size sheet was inspected at 16, 24, 32, 48, and 64 px. The 16 px and 24 px sources use heavier strokes and wider crop-corner openings. The green foreground, rear frame, central green square, and open-corner treatment remain identifiable at each size.

Automated package checks confirmed:

- Windows ICO entries at 16, 24, 32, 48, 64, 128, and 256 px, all at 32-bit colour with alpha.
- macOS ICNS contains ten modern PNG representation records and unpacks through `iconutil` into the standard iconset names.
- Linux PNG dimensions match every `hicolor` directory, with full-colour and symbolic scalable SVGs present.
- PWA manifest paths, MIME types, dimensions, and `purpose` values match real files.
- TTF and WOFF2 family names, weight and width axes, and OpenType timestamps match.
- Every portable wordmark and lockup uses outlined paths with no live SVG text.
- The A4 PDF is one 595.276 x 841.89 pt page, passes text extraction checks, and has no clipped artwork in its 150 dpi render.

No P0, P1, or P2 findings remain.

final result: passed
