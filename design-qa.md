# Mote shared interface design QA

- Source visual truth: [`docs/design-qa/mote-shared-ui/source-in-context.jpg`](docs/design-qa/mote-shared-ui/source-in-context.jpg)
- Implementation screenshot: [`docs/design-qa/mote-shared-ui/implementation-light-final.jpg`](docs/design-qa/mote-shared-ui/implementation-light-final.jpg)
- Combined comparison: [`docs/design-qa/mote-shared-ui/comparison-light.jpg`](docs/design-qa/mote-shared-ui/comparison-light.jpg)
- Dark-mode implementation: [`docs/design-qa/mote-shared-ui/implementation-dark.jpg`](docs/design-qa/mote-shared-ui/implementation-dark.jpg)
- Viewport: 1280 x 720 CSS pixels
- Source pixels: 1280 x 720
- Implementation pixels: 1280 x 720
- Density normalization: both comparison captures use a 1x device pixel ratio and the same CSS viewport.
- State: empty library, explicit Light appearance. Dark appearance was checked separately at the same viewport.

## Full-view comparison evidence

The source is the approved `IN CONTEXT` sample from the Mote Fredoka type test. The implementation carries its graphite navigation, cool-white canvas, green selection and action accents, Fredoka display heading, uppercase library label, and approved empty-state copy into the existing product shell.

The product keeps its 72-pixel toolbar and full-height navigation because those areas contain working appearance, source-selection, photo-wall, and responsive-drawer behavior. Within that constraint, the 208-pixel navigation width is close to the source sample's 220-pixel column and the centered empty-state hierarchy matches the reference.

## Focused region comparison evidence

- Brand and typography: the navigation uses the approved outlined Mote wordmark. The empty-state heading loads the tracked Fredoka variable font at weight 400 with tightened tracking. System type remains on controls and supporting copy.
- Spacing and layout rhythm: desktop navigation uses a 208-pixel column, a 72-pixel product toolbar, and a centered content group. The final heading remains on one line at 1280 and 1440 pixels. At 390 x 844 it wraps cleanly to two lines with no clipping.
- Colors and visual tokens: the implementation uses Mote graphite `#171A1F`, cool white `#F7F8FA`, mineral grey `#B9C1C9`, and green `#45A06B`. The primary button uses the darker approved green `#347D52` so white button text retains a 4.5:1 contrast ratio.
- Image quality and asset fidelity: the wordmark comes from `docs/brand/svg/mote-wordmark-light.svg`; it is not reconstructed with HTML, CSS, or a substitute typeface. Existing Lucide folder and appearance icons remain product controls.
- Copy and content: the heading, supporting sentence, and `Choose folder` action match the selected source. `Folders` replaces the mock's `All photos` navigation label because it opens the real folder-selection flow.

## Comparison history

1. Initial Mote pass, P2: the empty-state container was 540 pixels wide, which forced the approved heading onto two lines at desktop widths while the source kept it on one line.
2. Fix: increased the desktop empty-state width to 720 pixels and added a browser regression assertion that the heading stays below 60 pixels tall at the 1440-pixel test viewport.
3. Post-fix evidence: `implementation-light-final.jpg` shows the heading on one line at 1280 x 720. The focused browser test passes at 1440 x 1024. No actionable P0, P1, or P2 findings remain.

## Findings

No actionable P0, P1, or P2 findings remain.

## Browser checks

- Appearance menu opened and closed correctly.
- Explicit Dark and Light choices updated the rendered theme.
- The primary `Choose folder` action completed its memory-mode cancellation path without an error.
- The 390 x 844 layout showed the compact toolbar and an accessible Mote sources drawer.
- Browser console errors and warnings: none.
- Focused browser test: 16 passed, including axe checks with zero serious or critical violations and regressions for focus-ring and rail-hover contrast.
- Full browser suite: 182 passed across 4 files.

## Follow-up polish

- The source sample omits the working product toolbar and folder icon. They remain intentionally because they preserve existing source selection and appearance behavior.

final result: passed
