import { expect, it } from "vitest";
import { render } from "vitest-browser-react";
import { ViewerNavigator } from "./ViewerNavigator";
import "../styles/tokens.css";
import "../styles/global.css";
import styles from "../styles/photoViewer.module.css";

it("keeps the navigator viewport border visible in forced-colors mode", async () => {
	const view = await render(
		<ViewerNavigator
			assetName="Contrast photo"
			imageHeight={800}
			imageUrl={null}
			imageWidth={1200}
			interactive={false}
			onInteraction={() => undefined}
			onRecenter={() => undefined}
			visible
			visibleRect={{ x: 0.25, y: 0.25, width: 0.5, height: 0.5 }}
		/>,
	);
	const viewport = view
		.getByTestId("viewer-navigator")
		.element()
		.querySelector<HTMLElement>("[data-viewer-navigator-viewport]");
	if (!viewport) throw new Error("navigator viewport was not rendered");
	expect(window.matchMedia("(forced-colors: active)").matches).toBe(true);
	expect(getComputedStyle(viewport).borderTopWidth).toBe("2px");
	expect(getComputedStyle(viewport).borderTopColor).not.toBe("transparent");
	expect(getComputedStyle(viewport).boxShadow).toMatch(/2px/);
	await view.unmount();
});

it("keeps the viewer canvas and chrome dark under system-light appearance", async () => {
	const root = document.documentElement;
	const previousTheme = root.dataset.theme;
	const previousColorScheme = root.style.colorScheme;
	root.dataset.theme = "light";
	root.style.colorScheme = "light";
	try {
		const view = await render(
			<section className={styles.viewerOverlay} data-testid="viewer-overlay">
				<div className={styles.viewerStage} data-testid="viewer-canvas">
					<div className={styles.viewerFrame} />
				</div>
				<div className={styles.viewerChrome} data-testid="viewer-chrome">
					<button className={styles.viewerBack} type="button">
						Back
					</button>
				</div>
			</section>,
		);
		expect(
			getComputedStyle(view.getByTestId("viewer-overlay").element())
				.backgroundColor,
		).toBe("rgb(8, 9, 11)");
		expect(
			getComputedStyle(view.getByTestId("viewer-canvas").element())
				.backgroundColor,
		).toBe("rgb(8, 9, 11)");
		expect(
			getComputedStyle(view.getByRole("button", { name: "Back" }).element())
				.backgroundColor,
		).toBe("rgba(8, 9, 11, 0.62)");
		await view.unmount();
	} finally {
		if (previousTheme === undefined) delete root.dataset.theme;
		else root.dataset.theme = previousTheme;
		root.style.colorScheme = previousColorScheme;
	}
});
