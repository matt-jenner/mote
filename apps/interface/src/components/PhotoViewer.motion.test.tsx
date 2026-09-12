import { useState } from "react";
import { expect, it } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { PhotoService, WallAsset } from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import wallStyles from "../styles/photoWall.module.css";
import { ViewerNavigator } from "./ViewerNavigator";
import { ViewerStage } from "./ViewerStage";

it("retains pick feedback transitions when motion is allowed", async () => {
	const view = await render(
		<button
			className={wallStyles.tilePickButton}
			aria-pressed="true"
			type="button"
		>
			Picked
		</button>,
	);
	expect(window.matchMedia("(prefers-reduced-motion: reduce)").matches).toBe(
		false,
	);
	expect(
		getComputedStyle(view.getByRole("button", { name: "Picked" }).element())
			.transitionDuration,
	).not.toBe("0s");
});

const pixel =
	"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";

const asset: WallAsset = {
	id: "motion-photo",
	displayName: "Motion photo",
	mediaKind: "jpeg",
	provisionalOrder: 1,
	capturedAtUtc: "2024-01-01T12:00:00Z",
	dateState: "settled",
	width: 1200,
	height: 800,
	representativeRgb: 0x225670,
	shapeState: "ready",
	availability: "available",
	warning: null,
	wallThumbnail: null,
	screenPreview: null,
	rating: null,
};

const service = {} as PhotoService;

it("crossfades a decoded preview without changing fitted bounds", async () => {
	await page.viewport(800, 600);
	const originalDecode = HTMLImageElement.prototype.decode;
	let resolveDecode: (() => void) | null = null;
	HTMLImageElement.prototype.decode = function () {
		if (this.src.includes("#screen")) {
			return new Promise<void>((resolve) => {
				resolveDecode = resolve;
			});
		}
		return Promise.resolve();
	};
	try {
		const view = await render(
			<div style={{ display: "flex", height: "600px", width: "800px" }}>
				<ViewerStage
					asset={asset}
					baseUrl={`${pixel}#base`}
					currentUrl={`${pixel}#screen`}
					previewGeneration={1}
					service={service}
					viewportHeight={600}
					viewportWidth={800}
				/>
			</div>,
		);
		const frame = view.getByTestId("viewer-frame");
		const transformLayer = view.getByTestId("viewer-transform-layer").element();
		expect(getComputedStyle(transformLayer).transitionDuration).toBe("0s");
		await expect
			.poll(() => frame.element().getBoundingClientRect().width)
			.toBeGreaterThan(0);
		const boundsBefore = frame.element().getBoundingClientRect();
		const base = view
			.getByTestId("viewer-stage")
			.element()
			.querySelector("[data-viewer-layer='wallThumbnail']");
		const preview = view
			.getByTestId("viewer-stage")
			.element()
			.querySelector<HTMLImageElement>("[data-viewer-layer='screenPreview']");
		expect(base).not.toBeNull();
		expect(preview?.dataset.ready).toBe("false");
		expect(preview ? getComputedStyle(preview).opacity : "").toBe("0");
		expect(
			preview ? getComputedStyle(preview).transitionDuration : "0s",
		).not.toBe("0s");
		await expect.poll(() => resolveDecode !== null).toBe(true);
		const completeDecode = resolveDecode as (() => void) | null;
		if (!completeDecode) throw new Error("preview decode did not start");
		completeDecode();
		await expect.poll(() => preview?.dataset.ready).toBe("true");
		await new Promise((resolve) => setTimeout(resolve, 250));
		expect(preview ? getComputedStyle(preview).opacity : "").toBe("1");
		const boundsAfter = frame.element().getBoundingClientRect();
		expect(boundsAfter.width).toBe(boundsBefore.width);
		expect(boundsAfter.height).toBe(boundsBefore.height);
	} finally {
		HTMLImageElement.prototype.decode = originalDecode;
	}
});

it("keeps the navigator mounted and visible while manipulation is active", async () => {
	await page.viewport(800, 600);
	let active = false;
	let setActive: ((next: boolean) => void) | null = null;
	let setVisible: ((next: boolean) => void) | null = null;
	function Harness() {
		const [visible, updateVisible] = useState(true);
		const [manipulating, updateActive] = useState(false);
		setVisible = updateVisible;
		setActive = updateActive;
		active = manipulating;
		return (
			<ViewerNavigator
				assetName="Motion photo"
				imageHeight={800}
				imageUrl={null}
				imageWidth={1200}
				interactive
				onInteraction={() => undefined}
				onManipulationChange={updateActive}
				onRecenter={() => undefined}
				visible={visible || manipulating}
				visibleRect={{ x: 0.25, y: 0, width: 0.5, height: 1 }}
			/>
		);
	}
	const view = await render(<Harness />);
	const navigator = view.getByTestId("viewer-navigator").element();
	expect(getComputedStyle(navigator).transitionDuration).not.toBe("0s");
	const hide = setVisible as ((next: boolean) => void) | null;
	const activate = setActive as ((next: boolean) => void) | null;
	if (!hide || !activate) throw new Error("harness setters unavailable");
	hide(false);
	await expect.poll(() => getComputedStyle(navigator).opacity).toBe("0");
	expect(getComputedStyle(navigator).visibility).toBe("visible");
	expect(getComputedStyle(navigator).pointerEvents).toBe("none");
	activate(true);
	await expect.poll(() => active).toBe(true);
	hide(false);
	await expect.poll(() => getComputedStyle(navigator).opacity).toBe("1");
	await view.unmount();
});
