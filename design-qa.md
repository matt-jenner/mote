# Canvas First shell design QA

- Source visual truth: `docs/superpowers/specs/assets/2026-08-24-macos-canvas-first.png`
- Implementation screenshot: `.superpowers/sdd/2026-08-24-macos-open-and-return/task-5-desktop.png`
- Viewport: 1440 x 1024 CSS pixels
- Source pixels: 1487 x 1058
- Implementation pixels: 1440 x 1024
- Density normalization: both captures are 1x. The source was assessed at a uniform 0.9684 scale to align its 1487-pixel width with the 1440-pixel implementation viewport.
- State: explicit Dark appearance, checkpoint 1 empty canvas. The temporary browser adapter cancels folder selection by design.

## Full-view comparison evidence

The implementation keeps the source image's dominant hierarchy: a narrow left rail, a restrained 72-pixel toolbar, thin separators, and a large uninterrupted matte canvas. The implementation rail is 72 pixels at desktop, within 2 pixels of the reference rail after normalization. The toolbar is 72 pixels tall, matching the normalized reference content toolbar.

The reference's native title/progress strip, indexing message, photo wall, search/filter controls, and image-size slider are intentionally absent. Checkpoint 1 has no indexing or raster-photo contract, and adding them would create false progress or fake imagery. The empty canvas uses one quiet centered action instead.

## Focused region comparison evidence

- Top-left shell: Lucide line icons use the same restrained weight as the reference. The selected folder source gets the approved `--selection` surface rather than a high-contrast accent block.
- Toolbar: source naming stays left aligned and the appearance action stays at the far edge, with no permanently open settings UI.
- Canvas: the matte background remains uninterrupted. The empty state is compact and centered instead of filling the deferred photo region with placeholders.
- Phone: the 390 x 844 capture removes the permanent navigation rail, keeps 44-pixel controls, and opens source controls in a modal drawer.
- Tablet: the 834 x 1194 capture measures a 56-pixel rail and a canvas extending from y=72 through y=1194.

## Comparison history

1. Initial implementation, P2: the canvas occupied only its content height because an optional error row shifted it into an auto-sized grid track. The 1440 x 1024 main region ended at y=232.5.
2. Fix: changed the workspace to a column flex layout and made the canvas the flexible region. Added browser bounds assertions at all three approved viewport sizes.
3. Post-fix evidence: desktop main bounds are y=72 to y=1024, tablet y=72 to y=1194, and phone y=64 to y=844. No horizontal or vertical document overflow was present in browser inspection.

## Findings

No actionable P0, P1, or P2 findings remain.

## Browser checks

- Appearance button and Dark radio changed `data-theme` and `color-scheme` to `dark`.
- Phone drawer trigger opened an accessible modal containing the same Sources navigation.
- Desktop, tablet, and phone layouts had no document overflow.
- Browser console errors and warnings: none.
- Automated axe scan: zero serious or critical violations.

## Follow-up polish

The photo wall, indexing progress, search/filter actions, and image scaling controls remain deferred until their service contracts exist.

final result: passed
