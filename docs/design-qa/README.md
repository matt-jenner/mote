# Mote shared interface design QA

- Source visual truth: [`mote-shared-ui/source-in-context.jpg`](mote-shared-ui/source-in-context.jpg)
- Implementation screenshot: [`mote-shared-ui/implementation-light-final.jpg`](mote-shared-ui/implementation-light-final.jpg)
- Combined comparison: [`mote-shared-ui/comparison-light.jpg`](mote-shared-ui/comparison-light.jpg)
- Dark-mode implementation: [`mote-shared-ui/implementation-dark.jpg`](mote-shared-ui/implementation-dark.jpg)
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

---

# Persistent photo picks — design QA

## Source targets

- `docs/superpowers/specs/assets/2026-09-12-pick-list-desktop-selected.png`
- `docs/superpowers/specs/assets/2026-09-12-pick-list-mobile-browse.png`
- `docs/superpowers/specs/assets/2026-09-12-pick-list-mobile-sheet.png`

The approved references define the interaction hierarchy and responsive form: a docked desktop panel, a persistent mobile Picks bar, and a modal mobile sheet. Fixture photos, filenames, and the existing application theme are not pixel-match targets.

## Rendered implementation evidence

Automated browser captures are in `apps/interface/.vitest-attachments/` for light and dark themes at 1440, 768, and 390 CSS pixels. Each viewport covers wall, open panel/sheet, review, and empty states; desktop native-copy progress and partial-failure states are also captured.

Representative files:

- `picks-light-1440-panel.png` — docked desktop panel, 1440 × 720
- `picks-dark-768-panel.png` — compact desktop/tablet panel, 768 × 720
- `picks-light-390-wall.png` — mobile wall and persistent Picks bar, 390 × 720
- `picks-light-390-panel.png` — mobile sheet, 390 × 720
- `picks-dark-1440-copy-progress.png` — desktop copy progress
- `picks-dark-1440-copy-partial.png` — desktop partial failure and retry

## Comparison history

1. Initial comparison found row Remove actions wrapping below thumbnails because the row rendered four grid children into three columns.
2. The action was moved into the details column and a measured mobile regression assertion was added.
3. Final captures preserve the approved hierarchy across all three widths: selection remains visible on the wall; the desktop panel is docked and non-modal; the mobile bar persists while browsing; the mobile sheet is modal and dismissible; review and explicit Clear remain available.
4. Light/dark WCAG A/AA axe checks, 44 px mobile target checks, keyboard focus restoration, reduced motion, forced-colour selection visibility, and safe-area behavior pass in the browser suite.

## Accepted visual differences

- The implementation uses the established Mote light/dark tokens and existing compact application chrome instead of reproducing the concept artwork's generated dark styling.
- Real fixture content produces fewer and plainer thumbnails than the concept images.
- Hosted mode exposes per-row original download links only when the server capability is enabled; it never displays the desktop bulk-copy button.

## Final result

passed
