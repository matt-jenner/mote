import { useState } from "react";
import { flushSync } from "react-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { SourceUnavailableContext } from "../folders/SourceAvailabilityContext";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type { PhotoService, WallAsset } from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import type { PositionedWallAsset } from "../wall/layoutJustifiedRows";
import { PhotoTile } from "./PhotoTile";

interface Gate<T> {
	promise: Promise<T>;
	resolve: (value: T) => void;
}

function gate<T>(): Gate<T> {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((nextResolve) => {
		resolve = nextResolve;
	});
	return { promise, resolve };
}

function asset(overrides: Partial<WallAsset> = {}): WallAsset {
	return {
		id: "coast",
		displayName: "Coast.jpg",
		mediaKind: "jpeg",
		provisionalOrder: 1,
		capturedAtUtc: "2025-01-01T12:00:00Z",
		dateState: "settled",
		width: 1200,
		height: 800,
		representativeRgb: 0x3d536b,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
		rating: null,
		...overrides,
	};
}

function positioned(wallAsset: WallAsset): PositionedWallAsset {
	return { asset: wallAsset, height: 220, left: 0, width: 320 };
}

function photoService(): PhotoService {
	return {
		...createInMemoryPhotoService(),
		derivativeUrl: () => "/demo-photos/coast.jpg?pick-lifecycle=1",
	};
}

interface ImageRuntimeOverrides {
	complete: (image: HTMLImageElement) => boolean;
	naturalWidth: (image: HTMLImageElement) => number;
	decode: (image: HTMLImageElement) => Promise<void>;
}

function overrideImageRuntime(overrides: ImageRuntimeOverrides) {
	const descriptors = {
		complete: Object.getOwnPropertyDescriptor(
			HTMLImageElement.prototype,
			"complete",
		),
		naturalWidth: Object.getOwnPropertyDescriptor(
			HTMLImageElement.prototype,
			"naturalWidth",
		),
		decode: Object.getOwnPropertyDescriptor(
			HTMLImageElement.prototype,
			"decode",
		),
	};
	Object.defineProperty(HTMLImageElement.prototype, "complete", {
		configurable: true,
		get() {
			return overrides.complete(this as HTMLImageElement);
		},
	});
	Object.defineProperty(HTMLImageElement.prototype, "naturalWidth", {
		configurable: true,
		get() {
			return overrides.naturalWidth(this as HTMLImageElement);
		},
	});
	Object.defineProperty(HTMLImageElement.prototype, "decode", {
		configurable: true,
		value: function (this: HTMLImageElement) {
			return overrides.decode(this);
		},
	});
	return () => {
		for (const [property, descriptor] of Object.entries(descriptors))
			if (descriptor)
				Object.defineProperty(HTMLImageElement.prototype, property, descriptor);
	};
}

function overrideReducedMotion(matches: boolean) {
	const original = window.matchMedia;
	window.matchMedia = ((query: string) => {
		const list = original.call(window, query);
		if (query !== "(prefers-reduced-motion: reduce)") return list;
		return new Proxy(list, {
			get(target, property, _receiver) {
				if (property === "matches") return matches;
				const value = Reflect.get(target, property, target);
				return typeof value === "function" ? value.bind(target) : value;
			},
		});
	}) as typeof window.matchMedia;
	return () => {
		window.matchMedia = original;
	};
}

