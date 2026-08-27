import { expect, it } from "vitest";
import { render } from "vitest-browser-react";
import { ViewerNavigator } from "./ViewerNavigator";
import "../styles/tokens.css";
import "../styles/global.css";

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
