import { expect, it } from "vitest";
import { render } from "vitest-browser-react";
import { ViewerNavigator } from "./ViewerNavigator";
import "../styles/tokens.css";
import "../styles/global.css";
import styles from "../styles/photoViewer.module.css";

it("preserves a visible selected pick control and keyboard outline in forced colors", async () => {
	const view = await render(
		<button className={styles.viewerPick} aria-pressed="true" type="button">
			Picked
		</button>,
	);
	const pick = view.getByRole("button", { name: "Picked" });
	pick.element().focus();
	const computed = getComputedStyle(pick.element());
	expect(computed.borderTopStyle).not.toBe("none");
	expect(Number.parseFloat(computed.borderTopWidth)).toBeGreaterThanOrEqual(1);
	expect(computed.color).not.toBe(computed.backgroundColor);
	expect(computed.outlineStyle).not.toBe("none");
	await expect.element(pick).toHaveAttribute("aria-pressed", "true");
});

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
