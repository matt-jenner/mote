import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { usePhotoWall } from "../app/usePhotoWall";
import type {
	Appearance,
	BootstrapState,
	ChooseFolderResult,
	DerivativeReference,
	DerivativeRequest,
	PhotoService,
	WallAsset,
	WallPage,
	WallQueryRequest,
	WallUpdate,
} from "../services/photoService";
import { AppShell } from "./AppShell";
import "../styles/tokens.css";
import "../styles/global.css";

interface Gate<T> {
	promise: Promise<T>;
	resolve: (value: T) => void;
	reject: (error: unknown) => void;
}

function gate<T>(): Gate<T> {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((nextResolve, nextReject) => {
		resolve = nextResolve;
		reject = nextReject;
	});
	return { promise, resolve, reject };
}

class TestIntersectionObserver {
	static instances: TestIntersectionObserver[] = [];
	static fireOnObserve = false;
	private readonly targets = new Set<Element>();
	constructor(
		private readonly callback: IntersectionObserverCallback,
		private readonly options?: IntersectionObserverInit,
	) {
		TestIntersectionObserver.instances.push(this);
	}
	observe(target: Element) {
		this.targets.add(target);
		if (TestIntersectionObserver.fireOnObserve) this.trigger([target]);
	}
	unobserve(target: Element) {
		this.targets.delete(target);
	}
	disconnect() {
		this.targets.clear();
	}
	takeRecords(): IntersectionObserverEntry[] {
		return [];
	}
	trigger(targets?: readonly Element[]) {
		const selected = targets ?? [...this.targets];
		this.callback(
			selected.map(
				(target) =>
					({
						isIntersecting: true,
						intersectionRatio: 1,
						target,
					}) as IntersectionObserverEntry,
			),
			this as unknown as IntersectionObserver,
		);
	}
	static trigger(
		kind: "visible" | "near" | "sentinel",
		root: Element,
		assetIds?: readonly string[],
	) {
		const matchesKind = (candidate: TestIntersectionObserver) => {
			const margin = candidate.options?.rootMargin ?? "";
			return (
				candidate.options?.root === root &&
				(kind === "visible"
					? margin === "0px"
					: kind === "near"
						? margin.includes("720")
						: margin.includes("480"))
			);
		};
		const candidates = [...TestIntersectionObserver.instances].reverse();
		const observers = candidates.filter(matchesKind);
		if (observers.length === 0) throw new Error(`Missing ${kind} observer`);
		if (kind === "sentinel") {
			const sentinel = root.querySelector(".loadSentinel") ?? root;
			for (const observer of observers) observer.trigger([sentinel]);
		} else {
			const targets = [
				...root.querySelectorAll<HTMLElement>("[data-asset-id]"),
			];
			const selected = assetIds
				? targets.filter((target) =>
						assetIds.includes(target.dataset.assetId ?? ""),
					)
				: targets;
			for (const observer of observers) observer.trigger(selected);
		}
	}
	static isObserving(kind: "visible" | "near", root: Element, assetId: string) {
		const target = root.querySelector(`[data-asset-id='${assetId}']`);
		if (!target) return false;
		return [...TestIntersectionObserver.instances]
			.reverse()
			.some((candidate) => {
				const margin = candidate.options?.rootMargin ?? "";
				const matchesKind =
					candidate.options?.root === root &&
					(kind === "visible" ? margin === "0px" : margin.includes("720"));
				return matchesKind && candidate.targets.has(target);
			});
	}
}

class ControlledWallService implements PhotoService {
	readonly capabilities = { chooseFolder: true, locateFolder: false };
	readonly queryRequests: WallQueryRequest[] = [];
	readonly derivativeRequests: DerivativeRequest[] = [];
	readonly interactionCalls: boolean[] = [];
	readonly gates: Gate<WallPage>[] = [];
	private readonly listeners = new Set<(update: WallUpdate) => void>();
	private readonly urls = new Map<string, string>();
	private readonly sourceState: BootstrapState;

