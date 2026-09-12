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
