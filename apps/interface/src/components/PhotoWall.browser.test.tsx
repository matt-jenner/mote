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
	WallWarningState,
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
		return TestIntersectionObserver.isObservingElement(kind, root, target);
	}
	static isObservingElement(
		kind: "visible" | "near",
		root: Element,
		target: Element,
	) {
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
	derivativeRejectsRemaining = 0;
	derivativeHoldPriority: DerivativeRequest["priority"] | null = null;
	readonly heldDerivativeRequests: Array<Gate<void>> = [];
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
				selectionId: sourceId,
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
		if (this.derivativeRejectsRemaining > 0) {
			this.derivativeRejectsRemaining -= 1;
			throw new Error("derivative request rejected");
		}
		if (request.priority === this.derivativeHoldPriority) {
			const pending = gate<void>();
			this.heldDerivativeRequests.push(pending);
			await pending.promise;
		}
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
		this.emit({
			kind: "derivativesReady",
			selectionId: this.sourceState.activeSource?.selectionId ?? "",
			derivatives: [reference],
		});
	};
	emit = (update: WallUpdate) => {
		for (const listener of this.listeners) listener(update);
	};
}

function ControllerRequestHarness() {
	const wall = usePhotoWall("source-a");
	return (
		<button
			onClick={() => wall.requestVisibleDerivatives(["ready-controller"])}
			type="button"
		>
			Request thumbnail
		</button>
	);
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
	sourceWarnings: readonly WallWarningState[] = [],
): WallPage => ({
	items: [...items],
	orderState,
	nextCursor,
	sourceWarnings: [...sourceWarnings],
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
	const [selectionId, setSelectionId] = useState("selection-parent");
	const wall = usePhotoWall(selectionId);
	return (
		<div>
			<button onClick={() => setSelectionId("selection-child")} type="button">
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
const originalRequestIdleCallback = window.requestIdleCallback;
beforeEach(() => {
	TestIntersectionObserver.instances = [];
	Object.defineProperty(window, "IntersectionObserver", {
		configurable: true,
		value: TestIntersectionObserver,
	});
	Object.defineProperty(window, "requestIdleCallback", {
		configurable: true,
		value: originalRequestIdleCallback,
	});
});
afterEach(() => {
	TestIntersectionObserver.fireOnObserve = false;
	Object.defineProperty(window, "IntersectionObserver", {
		configurable: true,
		value: originalIntersectionObserver,
	});
	Object.defineProperty(window, "requestIdleCallback", {
		configurable: true,
		value: originalRequestIdleCallback,
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
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Preparing previews · 0 of 6");
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

	it("requests viewport, next rows, then remaining rows without observer callbacks", async () => {
		await page.viewport(1440, 520);
		const idleCallbacks: Array<
			(deadline: { didTimeout: boolean; timeRemaining: () => number }) => void
		> = [];
		Object.defineProperty(window, "requestIdleCallback", {
			configurable: true,
			value: (
				callback: (deadline: {
					didTimeout: boolean;
					timeRemaining: () => number;
				}) => void,
			) => {
				idleCallbacks.push(callback);
				return idleCallbacks.length;
			},
		});
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const assets = Array.from({ length: 24 }, (_, index) =>
			asset(`pass-${index}`, `Pass ${index}`, index + 1),
		);
		service.releaseQuery(0, pageOf(assets, "settled"));
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(0);
		const wall = screen.getByRole("region", { name: "Photos" }).element();
		const rowNodes = [
			...wall.querySelectorAll<HTMLElement>("[data-testid^='photo-row-']"),
		];
		const wallRect = wall.getBoundingClientRect();
		const visibleRows = rowNodes
			.map((row, index) => {
				const top =
					row.getBoundingClientRect().top - wallRect.top + wall.scrollTop;
				const bottom = top + row.getBoundingClientRect().height;
				return top < wall.scrollTop + wall.clientHeight &&
					bottom > wall.scrollTop
					? index
					: -1;
			})
			.filter((index) => index >= 0);
		const visibleEnd = visibleRows.at(-1) ?? 0;
		const nearRows = [visibleEnd + 1, visibleEnd + 2].filter(
			(index) => index < rowNodes.length,
		);
		const idsInRows = (indices: readonly number[]) =>
			indices.flatMap((index) =>
				[
					...(rowNodes[index]?.querySelectorAll<HTMLElement>(
						"[data-asset-id]",
					) ?? []),
				].map((tile) => tile.dataset.assetId ?? ""),
			);
		const expectedVisibleIds = idsInRows(visibleRows);
		const expectedNearIds = idsInRows(nearRows);
		const expectedRemainingIds = idsInRows(
			rowNodes
				.map((_row, index) => index)
				.filter(
					(index) => !visibleRows.includes(index) && !nearRows.includes(index),
				),
		);
		expect(service.derivativeRequests[0]).toEqual({
			assetIds: expectedVisibleIds,
			priority: "visible",
		});
		const near = service.derivativeRequests.find(
			(request) => request.priority === "nearViewport",
		);
		expect(near?.assetIds).toEqual(expectedNearIds);
		for (const callback of idleCallbacks.splice(0))
			callback({ didTimeout: false, timeRemaining: () => 50 });
		while (idleCallbacks.length > 0)
			for (const callback of idleCallbacks.splice(0))
				callback({ didTimeout: false, timeRemaining: () => 50 });
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(1);
		const idleRequests = service.derivativeRequests
			.filter((request) => request.priority === "nearViewport")
			.flatMap((request) => request.assetIds);
		expect(idleRequests).toEqual(expectedNearIds.concat(expectedRemainingIds));
		for (const request of service.derivativeRequests)
			expect(request.assetIds.length).toBeLessThanOrEqual(50);
	});

	it("defers the provisional remainder until settlement and promotes settled viewport first", async () => {
		await page.viewport(1440, 520);
		const idleCallbacks: Array<() => void> = [];
		Object.defineProperty(window, "requestIdleCallback", {
			configurable: true,
			value: (callback: () => void) => {
				idleCallbacks.push(callback);
				return idleCallbacks.length;
			},
		});
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const provisional = Array.from({ length: 60 }, (_, index) =>
			asset(`provisional-${index}`, `Provisional ${index}`, index + 1),
		);
		service.releaseQuery(0, pageOf(provisional, "provisional"));
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(0);
		expect(
			service.derivativeRequests.map((request) => request.priority),
		).toEqual(["visible", "nearViewport"]);
		expect(idleCallbacks).toHaveLength(0);

		service.emit({
			kind: "metadataSettled",
			selectionId: "source-a",
			sourceId: "source-a",
			generation: 1,
		});
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([...provisional].reverse(), "settled"));
		const settledStart = service.derivativeRequests.length;
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(settledStart);
		expect(service.derivativeRequests[settledStart]?.priority).toBe("visible");
		const settledIdle = idleCallbacks.splice(0);
		expect(settledIdle.length).toBeGreaterThan(0);
		for (const callback of settledIdle) callback();
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(settledStart + 1);
		expect(service.derivativeRequests[settledStart + 1]?.priority).toBe(
			"nearViewport",
		);
		screen.unmount();
	});

	it("does not submit an idle request for a thumbnail that became ready before the flush", async () => {
		await page.viewport(1440, 520);
		const idleCallbacks: Array<() => void> = [];
		Object.defineProperty(window, "requestIdleCallback", {
			configurable: true,
			value: (callback: () => void) => {
				idleCallbacks.push(callback);
				return idleCallbacks.length;
			},
		});
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const assets = Array.from({ length: 60 }, (_, index) =>
			asset(`idle-ready-${index}`, `Idle ready ${index}`, index + 1),
		);
		service.releaseQuery(0, pageOf(assets, "settled"));
		await expect.poll(() => idleCallbacks.length).toBeGreaterThan(0);
		const initialIds = new Set(
			service.derivativeRequests.flatMap((request) => request.assetIds),
		);
		const candidate = assets.find((item) => !initialIds.has(item.id));
		expect(candidate).toBeDefined();
		if (!candidate) return;
		const initialRequestCount = service.derivativeRequests.length;
		service.releaseThumbnail(candidate.id, `/demo-photos/${candidate.id}.jpg`);
		for (const callback of idleCallbacks.splice(0)) callback();
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(
			service.derivativeRequests
				.slice(initialRequestCount)
				.flatMap((request) => request.assetIds),
		).not.toContain(candidate.id);
		screen.unmount();
	});

	it("does not let a rejected near request erase a newer visible promotion", async () => {
		const service = new ControlledWallService();
		service.derivativeHoldPriority = "nearViewport";
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const assets = Array.from({ length: 24 }, (_, index) =>
			asset(`promote-${index}`, `Promote ${index}`, index + 1),
		);
		service.releaseQuery(0, pageOf(assets, "settled"));
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) => request.priority === "nearViewport",
				),
			)
			.toBe(true);
		const near = service.derivativeRequests.find(
			(request) => request.priority === "nearViewport",
		);
		const promoted = near?.assetIds[0];
		expect(promoted).toBeDefined();
		if (!promoted) return;
		TestIntersectionObserver.trigger(
			"visible",
			screen.getByRole("region", { name: "Photos" }).element(),
			[promoted],
		);
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) =>
						request.priority === "visible" &&
						request.assetIds.includes(promoted),
				),
			)
			.toBe(true);
		service.heldDerivativeRequests[0]?.reject(new Error("near failed"));
		await new Promise((resolve) => window.setTimeout(resolve, 100));
		expect(
			service.derivativeRequests.filter(
				(request) =>
					request.priority === "nearViewport" &&
					request.assetIds.includes(promoted),
			),
		).toHaveLength(1);
		screen.unmount();
	});

	it("filters already-ready thumbnails at the controller boundary", async () => {
		const service = new ControlledWallService();
		const screen = await render(
			<PhotoServiceProvider service={service}>
				<ControllerRequestHarness />
			</PhotoServiceProvider>,
		);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf(
				[
					asset("ready-controller", "Ready controller", 1, {
						wallThumbnail: {
							assetId: "ready-controller",
							kind: "wallThumbnail",
							key: "ready-controller-wall",
						},
					}),
				],
				"settled",
			),
		);
		await screen.getByRole("button", { name: "Request thumbnail" }).click();
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(service.derivativeRequests).toHaveLength(0);
		screen.unmount();
	});

	it("renders an accessible static indeterminate progress track with reduced motion", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf([]));
		const progress = screen.getByRole("progressbar").element();
		expect(progress.getAttribute("aria-label")).toBe("Photo preview progress");
		expect(progress.hasAttribute("value")).toBe(false);
		expect(getComputedStyle(progress).appearance).toBe("none");
		expect(getComputedStyle(progress).animationName).toBe("none");
	});

	it("retries a missing thumbnail after changing sort direction", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf([asset("retry", "Retry", 1)], "settled"));
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(0);
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([asset("retry", "Retry", 1)], "settled"));
		await expect
			.poll(
				() =>
					service.derivativeRequests.filter(
						(request) => request.priority === "visible",
					).length,
			)
			.toBe(2);
		expect(service.derivativeRequests.at(-1)).toMatchObject({
			assetIds: ["retry"],
			priority: "visible",
		});
	});

	it("retries a rejected visible thumbnail request and reports the retry", async () => {
		const service = new ControlledWallService();
		service.derivativeRejectsRemaining = 1;
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf([asset("retry-visible", "Retry visible", 1)], "settled"),
		);
		await expect.poll(() => service.derivativeRequests.length).toBe(1);
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Retrying previews");
		await expect.poll(() => service.derivativeRequests.length).toBe(2);
		expect(service.derivativeRequests.at(-1)).toMatchObject({
			assetIds: ["retry-visible"],
			priority: "visible",
		});
	});

	it("does not re-request a thumbnail after it becomes ready", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf([asset("ready-once", "Ready once", 1)], "settled"),
		);
		await expect.poll(() => service.derivativeRequests.length).toBe(1);
		service.releaseThumbnail("ready-once", "/demo-photos/ready-once.jpg");
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Photos ready");
		const wall = screen.getByRole("region", { name: "Photos" });
		TestIntersectionObserver.trigger("visible", wall.element(), ["ready-once"]);
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(service.derivativeRequests).toHaveLength(1);
	});

	it("does not retry wall thumbnails for a screen-preview warning", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf([asset("screen-warning", "Screen warning", 1)], "settled"),
		);
		await expect.poll(() => service.derivativeRequests.length).toBe(1);
		service.emit({
			kind: "warning",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "screen-warning",
			warning: { code: "screenPreviewUnavailable", retryable: true },
		});
		const wall = screen.getByRole("region", { name: "Photos" });
		TestIntersectionObserver.trigger("visible", wall.element(), [
			"screen-warning",
		]);
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(service.derivativeRequests).toHaveLength(1);
	});

	it("keeps loaded images painted while a replacement sort query is pending", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		await expect.element(screen.getByRole("img").first()).toBeVisible();
		const wall = screen.getByRole("region", { name: "Photos" });
		wall.element().scrollTop = 500;
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		expect(wall.element().getAttribute("aria-busy")).toBe("true");
		expect(screen.getByRole("img").first().element().getAttribute("alt")).toBe(
			"Coast",
		);
		service.releaseQuery(1, pageOf([...settledFixtures].reverse(), "settled"));
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");
		expect(wall.element().scrollTop).toBe(0);
	});

	it("keeps fixture thumbnails visible after responsive row regrouping", async () => {
		await page.viewport(1440, 900);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.poll(() => wall.element().querySelectorAll("img").length)
			.toBe(realFixtureAssets.length);
		await expect
			.element(screen.getByRole("img", { name: "Coast" }))
			.toBeVisible();
		const beforeRows = wall
			.element()
			.querySelectorAll("[data-testid^='photo-row-']").length;

		await page.viewport(320, 900);
		window.dispatchEvent(new Event("resize"));
		await expect
			.poll(
				() =>
					wall.element().querySelectorAll("[data-testid^='photo-row-']").length,
			)
			.not.toBe(beforeRows);
		await expect
			.poll(() =>
				[...wall.element().querySelectorAll<HTMLImageElement>("img")].every(
					(image) => getComputedStyle(image).opacity === "1",
				),
			)
			.toBe(true);
		screen.unmount();
	});

	it("shows contact-preview and larger-preview progress stages", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.emit({
			kind: "progress",
			selectionId: "source-a",
			generation: 1,
			progress: { discovered: 4, shaped: 4, enriched: 4, total: 4 },
		});
		service.releaseQuery(0, pageOf(realFixtureAssets.slice(0, 4), "settled"));
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Preparing previews · 0 of 4");
		for (const item of realFixtureAssets.slice(0, 4))
			service.releaseThumbnail(item.id, `/demo-photos/${item.id}.jpg`);
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Photos ready · preparing larger previews · 0 of 4");
		expect(screen.getByRole("progressbar").element()).toHaveAttribute(
			"max",
			"4",
		);
		expect(screen.getByRole("progressbar").element()).toHaveAttribute(
			"value",
			"0",
		);
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

	it("retains thumbnail references through a deferred settlement after sort", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		await expect
			.poll(
				() =>
					screen
						.getByRole("region", { name: "Photos" })
						.element()
						.querySelectorAll("img").length,
			)
			.toBe(6);
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.emit({
			kind: "metadataSettled",
			selectionId: "source-a",
			sourceId: "source-a",
			generation: 2,
		});
		service.releaseQuery(1, pageOf([...settledFixtures].reverse(), "settled"));
		await expect.poll(() => service.queryRequests.length).toBe(3);
		const wall = screen.getByRole("region", { name: "Photos" });
		expect(wall.element().querySelectorAll("img").length).toBe(6);
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");
		service.releaseQuery(
			2,
			pageOf(
				[...settledFixtures].reverse().map((item) => ({
					...item,
					wallThumbnail: null,
					screenPreview: null,
				})),
				"settled",
			),
		);
		await expect
			.poll(() => wall.element().querySelectorAll("img").length)
			.toBe(6);
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Photos ready · preparing larger previews · 0 of 6");
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
		service.emit({
			kind: "metadataSettled",
			selectionId: "source-a",
			sourceId: "source-a",
			generation: 1,
		});
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
		expect(
			service.derivativeRequests.some(
				(request) =>
					request.priority === "visible" && request.assetIds.includes("coast"),
			),
		).toBe(true);
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
		const before = new Set<HTMLElement>([
			...wall.element().querySelectorAll<HTMLElement>("[data-asset-id]"),
		]);
		for (const [width, height] of [
			[390, 844],
			[1440, 1024],
			[390, 844],
		] as const) {
			await page.viewport(width, height);
			await expect
				.poll(() =>
					[
						...wall.element().querySelectorAll<HTMLElement>("[data-asset-id]"),
					].some((tile) => !before.has(tile)),
				)
				.toBe(true);
			const current = new Set([
				...wall.element().querySelectorAll<HTMLElement>("[data-asset-id]"),
			]);
			const removed = [...before].filter((tile) => !current.has(tile));
			expect(removed.length).toBeGreaterThan(0);
			for (const tile of removed) {
				expect(
					TestIntersectionObserver.isObservingElement(
						"visible",
						wall.element(),
						tile,
					),
				).toBe(false);
				expect(
					TestIntersectionObserver.isObservingElement(
						"near",
						wall.element(),
						tile,
					),
				).toBe(false);
			}
			const replacement = [...current].find((tile) => !before.has(tile));
			expect(replacement).toBeTruthy();
			if (!replacement) return;
			await expect
				.poll(() =>
					TestIntersectionObserver.isObservingElement(
						"visible",
						wall.element(),
						replacement,
					),
				)
				.toBe(true);
			before.clear();
			for (const tile of current) before.add(tile);
		}
		const requestsBeforeIntersections = service.derivativeRequests.length;
		TestIntersectionObserver.trigger("visible", wall.element(), ["coast"]);
		TestIntersectionObserver.trigger("near", wall.element(), ["forest"]);
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThanOrEqual(requestsBeforeIntersections);
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
			expect(service.derivativeRequests.length).toBeGreaterThan(0);
			callbacks.splice(0).forEach((callback) => {
				callback(0);
			});
			await expect
				.poll(() => service.derivativeRequests.length)
				.toBeGreaterThan(0);
			expect(
				service.derivativeRequests.some(
					(request) =>
						request.priority === "visible" &&
						request.assetIds.includes("coast"),
				),
			).toBe(true);
			expect(
				service.derivativeRequests.flatMap((request) => request.assetIds),
			).toContain("city");
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
		service.emit({
			kind: "metadataSettled",
			selectionId: "source-a",
			sourceId: "source-a",
			generation: 1,
		});
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([], "settled"));
		await expect
			.element(
				screen
					.getByRole("region", { name: "Photos" })
					.getByText("No photos found", { exact: true }),
			)
			.toBeVisible();
		expect(screen.getByRole("progressbar").element()).not.toHaveAttribute(
			"max",
		);
		expect(screen.getByRole("progressbar").element()).not.toHaveAttribute(
			"value",
		);
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
			service.emit({
				kind: "metadataSettled",
				selectionId: "source-a",
				sourceId: "source-a",
				generation: 1,
			});
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

	it("ignores a pending parent completion after a same-library child selection", async () => {
		const service = new ControlledWallService();
		const screen = await renderSwap(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		await screen.getByRole("button", { name: "Switch source" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		await expect
			.poll(() => screen.getByTestId("swap-items").element().textContent)
			.toBe("");
		service.releaseQuery(1, pageOf([asset("b", "B", 1)], "settled"));
		service.releaseQuery(0, pageOf([asset("a", "A", 1)], "settled"));
		service.emit({
			kind: "catalogBatch",
			selectionId: "selection-parent",
			assets: [asset("stale", "Stale parent", 1)],
			orderState: "provisional",
			generation: 1,
			progress: { discovered: 1, shaped: 1, enriched: 0, total: 1 },
		});
		await expect
			.poll(() => screen.getByTestId("swap-items").element().textContent)
			.toBe("b");
	});

	it("resyncs the current direction after a typed lag notification", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([...settledFixtures].reverse(), "settled"));
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");

		service.emit({ kind: "resyncRequired", selectionId: "source-a" });
		await expect.poll(() => service.queryRequests.length).toBe(3);
		expect(service.queryRequests[2]).toMatchObject({
			cursor: null,
			direction: "newestFirst",
		});
		service.releaseQuery(2, pageOf([...settledFixtures].reverse(), "settled"));
		await expect
			.element(screen.getByRole("img").first())
			.toHaveAttribute("alt", "Interior");
	});

	it("restores and independently clears source cache warnings from page snapshots", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const sourceWarnings = [
			{ code: "wallThumbnailCacheUnavailable", retryable: true },
			{ code: "screenPreviewCacheUnavailable", retryable: true },
		] as const;
		service.releaseQuery(
			0,
			pageOf(settledFixtures, "settled", null, sourceWarnings),
		);
		await expect
			.element(screen.getByText("Some previews need attention"))
			.toBeVisible();

		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.emit({
			kind: "warningCleared",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: null,
			code: "wallThumbnailCacheUnavailable",
		});
		service.releaseQuery(
			1,
			pageOf([...settledFixtures].reverse(), "settled", null, sourceWarnings),
		);
		await expect
			.element(screen.getByText("Some previews need attention"))
			.toBeVisible();

		service.emit({
			kind: "warningCleared",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: null,
			code: "screenPreviewCacheUnavailable",
		});
		await expect
			.poll(() => screen.getByRole("status").element().textContent)
			.toBe("Photos ready · preparing larger previews · 0 of 6");
	});

	it("distinguishes a preview warning from an unavailable asset", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const unavailable = asset("offline", "Offline", 7, {
			availability: "missing",
			shapeState: "fallback",
		});
		service.releaseQuery(
			0,
			pageOf([...realFixtureAssets, unavailable], "settled"),
		);
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		service.emit({
			kind: "warning",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "coast",
			warning: { code: "derivativeUnavailable", retryable: true },
		});
		const wall = screen.getByRole("region", { name: "Photos" });
		const warnedTile = () =>
			wall.element().querySelector<HTMLElement>('[data-asset-id="coast"]');
		await expect
			.poll(() =>
				warnedTile()?.querySelector('[aria-label="Photo preview warning"]'),
			)
			.not.toBeNull();
		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Some previews need attention");
		expect(
			warnedTile()?.querySelector('[data-testid="photo-fallback"]'),
		).toBeNull();
		expect(warnedTile()?.textContent).not.toContain("File unavailable");
		expect(warnedTile()?.textContent).not.toContain("/");
		await expect
			.element(screen.getByTestId("photo-fallback").last())
			.toBeVisible();
		service.emit({
			kind: "warningCleared",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "coast",
			code: "derivativeUnavailable",
		});
		await expect
			.poll(() =>
				warnedTile()?.querySelector('[aria-label="Photo preview warning"]'),
			)
			.toBeNull();
		expect(
			warnedTile()?.querySelector('[data-testid="photo-fallback"]'),
		).toBeNull();
		await expect
			.element(screen.getByTestId("photo-fallback").last())
			.toBeVisible();
	});

	it("keeps a later unrelated asset warning after a stale derivative page", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		service.emit({
			kind: "warning",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "coast",
			warning: { code: "derivativeUnavailable", retryable: true },
		});
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.emit({
			kind: "warningCleared",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "coast",
			code: "derivativeUnavailable",
		});
		service.emit({
			kind: "warning",
			selectionId: "source-a",
			sourceId: "source-a",
			assetId: "coast",
			warning: { code: "assetWarning", retryable: true },
		});
		const staleItems = settledFixtures.map((item) =>
			item.id === "coast"
				? {
						...item,
						warning: { code: "derivativeUnavailable", retryable: true },
					}
				: item,
		);
		service.releaseQuery(1, pageOf(staleItems, "settled"));
		const coastTile = () =>
			screen
				.getByRole("region", { name: "Photos" })
				.element()
				.querySelector<HTMLElement>('[data-asset-id="coast"]');
		await expect
			.poll(() =>
				coastTile()?.querySelector('[aria-label="Photo preview warning"]'),
			)
			.not.toBeNull();
	});

	it("keeps an available asset geometry when its preview image fails", async () => {
		const service = new ControlledWallService();
		const broken = asset("broken", "Broken", 1);
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf([broken], "settled"));
		await expect
			.poll(() => screen.getByTestId("photo-row-0").query())
			.not.toBeNull();
		service.releaseThumbnail(
			"broken",
			"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
		);
		const tile = screen
			.getByRole("region", { name: "Photos" })
			.element()
			.querySelector<HTMLElement>('[data-asset-id="broken"]');
		await expect
			.poll(() => tile?.querySelector<HTMLImageElement>('img[alt="Broken"]'))
			.not.toBeNull();
		const image = tile?.querySelector<HTMLImageElement>('img[alt="Broken"]');
		expect(image).not.toBeNull();
		image?.dispatchEvent(new Event("error"));
		await expect
			.poll(() =>
				tile?.querySelector('[aria-label="Photo preview unavailable"]'),
			)
			.not.toBeNull();
		expect(tile?.querySelector('[data-testid="photo-fallback"]')).toBeNull();
		expect(tile?.textContent).not.toContain("File unavailable");
		expect(tile?.textContent).not.toContain("/");
	});
});