	constructor(sourceId = "source-a") {
		this.sourceState = {
			settings: { appearance: "system" },
			activeSource: {
				id: sourceId,
				displayName: sourceId,
				availability: "available",
			},
		};
	}
	getBootstrapState = async () => structuredClone(this.sourceState);
	chooseFolder = async (): Promise<ChooseFolderResult> => ({
		kind: "selected",
		state: structuredClone(this.sourceState),
	});
	updateAppearance = async (appearance: Appearance) => ({
		...this.sourceState,
		settings: { appearance },
	});
	queryWall = (request: WallQueryRequest) => {
		this.queryRequests.push(request);
		const next = gate<WallPage>();
		this.gates.push(next);
		return next.promise;
	};
	requestDerivatives = async (request: DerivativeRequest) => {
		this.derivativeRequests.push({
			...request,
			assetIds: [...request.assetIds],
		});
	};
	setWallInteraction = async (active: boolean) => {
		this.interactionCalls.push(active);
	};
	watchWallUpdates = (listener: (update: WallUpdate) => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};
	derivativeUrl = (reference: DerivativeReference) => {
		const url =
			this.urls.get(reference.key) ?? `/demo-photos/${reference.assetId}.jpg`;
		if (!url) throw new Error("Derivative unavailable");
		return url;
	};
	releaseQuery = (index: number, page: WallPage) =>
		this.gates[index]?.resolve(page);
	rejectQuery = (index: number) =>
		this.gates[index]?.reject(new Error("native path / unavailable"));
	releaseThumbnail = (assetId: string, url: string) => {
		const reference = {
			assetId,
			kind: "wallThumbnail" as const,
			key: `${assetId}-wall`,
		};
		this.urls.set(reference.key, url);
		this.emit({ kind: "derivativesReady", derivatives: [reference] });
	};
	emit = (update: WallUpdate) => {
		for (const listener of this.listeners) listener(update);
	};
}

function asset(
	id: string,
	name: string,
	order: number,
	options: Partial<WallAsset> = {},
): WallAsset {
	const portrait = id === "portrait";
	return {
		id,
		displayName: name,
		mediaKind: "jpeg",
		provisionalOrder: order,
		capturedAtUtc: `2024-01-${String(order).padStart(2, "0")}T12:00:00Z`,
		dateState: "provisional",
		width: portrait ? 1024 : 1536,
		height: portrait ? 1536 : 1024,
		representativeRgb: 0x225670,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
		...options,
	};
}

const realFixtureAssets = [
	asset("coast", "Coast", 1),
	asset("forest", "Forest", 2),
	asset("city", "City", 3),
	asset("mountain", "Mountain", 4),
	asset("portrait", "Portrait", 5),
	asset("interior", "Interior", 6),
] as const;

const settledFixtures = realFixtureAssets.map((item) => ({
	...item,
	dateState: "settled" as const,
	wallThumbnail: {
		assetId: item.id,
		kind: "wallThumbnail" as const,
		key: `${item.id}-wall`,
	},
}));

const pageOf = (
	items: readonly WallAsset[],
	orderState: "provisional" | "settled" = "provisional",
	nextCursor: string | null = null,
): WallPage => ({
	items: [...items],
	orderState,
	nextCursor,
});