describe("photo tile pick lifecycle", () => {
	beforeEach(async () => {
		await page.viewport(1024, 768);
	});

	afterEach(() => {
		document.documentElement.dataset.theme = "system";
	});

	it("reveals add-to-picks only after the preview becomes interactive", async () => {
		const fixtureResponse = await fetch("/demo-photos/coast.jpg");
		expect(fixtureResponse.status).toBe(200);
		expect(fixtureResponse.headers.get("content-type")).toContain("image/jpeg");
		const onTogglePick = vi.fn();
		const service = photoService();
		const placeholder = await render(
			<PhotoTile
				onTogglePick={onTogglePick}
				positioned={positioned(asset())}
				service={service}
			/>,
		);
		expect(
			placeholder
				.getByRole("button", { name: "Add Coast.jpg to picks" })
				.query(),
		).toBeNull();
		await placeholder.unmount();

		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			decode: () => decodeGate.promise,
			naturalWidth: () => naturalWidth,
		});
		const restoreReducedMotion = overrideReducedMotion(false);
		try {
			const screen = await render(
				<PhotoTile
					onTogglePick={onTogglePick}
					positioned={positioned(
						asset({
							wallThumbnail: {
								assetId: "coast",
								key: "coast-wall",
								kind: "wallThumbnail",
							},
						}),
					)}
					service={service}
				/>,
			);
			const add = () =>
				screen.getByRole("button", { name: "Add Coast.jpg to picks" }).query();
			const image = screen.getByRole("img", { name: "Coast.jpg" });

			expect(add()).toBeNull();
			complete = true;
			naturalWidth = 320;
			image.element().dispatchEvent(new Event("load"));
			decodeGate.resolve();
			await expect
				.poll(() => getComputedStyle(image.element()).opacity)
				.toBe("1");
			expect(add()).toBeNull();

			flushSync(() => {
				image.element().dispatchEvent(
					new TransitionEvent("transitionend", {
						bubbles: true,
						propertyName: "opacity",
					}),
				);
			});
			await expect
				.element(screen.getByRole("button", { name: "Add Coast.jpg to picks" }))
				.toBeVisible();
		} finally {
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("keeps removal available after a picked preview loses its source", async () => {
		const service = photoService();
		function FailureHarness() {
			const [sourceFailed, setSourceFailed] = useState(false);
			return (
				<>
					<button onClick={() => setSourceFailed(true)} type="button">
						Simulate source failure
					</button>
					<PhotoTile
						onTogglePick={() => undefined}
						picked
						positioned={positioned(
							asset(
								sourceFailed
									? { availability: "missing", wallThumbnail: null }
									: {
											wallThumbnail: {
												assetId: "coast",
												key: "coast-wall",
												kind: "wallThumbnail",
											},
										},
							),
						)}
						service={service}
					/>
				</>
			);
		}
		const screen = await render(<FailureHarness />);
		const remove = () =>
			screen.getByRole("button", { name: "Remove Coast.jpg from picks" });

		await expect.element(remove()).toBeVisible();
		await screen
			.getByRole("button", { name: "Simulate source failure" })
			.click();
		await expect.element(screen.getByTestId("photo-fallback")).toBeVisible();
		await expect.element(remove()).toBeVisible();
	});

	it("hides a stale source warning after the folder becomes available", async () => {
		const screen = await render(
			<SourceUnavailableContext value={false}>
				<PhotoTile
					positioned={positioned(
						asset({
							warning: { code: "sourceUnavailable", retryable: true },
						}),
					)}
					service={photoService()}
				/>
			</SourceUnavailableContext>,
		);

		expect(
			screen
				.getByRole("img", { name: "Source unavailable. Showing cached image." })
				.query(),
		).toBeNull();
		expect(
			screen.getByRole("img", { name: "Photo preview warning" }).query(),
		).toBeNull();
	});

	it("keeps item warnings after the folder becomes available", async () => {
		const derivative = await render(
			<SourceUnavailableContext value={false}>
				<PhotoTile
					positioned={positioned(
						asset({
							warning: { code: "derivativeUnavailable", retryable: true },
						}),
					)}
					service={photoService()}
				/>
			</SourceUnavailableContext>,
		);
		await expect
			.element(derivative.getByRole("img", { name: "Photo preview warning" }))
			.toBeVisible();
		await derivative.unmount();

		const unreadable = await render(
			<SourceUnavailableContext value={false}>
				<PhotoTile
					positioned={positioned(
						asset({
							availability: "unreadable",
							warning: { code: "sourceUnreadable", retryable: true },
						}),
					)}
					service={photoService()}
				/>
			</SourceUnavailableContext>,
		);
		await expect
			.element(
				unreadable.getByRole("img", {
					name: "Source unavailable. Showing cached image.",
				}),
			)
			.toBeVisible();
	});
});