function renderWall(service: PhotoService) {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	return render(
		<QueryClientProvider client={client}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
}

function SourceSwapHarness() {
	const [sourceId, setSourceId] = useState("source-a");
	const wall = usePhotoWall(sourceId);
	return (
		<div>
			<button onClick={() => setSourceId("source-b")} type="button">
				Switch source
			</button>
			<div data-testid="swap-items">
				{wall.state.items.map((item) => item.id).join(",")}
			</div>
		</div>
	);
}

function renderSwap(service: PhotoService) {
	return render(
		<PhotoServiceProvider service={service}>
			<SourceSwapHarness />
		</PhotoServiceProvider>,
	);
}

function seriousViolations(result: axe.AxeResults) {
	return result.violations.filter(
		(entry) => entry.impact === "serious" || entry.impact === "critical",
	);
}

const originalIntersectionObserver = window.IntersectionObserver;
beforeEach(() => {
	TestIntersectionObserver.instances = [];
	Object.defineProperty(window, "IntersectionObserver", {
		configurable: true,
		value: TestIntersectionObserver,
	});
});
afterEach(() => {
	TestIntersectionObserver.fireOnObserve = false;
	Object.defineProperty(window, "IntersectionObserver", {
		configurable: true,
		value: originalIntersectionObserver,
	});
	document.documentElement.style.removeProperty("--fade-duration");
});

describe("progressive photo wall", () => {
	it("shows a complete row before metadata settles and preserves exact geometry through JPEG refinement", async () => {
		await page.viewport(1440, 1024);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(realFixtureAssets));
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		const tile = screen
			.getByTestId("photo-row-0")
			.element()
			.querySelector<HTMLElement>("[data-asset-id='coast']");
		await expect.element(screen.getByText("Indexing photos")).toBeVisible();
		expect(tile).not.toBeNull();
		if (!tile) return;
		const before = tile.getBoundingClientRect().toJSON();
		const colourLayer = tile.querySelector<HTMLElement>("div:nth-child(2)");
		expect(colourLayer).not.toBeNull();
		expect(colourLayer && getComputedStyle(colourLayer).backgroundColor).toBe(
			"rgb(34, 86, 112)",
		);
		TestIntersectionObserver.trigger(
			"visible",
			screen.getByRole("region", { name: "Photos" }).element(),
		);
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(0);
		service.releaseThumbnail("coast", "/demo-photos/coast.jpg");
		await expect
			.element(screen.getByRole("img", { name: "Coast" }))
			.toBeVisible();
		expect(tile.getBoundingClientRect().toJSON()).toEqual(before);
	});

	it("fences overlapping sort queries and shows newest first after resetting scroll", async () => {
		await page.viewport(834, 1194);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([...settledFixtures].reverse(), "settled"));
		const wall = screen.getByRole("region", { name: "Photos" });
		wall.element().scrollTop = 500;
		await expect.poll(() => wall.element().scrollTop).toBe(0);
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");
	});

	it("settles provisional newest-first data with the current direction and epoch", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([...realFixtureAssets].reverse()));
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		service.emit({ kind: "metadataSettled", sourceId: "source-a" });
		await expect.poll(() => service.queryRequests.length).toBe(3);
		expect(service.queryRequests[2]).toMatchObject({
			direction: "newestFirst",
			cursor: null,
			limit: 100,
		});
		service.releaseQuery(2, pageOf([...settledFixtures].reverse(), "settled"));
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");
	});

	it("loads a bounded second page and batches visible work before near-viewport work", async () => {
		await page.viewport(1440, 1024);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(realFixtureAssets, "settled", "cursor-2"));
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		TestIntersectionObserver.trigger("sentinel", wall.element());
		await expect.poll(() => service.queryRequests.length).toBe(2);
		expect(service.queryRequests[1]).toMatchObject({
			limit: 100,
			cursor: "cursor-2",
		});
		service.releaseQuery(
			1,
			pageOf(
				Array.from({ length: 30 }, (_, index) =>
					asset(`page-2-${index}`, `Page 2 ${index}`, index + 7),
				),
				"settled",
			),
		);
		await expect
			.poll(() => wall.element().querySelectorAll("[data-asset-id]").length)
			.toBe(36);
		expect(wall.element().scrollHeight).toBeGreaterThan(
			wall.element().clientHeight,
		);
		await expect
			.poll(() => wall.element().querySelector("[data-asset-id='coast']"))
			.not.toBeNull();
		await expect
			.poll(() =>
				TestIntersectionObserver.isObserving(
					"visible",
					wall.element(),
					"coast",
				),
			)
			.toBe(true);
		TestIntersectionObserver.trigger("visible", wall.element(), ["coast"]);
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) => request.priority === "visible",
				),
			)
			.toBe(true);
		expect(service.derivativeRequests).toContainEqual({
			assetIds: ["coast"],
			priority: "visible",
		});
		TestIntersectionObserver.trigger("near", wall.element(), ["forest"]);
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) => request.priority === "nearViewport",
				),
			)
			.toBe(true);
		const visible = service.derivativeRequests.find(
			(request) => request.priority === "visible",
		);
		const near = service.derivativeRequests.find(
			(request) => request.priority === "nearViewport",
		);
		expect(visible?.assetIds).toContain("coast");
		expect(near?.assetIds).not.toContain("coast");
	});

	it("re-observes replacement tiles after a responsive row regroup", async () => {
		await page.viewport(1440, 1024);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(realFixtureAssets, "settled"));
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.poll(() => wall.element().querySelector("[data-asset-id='coast']"))
			.not.toBeNull();
		const before = new Set([
			...wall.element().querySelectorAll("[data-asset-id]"),
		]);
		await page.viewport(390, 844);
		await expect
			.poll(() =>
				[...wall.element().querySelectorAll("[data-asset-id]")].some(
					(tile) => !before.has(tile),
				),
			)
			.toBe(true);
		const replacement = [
			...wall.element().querySelectorAll<HTMLElement>("[data-asset-id]"),
		].find((tile) => !before.has(tile));
		const replacementId = replacement?.dataset.assetId;
		expect(replacementId).toBeTruthy();
		if (!replacementId) return;
		await expect
			.poll(() =>
				TestIntersectionObserver.isObserving(
					"visible",
					wall.element(),
					replacementId,
				),
			)
			.toBe(true);
		TestIntersectionObserver.trigger("visible", wall.element(), [
			replacementId,
		]);
		TestIntersectionObserver.trigger("near", wall.element(), ["coast"]);
		await expect.poll(() => service.derivativeRequests.length).toBe(2);
		screen.unmount();
	});

	it("preserves visible-first dedupe when intersections share one RAF", async () => {
		const callbacks: FrameRequestCallback[] = [];
		const originalRequestAnimationFrame = window.requestAnimationFrame;
		const originalCancelAnimationFrame = window.cancelAnimationFrame;
		Object.defineProperty(window, "requestAnimationFrame", {
			configurable: true,
			value: (callback: FrameRequestCallback) => {
				callbacks.push(callback);
				return callbacks.length;
			},
		});
		Object.defineProperty(window, "cancelAnimationFrame", {
			configurable: true,
			value: () => undefined,
		});
		try {
			const service = new ControlledWallService();
			const screen = await renderWall(service);
			await expect.poll(() => service.queryRequests.length).toBe(1);
			service.releaseQuery(0, pageOf(realFixtureAssets, "settled"));
			const wall = screen.getByRole("region", { name: "Photos" });
			await expect
				.poll(() =>
					TestIntersectionObserver.isObserving(
						"visible",
						wall.element(),
						"coast",
					),
				)
				.toBe(true);
			TestIntersectionObserver.trigger("visible", wall.element(), [
				"coast",
				"forest",
			]);
			TestIntersectionObserver.trigger("near", wall.element(), [
				"forest",
				"city",
			]);
			expect(service.derivativeRequests).toEqual([]);
			callbacks.splice(0).forEach((callback) => {
				callback(0);
			});
			await expect.poll(() => service.derivativeRequests.length).toBe(2);
			expect(service.derivativeRequests[0]).toMatchObject({
				priority: "visible",
				assetIds: ["coast", "forest"],
			});
			expect(service.derivativeRequests[1]).toMatchObject({
				priority: "nearViewport",
				assetIds: ["city"],
			});
			screen.unmount();
		} finally {
			Object.defineProperty(window, "requestAnimationFrame", {
				configurable: true,
				value: originalRequestAnimationFrame,
			});
			Object.defineProperty(window, "cancelAnimationFrame", {
				configurable: true,
				value: originalCancelAnimationFrame,
			});
		}
	});

	it("keeps fallback geometry and populated-wall accessibility across required viewports", async () => {
		document.documentElement.style.setProperty("--fade-duration", "0ms");
		for (const [width, height] of [
			[1440, 1024],
			[834, 1194],
			[390, 844],
		] as const) {
			await page.viewport(width, height);
			const service = new ControlledWallService();
			const fallback = asset("missing", "Missing", 0, {
				shapeState: "fallback",
				width: 4,
				height: 3,
				availability: "missing",
			});
			const unavailableWithShape = asset("unreadable", "Unreadable", 7, {
				availability: "unreadable",
			});
			const screen = await renderWall(service);
			await expect.poll(() => service.queryRequests.length).toBe(1);
			service.releaseQuery(
				0,
				pageOf([fallback, unavailableWithShape, ...settledFixtures], "settled"),
			);
			await expect
				.poll(() => screen.getByTestId("photo-fallback").first().query())
				.not.toBeNull();
			expect(
				screen
					.getByRole("region", { name: "Photos" })
					.element()
					.querySelectorAll("[data-testid='photo-fallback']").length,
			).toBe(2);
			const fallbackBox = screen
				.getByTestId("photo-fallback")
				.first()
				.element()
				.getBoundingClientRect();
			expect(fallbackBox.width / fallbackBox.height).toBeCloseTo(4 / 3, 2);
			await expect
				.element(screen.getByText("File unavailable").first())
				.toBeVisible();
			expect(
				getComputedStyle(screen.getByTestId("photo-row-0").element())
					.animationDuration,
			).toBe("0s");
			expect(seriousViolations(await axe.run(document))).toEqual([]);
			screen.unmount();
		}
	});

	it("does not announce an empty result until scan completion and all pages are exhausted", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		service.releaseQuery(0, pageOf([]));
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Indexing photos");
		expect(
			screen
				.getByRole("region", { name: "Photos" })
				.getByText("No photos found", { exact: true })
				.query(),
		).toBeNull();
		service.emit({ kind: "metadataSettled", sourceId: "source-a" });
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([], "settled"));
		await expect
			.element(
				screen
					.getByRole("region", { name: "Photos" })
					.getByText("No photos found", { exact: true }),
			)
			.toBeVisible();
	});

	it("reports interaction quieting, retry, and source swaps without stale assets", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		service.rejectQuery(0);
		await expect
			.element(screen.getByRole("button", { name: "Retry" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Retry" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf(settledFixtures, "settled"));
		const wall = screen.getByRole("region", { name: "Photos" });
		wall.element().dispatchEvent(new Event("pointerdown"));
		wall.element().dispatchEvent(new Event("touchstart"));
		wall.element().dispatchEvent(new Event("wheel"));
		wall.element().dispatchEvent(new Event("scroll"));
		window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
		window.dispatchEvent(new Event("resize"));
		await expect.poll(() => service.interactionCalls.includes(true)).toBe(true);
		await expect.poll(() => service.interactionCalls.at(-1)).toBe(false);
		screen.unmount();
		expect(service.interactionCalls.at(-1)).toBe(false);
	});

	it("does not auto-retry a failed page when the sentinel is observed", async () => {
		TestIntersectionObserver.fireOnObserve = true;
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.rejectQuery(0);
		await expect
			.element(screen.getByRole("button", { name: "Retry" }))
			.toBeVisible();
		await expect.poll(() => service.queryRequests.length).toBe(1);
		screen.unmount();
	});

	it("fences an initial callback that is queued before unmount", async () => {
		const originalQueueMicrotask = window.queueMicrotask;
		const queued: VoidFunction[] = [];
		Object.defineProperty(window, "queueMicrotask", {
			configurable: true,
			value: (callback: VoidFunction) => queued.push(callback),
		});
		try {
			const service = new ControlledWallService();
			const screen = await renderWall(service);
			await screen.unmount();
			queued.splice(0).forEach((callback) => {
				callback();
			});
			await expect.poll(() => service.queryRequests.length).toBe(0);
		} finally {
			Object.defineProperty(window, "queueMicrotask", {
				configurable: true,
				value: originalQueueMicrotask,
			});
		}
	});

	it("fences a queued settlement callback before unmount", async () => {
		const originalQueueMicrotask = window.queueMicrotask;
		const queued: VoidFunction[] = [];
		Object.defineProperty(window, "queueMicrotask", {
			configurable: true,
			value: (callback: VoidFunction) => queued.push(callback),
		});
		try {
			const service = new ControlledWallService();
			const screen = await renderWall(service);
			queued.splice(0).forEach((callback) => {
				callback();
			});
			await expect.poll(() => service.queryRequests.length).toBe(1);
			service.emit({ kind: "metadataSettled", sourceId: "source-a" });
			service.releaseQuery(0, pageOf(realFixtureAssets));
			await expect.poll(() => queued.length).toBeGreaterThan(0);
			await screen.unmount();
			queued.splice(0).forEach((callback) => {
				callback();
			});
			await expect.poll(() => service.queryRequests.length).toBe(1);
		} finally {
			Object.defineProperty(window, "queueMicrotask", {
				configurable: true,
				value: originalQueueMicrotask,
			});
		}
	});

	it("does not emit a second false after timer cleanup on unmount", async () => {
		vi.useFakeTimers();
		try {
			const service = new ControlledWallService();
			const screen = await renderWall(service);
			await expect.poll(() => service.queryRequests.length).toBe(1);
			const wall = screen.getByRole("region", { name: "Photos" });
			wall.element().dispatchEvent(new Event("pointerdown"));
			await expect.poll(() => service.interactionCalls.at(-1)).toBe(true);
			screen.unmount();
			expect(service.interactionCalls).toEqual([true, false]);
			vi.advanceTimersByTime(201);
			expect(service.interactionCalls).toEqual([true, false]);
		} finally {
			vi.useRealTimers();
		}
	});

	it("ignores a pending source A completion after source B takes ownership", async () => {
		const service = new ControlledWallService();
		const screen = await renderSwap(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		await screen.getByRole("button", { name: "Switch source" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([asset("b", "B", 1)], "settled"));
		service.releaseQuery(0, pageOf([asset("a", "A", 1)], "settled"));
		await expect
			.poll(() => screen.getByTestId("swap-items").element().textContent)
			.toBe("b");
	});
});
