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
	GalleryScope,
	PhotoService,
	SortDirection,
	SourceAvailability,
	WallAsset,
	WallPage,
	WallQueryRequest,
	WallUpdate,
	WallWarningState,
} from "../services/photoService";
import { AppShell } from "./AppShell";
import { PhotoTile } from "./PhotoTile";
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
	readonly capabilities = {
		chooseFolder: true,
		folderSelection: "native" as const,
		locateFolder: false,
	};
	readonly queryRequests: WallQueryRequest[] = [];
	readonly queryScopes: GalleryScope[] = [];
	readonly watchedScopes: GalleryScope[] = [];
	readonly derivativeRequests: DerivativeRequest[] = [];
	derivativeRejectsRemaining = 0;
	derivativeHoldPriority: DerivativeRequest["priority"] | null = null;
	readonly heldDerivativeRequests: Array<Gate<void>> = [];
	readonly interactionCalls: boolean[] = [];
	readonly rememberedDirections: SortDirection[] = [];
	readonly gates: Gate<WallPage>[] = [];
	scopeUpdateReplay: (() => void) | null = null;
	private readonly listeners = new Set<(update: WallUpdate) => void>();
	private readonly urls = new Map<string, string>();
	private readonly sourceState: BootstrapState;

	constructor(
		sourceId = "source-a",
		private readonly restoredDirection: SortDirection = "oldestFirst",
		sourceAvailability: SourceAvailability = "available",
		private currentGalleryScope: GalleryScope = "includeSubfolders",
	) {
		this.sourceState = {
			settings: { appearance: "system", galleryScope: currentGalleryScope },
			activeSource: {
				id: sourceId,
				selectionId: sourceId,
				displayName: sourceId,
				availability: sourceAvailability,
			},
		};
	}
	getBootstrapState = async () => structuredClone(this.sourceState);
	chooseFolder = async (): Promise<ChooseFolderResult> => ({
		kind: "selected",
		state: structuredClone(this.sourceState),
	});
	listFolders = async () => {
		throw new Error("native picker fixture");
	};
	selectFolder = async () => {
		throw new Error("native picker fixture");
	};
	folderBrowserState = () => ({ breadcrumbs: [], initialPath: "" });
	initialSortDirection = () => this.restoredDirection;
	rememberSortDirection = (direction: SortDirection) => {
		this.rememberedDirections.push(direction);
	};
	updateAppearance = async (appearance: Appearance) => ({
		...this.sourceState,
		settings: { ...this.sourceState.settings, appearance },
	});
	updateGalleryScope = async (galleryScope: GalleryScope) => {
		this.currentGalleryScope = galleryScope;
		this.sourceState.settings.galleryScope = galleryScope;
		this.scopeUpdateReplay?.();
		return structuredClone(this.sourceState);
	};
	queryWall = (request: WallQueryRequest) => {
		this.queryRequests.push(request);
		this.queryScopes.push(this.currentGalleryScope);
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
		this.watchedScopes.push(this.currentGalleryScope);
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
	setDerivativeUrl = (key: string, url: string) => {
		this.urls.set(key, url);
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
		rating: null,
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

interface ImageRuntimeOverrides {
	complete?: (image: HTMLImageElement) => boolean;
	naturalWidth?: (image: HTMLImageElement) => number;
	decode?: (image: HTMLImageElement) => Promise<void>;
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
	if (overrides.complete)
		Object.defineProperty(HTMLImageElement.prototype, "complete", {
			configurable: true,
			get() {
				return overrides.complete?.(this as HTMLImageElement);
			},
		});
	if (overrides.naturalWidth)
		Object.defineProperty(HTMLImageElement.prototype, "naturalWidth", {
			configurable: true,
			get() {
				return overrides.naturalWidth?.(this as HTMLImageElement);
			},
		});
	if (overrides.decode)
		Object.defineProperty(HTMLImageElement.prototype, "decode", {
			configurable: true,
			value: function (this: HTMLImageElement) {
				return overrides.decode?.(this);
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

function suppressCapture(type: "error" | "load") {
	const handler = (event: Event) => event.stopImmediatePropagation();
	window.addEventListener(type, handler, true);
	document.addEventListener(type, handler, true);
	return () => {
		window.removeEventListener(type, handler, true);
		document.removeEventListener(type, handler, true);
	};
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
	it("uses the restored direction for the first wall request and remembers changes", async () => {
		const service = new ControlledWallService("source-a", "newestFirst");
		const screen = await renderWall(service);

		await expect.poll(() => service.queryRequests.length).toBe(1);
		expect(service.queryRequests[0]?.direction).toBe("newestFirst");
		service.releaseQuery(0, pageOf(settledFixtures, "settled"));
		await screen.getByRole("button", { name: "Oldest first" }).click();

		await expect.poll(() => service.queryRequests.length).toBe(2);
		expect(service.rememberedDirections).toEqual(["oldestFirst"]);
	});

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
		const refinedTile = screen
			.getByTestId("photo-row-0")
			.element()
			.querySelector<HTMLElement>("[data-asset-id='coast']");
		expect(refinedTile?.getBoundingClientRect().toJSON()).toEqual(before);
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
			kind: "wallThumbnail",
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
		const wall = screen.getByRole("region", { name: "Photos" });
		TestIntersectionObserver.trigger("near", wall.element(), [
			"provisional-40",
		]);
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(
			service.derivativeRequests.flatMap((request) => request.assetIds),
		).not.toContain("provisional-40");

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
		const settledRows = [
			...wall
				.element()
				.querySelectorAll<HTMLElement>("[data-testid^='photo-row-']"),
		];
		const wallRect = wall.element().getBoundingClientRect();
		const visibleRows = settledRows
			.map((row, index) => {
				const top =
					row.getBoundingClientRect().top -
					wallRect.top +
					wall.element().scrollTop;
				const bottom = top + row.getBoundingClientRect().height;
				return top < wall.element().scrollTop + wall.element().clientHeight &&
					bottom > wall.element().scrollTop
					? index
					: -1;
			})
			.filter((index) => index >= 0);
		const lastVisible = visibleRows.at(-1) ?? 0;
		const nearRows = [lastVisible + 1, lastVisible + 2].filter(
			(index) => index < settledRows.length,
		);
		const idsInRows = (indices: readonly number[]) =>
			indices.flatMap((index) =>
				[
					...(settledRows[index]?.querySelectorAll<HTMLElement>(
						"[data-asset-id]",
					) ?? []),
				].map((tile) => tile.dataset.assetId ?? ""),
			);
		const settledVisibleIds = idsInRows(visibleRows);
		const settledNearIds = idsInRows(nearRows);
		const settledRemainingIds = idsInRows(
			settledRows
				.map((_row, index) => index)
				.filter(
					(index) => !visibleRows.includes(index) && !nearRows.includes(index),
				),
		);
		const previouslyRequested = new Set(
			service.derivativeRequests
				.slice(0, settledStart)
				.flatMap((request) => request.assetIds),
		);
		const expectedIdleIds = settledRemainingIds.filter(
			(id) => !previouslyRequested.has(id),
		);
		expect(service.derivativeRequests[settledStart]).toEqual({
			assetIds: settledVisibleIds,
			priority: "visible",
			kind: "wallThumbnail",
		});
		expect(service.derivativeRequests[settledStart + 1]).toEqual({
			assetIds: settledNearIds,
			priority: "nearViewport",
			kind: "wallThumbnail",
		});
		const settledIdle = idleCallbacks.splice(0);
		expect(settledIdle.length).toBeGreaterThan(0);
		for (const callback of settledIdle) callback();
		while (idleCallbacks.length > 0)
			for (const callback of idleCallbacks.splice(0)) callback();
		await expect
			.poll(() => service.derivativeRequests.length)
			.toBeGreaterThan(settledStart + 1);
		expect(
			service.derivativeRequests
				.slice(settledStart + 2)
				.flatMap((request) => request.assetIds),
		).toEqual(expectedIdleIds);
		screen.unmount();
	});

	it("tracks the current provisional viewport for observer promotions", async () => {
		await page.viewport(1440, 520);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		const provisional = Array.from({ length: 60 }, (_, index) =>
			asset(`scroll-provisional-${index}`, `Scroll ${index}`, index + 1),
		);
		service.releaseQuery(0, pageOf(provisional, "provisional"));
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.poll(() => wall.element().scrollHeight)
			.toBeGreaterThan(wall.element().clientHeight);
		const candidates = provisional.slice(30).map((item) => item.id);
		TestIntersectionObserver.trigger("near", wall.element(), candidates);
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		for (const id of candidates)
			expect(
				service.derivativeRequests
					.filter((request) => request.priority === "nearViewport")
					.flatMap((request) => request.assetIds),
			).not.toContain(id);
		wall.element().scrollTop = wall.element().scrollHeight / 2;
		wall.element().dispatchEvent(new Event("scroll"));
		await new Promise((resolve) =>
			window.requestAnimationFrame(() => resolve(undefined)),
		);
		const rows = [
			...wall
				.element()
				.querySelectorAll<HTMLElement>("[data-testid^='photo-row-']"),
		];
		const rootRect = wall.element().getBoundingClientRect();
		const visibleRows = rows
			.map((row, index) => {
				const top =
					row.getBoundingClientRect().top -
					rootRect.top +
					wall.element().scrollTop;
				const bottom = top + row.getBoundingClientRect().height;
				return top < wall.element().scrollTop + wall.element().clientHeight &&
					bottom > wall.element().scrollTop
					? index
					: -1;
			})
			.filter((index) => index >= 0);
		const lastVisible = visibleRows.at(-1) ?? 0;
		const nearRows = [lastVisible + 1, lastVisible + 2].filter(
			(index) => index < rows.length,
		);
		const beyondRow = lastVisible + 3;
		const idsInRows = (indices: readonly number[]) =>
			indices.flatMap((index) =>
				[
					...(rows[index]?.querySelectorAll<HTMLElement>("[data-asset-id]") ??
						[]),
				].map((tile) => tile.dataset.assetId ?? ""),
			);
		const visibleIds = idsInRows(visibleRows);
		const nearIds = idsInRows(nearRows);
		const beyondIds = idsInRows([beyondRow]);
		expect(visibleIds.length).toBeGreaterThan(0);
		expect(nearIds.length).toBeGreaterThan(0);
		expect(beyondIds.length).toBeGreaterThan(0);

		expect(
			visibleIds.every((id) =>
				service.derivativeRequests.some(
					(request) =>
						request.priority === "visible" && request.assetIds.includes(id),
				),
			),
		).toBe(true);
		expect(
			nearIds.every((id) =>
				service.derivativeRequests.some(
					(request) =>
						request.priority === "nearViewport" &&
						request.assetIds.includes(id),
				),
			),
		).toBe(true);
		for (const id of beyondIds)
			expect(
				service.derivativeRequests
					.filter((request) => request.priority === "nearViewport")
					.flatMap((request) => request.assetIds),
			).not.toContain(id);
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
			kind: "wallThumbnail",
		});
	});

	it("persists the subfolder toggle and reloads the wall from the new scope", async () => {
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf([asset("recursive", "Recursive", 1)], "settled"),
		);
		const toggle = screen.getByRole("button", { name: "Include subfolders" });
		await expect.element(toggle).toHaveAttribute("aria-pressed", "true");

		await toggle.click();

		await expect.element(toggle).toHaveAttribute("aria-pressed", "false");
		await expect.poll(() => service.queryRequests.length).toBe(2);
		service.releaseQuery(1, pageOf([asset("direct", "Direct", 1)], "settled"));
		const wall = screen.getByRole("region", { name: "Photos" }).element();
		await expect
			.poll(() => wall.querySelector("[data-asset-id='direct']"))
			.not.toBeNull();
		expect(wall.querySelector("[data-asset-id='recursive']")).toBeNull();
	});

	it("keeps gallery controls inside a phone-width header", async () => {
		await page.viewport(390, 844);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf([]));

		for (const name of [
			"Include subfolders",
			"Oldest first",
			"Newest first",
			"Appearance",
		]) {
			const bounds = screen
				.getByRole("button", { name })
				.element()
				.getBoundingClientRect();
			expect(
				bounds.left,
				`${name} starts outside the viewport`,
			).toBeGreaterThanOrEqual(0);
			expect(
				bounds.right,
				`${name} ends outside the viewport`,
			).toBeLessThanOrEqual(390);
		}
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
			kind: "wallThumbnail",
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
		expect(
			wall.element().querySelectorAll<HTMLImageElement>("img").length,
		).toBe(realFixtureAssets.length);
		screen.unmount();
	});

	it("reveals a cached image when its load event does not reach React", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl(
			"cached-completion",
			"/demo-photos/coast.jpg?cached-completion=1",
		);
		const positioned = {
			asset: asset("coast", "Coast", 1, {
				wallThumbnail: {
					assetId: "coast",
					kind: "wallThumbnail" as const,
					key: "cached-completion",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		const warmImage = new Image();
		warmImage.src = "/demo-photos/coast.jpg?cached-completion=1";
		await expect
			.poll(() => warmImage.complete && warmImage.naturalWidth > 0)
			.toBe(true);
		const suppressImageLoad = (event: Event) => {
			event.stopImmediatePropagation();
		};
		const restoreReducedMotion = overrideReducedMotion(false);
		window.addEventListener("load", suppressImageLoad, true);
		document.addEventListener("load", suppressImageLoad, true);
		try {
			const screen = await render(
				<PhotoTile onOpen={onOpen} positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Coast" });
			await expect
				.poll(() => {
					const element = image.element() as HTMLImageElement;
					return element.complete && element.naturalWidth > 0;
				})
				.toBe(true);
			expect(image.element().getAttribute("src")).toBe(
				"/demo-photos/coast.jpg?cached-completion=1",
			);
			await expect
				.poll(() => getComputedStyle(image.element()).opacity)
				.toBe("1");
			expect(
				screen.getByRole("button", { name: "Open Coast", exact: true }).query(),
			).toBeNull();
			image.element().dispatchEvent(
				new TransitionEvent("transitionend", {
					bubbles: true,
					propertyName: "opacity",
				}),
			);
			await expect
				.element(
					screen.getByRole("button", { name: "Open Coast", exact: true }),
				)
				.toBeVisible();
			await screen
				.getByRole("button", { name: "Open Coast", exact: true })
				.click();
			expect(onOpen).toHaveBeenCalledTimes(1);
			screen.unmount();
		} finally {
			window.removeEventListener("load", suppressImageLoad, true);
			document.removeEventListener("load", suppressImageLoad, true);
			restoreReducedMotion();
		}
	});

	it("keeps a screen-preview-only tile inert until its wall thumbnail is ready", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		const positioned = {
			asset: asset("screen-only", "Screen only", 1, {
				wallThumbnail: null,
				screenPreview: {
					assetId: "screen-only",
					kind: "screenPreview" as const,
					key: "screen-only-preview",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};

		const screen = await render(
			<PhotoTile onOpen={onOpen} positioned={positioned} service={service} />,
		);

		expect(
			service.derivativeRequests.filter(
				(request) => request.kind === "screenPreview",
			),
		).toHaveLength(0);
		expect(
			screen
				.getByRole("button", { name: "Open Screen only", exact: true })
				.query(),
		).toBeNull();
		expect(
			document.querySelector("[data-asset-id='screen-only']")?.tagName,
		).toBe("FIGURE");
		expect(onOpen).not.toHaveBeenCalled();
		await screen.unmount();
	});

	it("keeps an integrated screen-only tile inert on pointer activation", async () => {
		await page.viewport(1440, 1024);
		const service = new ControlledWallService();
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(
			0,
			pageOf(
				[
					asset("screen-only-integrated", "Screen only", 1, {
						wallThumbnail: null,
						screenPreview: {
							assetId: "screen-only-integrated",
							kind: "screenPreview" as const,
							key: "screen-only-integrated-preview",
						},
					}),
				],
				"settled",
			),
		);
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.poll(() =>
				wall
					.element()
					.querySelector("[data-asset-id='screen-only-integrated']"),
			)
			.not.toBeNull();
		const tile = wall
			.element()
			.querySelector<HTMLElement>("[data-asset-id='screen-only-integrated']");
		if (!tile) throw new Error("integrated screen-only tile did not render");
		const screenRequests = () =>
			service.derivativeRequests.filter(
				(request) => request.kind === "screenPreview",
			).length;
		const derivativeRequestsBeforeActivation =
			service.derivativeRequests.length;
		expect(screenRequests()).toBe(0);
		tile.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				cancelable: true,
				isPrimary: true,
				pointerId: 42,
				pointerType: "mouse",
			}),
		);
		tile.dispatchEvent(
			new MouseEvent("click", { bubbles: true, cancelable: true }),
		);
		expect(screenRequests()).toBe(0);
		expect(service.derivativeRequests.length).toBe(
			derivativeRequestsBeforeActivation,
		);
		expect(
			screen.getByRole("dialog", { name: "Photo viewer" }).query(),
		).toBeNull();
		expect(tile.querySelector("button")).toBeNull();
		screen.unmount();
	});

	it("opens a root-offline photo from cached wall and screen derivatives", async () => {
		await page.viewport(1440, 1024);
		const restoreReducedMotion = overrideReducedMotion(true);
		const service = new ControlledWallService(
			"source-a",
			"newestFirst",
			"rootOffline",
		);
		service.setDerivativeUrl(
			"root-offline-wall",
			"/demo-photos/coast.jpg?root-offline-wall=1",
		);
		service.setDerivativeUrl(
			"root-offline-screen",
			"/demo-photos/mountain.jpg?root-offline-screen=1",
		);
		try {
			const screen = await renderWall(service);
			await expect.poll(() => service.queryRequests.length).toBe(1);
			expect(service.queryRequests[0]?.direction).toBe("newestFirst");
			service.emit({
				kind: "sourceUnavailable",
				selectionId: "source-a",
				sourceId: "source-a",
			});
			service.releaseQuery(
				0,
				pageOf(
					[
						asset("root-offline", "Root offline", 1, {
							availability: "rootOffline",
							warning: { code: "sourceUnavailable", retryable: true },
							wallThumbnail: {
								assetId: "root-offline",
								kind: "wallThumbnail",
								key: "root-offline-wall",
							},
							screenPreview: {
								assetId: "root-offline",
								kind: "screenPreview",
								key: "root-offline-screen",
							},
						}),
					],
					"settled",
				),
			);

			await expect
				.element(screen.getByRole("img", { name: "Root offline" }))
				.toBeVisible();
			const open = screen.getByRole("button", {
				name: "Open Root offline",
				exact: true,
			});
			await expect.element(open).toBeVisible();
			await open.click();
			await expect
				.element(screen.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			await expect
				.element(screen.getByTestId("viewer-status"))
				.toHaveTextContent("Root offline, photo 1 of 1");
			screen.unmount();
		} finally {
			restoreReducedMotion();
		}
	});

	it("shows an uncached offline child after broadening the restored scope", async () => {
		await page.viewport(1280, 800);
		const service = new ControlledWallService(
			"source-a",
			"oldestFirst",
			"rootOffline",
			"currentFolder",
		);
		service.setDerivativeUrl(
			"offline-parent-wall",
			"/demo-photos/coast.jpg?offline-parent-wall=1",
		);
		const parent = asset("offline-parent", "Parent", 1, {
			availability: "rootOffline",
			wallThumbnail: {
				assetId: "offline-parent",
				kind: "wallThumbnail",
				key: "offline-parent-wall",
			},
		});
		const child = asset("offline-child", "Child", 2, {
			availability: "rootOffline",
		});
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		expect(service.queryScopes).toEqual(["currentFolder"]);
		service.releaseQuery(0, pageOf([parent], "settled"));
		await expect
			.element(screen.getByRole("img", { name: "Parent" }))
			.toBeVisible();

		await screen
			.getByRole("button", { name: "Include subfolders", exact: true })
			.click();
		await expect.poll(() => service.queryRequests.length).toBe(2);
		expect(service.queryScopes).toEqual(["currentFolder", "includeSubfolders"]);
		expect(service.watchedScopes).toEqual([
			"currentFolder",
			"includeSubfolders",
		]);
		service.emit({
			kind: "sourceUnavailable",
			selectionId: "source-a",
			sourceId: "source-a",
		});
		service.releaseQuery(1, pageOf([parent, child], "settled"));

		await expect
			.poll(
				() =>
					document.querySelector('figure[data-asset-id="offline-child"]')
						?.textContent,
			)
			.toContain("File unavailable");
		service.emit({
			kind: "catalogBatch",
			selectionId: "source-a",
			generation: 1,
			orderState: "provisional",
			progress: { discovered: 2, shaped: 2, enriched: 0, total: 2 },
			assets: [
				{
					...child,
					availability: "available",
					representativeRgb: 0x123456,
				},
			],
		});
		await expect
			.poll(() =>
				document
					.querySelector<HTMLElement>('figure[data-asset-id="offline-child"]')
					?.getAttribute("style"),
			)
			.toContain("rgb(18, 52, 86)");
		await expect
			.poll(
				() =>
					document.querySelector('figure[data-asset-id="offline-child"]')
						?.textContent,
			)
			.toContain("File unavailable");
		expect(
			screen.getByRole("button", { name: "Open Child", exact: true }).query(),
		).toBeNull();

		await screen
			.getByRole("button", { name: "Newest first", exact: true })
			.click();
		await expect.poll(() => service.queryRequests.length).toBe(3);
		service.releaseQuery(
			2,
			pageOf([parent, { ...child, availability: "available" }], "settled"),
		);
		await expect
			.poll(
				() =>
					document.querySelector('figure[data-asset-id="offline-child"]')
						?.textContent,
			)
			.not.toContain("File unavailable");
		screen.unmount();
	});

	it("keeps an uncached offline child visible when scope replay ends during a provisional page", async () => {
		await page.viewport(1280, 800);
		const service = new ControlledWallService(
			"source-a",
			"oldestFirst",
			"rootOffline",
			"currentFolder",
		);
		service.setDerivativeUrl(
			"offline-parent-wall",
			"/demo-photos/coast.jpg?offline-parent-wall=1",
		);
		const parent = asset("offline-parent", "Parent", 1, {
			availability: "rootOffline",
			wallThumbnail: {
				assetId: "offline-parent",
				kind: "wallThumbnail",
				key: "offline-parent-wall",
			},
		});
		const child = asset("offline-child", "Child", 2, {
			availability: "rootOffline",
		});
		const screen = await renderWall(service);
		await expect.poll(() => service.queryRequests.length).toBe(1);
		service.releaseQuery(0, pageOf([parent], "settled"));
		await expect
			.element(screen.getByRole("img", { name: "Parent" }))
			.toBeVisible();

		service.scopeUpdateReplay = () => {
			service.emit({
				kind: "catalogBatch",
				selectionId: "source-a",
				generation: 1,
				orderState: "provisional",
				progress: { discovered: 2, shaped: 2, enriched: 0, total: 2 },
				assets: [{ ...child, availability: "available" }],
			});
			service.emit({
				kind: "sourceUnavailable",
				selectionId: "source-a",
				sourceId: "source-a",
			});
			service.emit({ kind: "resyncRequired", selectionId: "source-a" });
		};
		await screen
			.getByRole("button", { name: "Include subfolders", exact: true })
			.click();
		await expect.poll(() => service.queryRequests.length).toBe(3);
		expect(service.queryScopes).toEqual([
			"currentFolder",
			"includeSubfolders",
			"includeSubfolders",
		]);
		service.releaseQuery(2, pageOf([parent, child], "provisional"));
		service.emit({
			kind: "catalogBatch",
			selectionId: "source-a",
			generation: 1,
			orderState: "provisional",
			progress: { discovered: 2, shaped: 2, enriched: 0, total: 2 },
			assets: [{ ...child, availability: "available" }],
		});

		await expect
			.element(screen.getByRole("status"))
			.toHaveTextContent("Preparing previews · 1 of 2");
		const figure = () =>
			document.querySelector<HTMLElement>(
				'figure[data-asset-id="offline-child"]',
			);
		await expect.poll(() => figure()).not.toBeNull();
		await expect
			.poll(() => figure()?.textContent)
			.toContain("File unavailable");
		expect(
			screen.getByRole("button", { name: "Open Child", exact: true }).query(),
		).toBeNull();
		screen.unmount();
	});

	it("keeps the current tile inert through decode and fade, then enables it", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl(
			"paint-before-open",
			"/demo-photos/coast.jpg?paint-before-open=1",
		);
		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			naturalWidth: () => naturalWidth,
			decode: () => decodeGate.promise,
		});
		const restoreReducedMotion = overrideReducedMotion(false);
		const restoreLoadCapture = suppressCapture("load");
		const positioned = {
			asset: asset("paint-before-open", "Photo A", 1, {
				wallThumbnail: {
					assetId: "paint-before-open",
					kind: "wallThumbnail" as const,
					key: "paint-before-open",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		try {
			const screen = await render(
				<PhotoTile onOpen={onOpen} positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Photo A" });
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();

			complete = true;
			naturalWidth = 320;
			image.element().dispatchEvent(new Event("load"));
			decodeGate.resolve();
			await expect
				.poll(() => getComputedStyle(image.element()).opacity)
				.toBe("1");
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();

			image.element().dispatchEvent(
				new TransitionEvent("transitionend", {
					bubbles: true,
					propertyName: "opacity",
				}),
			);
			await expect
				.element(
					screen.getByRole("button", { name: "Open Photo A", exact: true }),
				)
				.toBeVisible();
			expect(getComputedStyle(image.element()).opacity).toBe("1");
			expect(onOpen).not.toHaveBeenCalled();
			screen.unmount();
		} finally {
			restoreLoadCapture();
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("ignores a stale opacity transition after a thumbnail URL replacement", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl("paint-old", "/demo-photos/coast.jpg?paint-old=1");
		service.setDerivativeUrl(
			"paint-new",
			"/demo-photos/forest.jpg?paint-new=1",
		);
		const oldDecode = gate<void>();
		const newDecode = gate<void>();
		let oldComplete = false;
		let newComplete = false;
		const restoreImageRuntime = overrideImageRuntime({
			complete: (image) =>
				image.src.includes("paint-new") ? newComplete : oldComplete,
			naturalWidth: (image) =>
				image.src.includes("paint-new")
					? newComplete
						? 320
						: 0
					: oldComplete
						? 320
						: 0,
			decode: (image) =>
				image.src.includes("paint-new") ? newDecode.promise : oldDecode.promise,
		});
		const restoreLoadCapture = suppressCapture("load");
		const restoreReducedMotion = overrideReducedMotion(false);
		function SwapHarness() {
			const [key, setKey] = useState("paint-old");
			const positioned = {
				asset: asset("paint-replacement", "Photo A", 1, {
					wallThumbnail: {
						assetId: "paint-replacement",
						kind: "wallThumbnail" as const,
						key,
					},
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<>
					<button onClick={() => setKey("paint-new")} type="button">
						Replace thumbnail
					</button>
					<PhotoTile
						onOpen={onOpen}
						positioned={positioned}
						service={service}
					/>
				</>
			);
		}
		try {
			const screen = await render(<SwapHarness />);
			const image = screen.getByRole("img", { name: "Photo A" });
			const imageElement = image.element();
			oldComplete = true;
			oldDecode.resolve();
			await expect.poll(() => getComputedStyle(imageElement).opacity).toBe("1");
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();
			expect(onOpen).not.toHaveBeenCalled();

			await screen.getByRole("button", { name: "Replace thumbnail" }).click();
			await expect
				.poll(() => imageElement.getAttribute("src"))
				.toContain("paint-new=1");
			expect(imageElement).toBe(image.element());
			expect(getComputedStyle(imageElement).opacity).toBe("0");
			imageElement.dispatchEvent(
				new TransitionEvent("transitionend", {
					bubbles: true,
					propertyName: "opacity",
				}),
			);
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();

			newComplete = true;
			newDecode.resolve();
			await expect.poll(() => getComputedStyle(imageElement).opacity).toBe("1");
			imageElement.dispatchEvent(
				new TransitionEvent("transitionend", {
					bubbles: true,
					propertyName: "opacity",
				}),
			);
			await expect
				.element(
					screen.getByRole("button", { name: "Open Photo A", exact: true }),
				)
				.toBeVisible();
			screen.unmount();
		} finally {
			restoreLoadCapture();
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("uses one reduced-motion frame after decode before enabling the overlay", async () => {
		const service = new ControlledWallService();
		service.setDerivativeUrl(
			"reduced-paint",
			"/demo-photos/coast.jpg?reduced-paint=1",
		);
		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			naturalWidth: () => naturalWidth,
			decode: () => decodeGate.promise,
		});
		const restoreReducedMotion = overrideReducedMotion(true);
		const originalRequestAnimationFrame = window.requestAnimationFrame;
		const originalCancelAnimationFrame = window.cancelAnimationFrame;
		const frames = new Map<number, FrameRequestCallback>();
		let nextFrame = 0;
		let frameRequests = 0;
		window.requestAnimationFrame = (callback) => {
			const id = ++nextFrame;
			frameRequests += 1;
			frames.set(id, callback);
			return id;
		};
		window.cancelAnimationFrame = (id) => {
			frames.delete(id);
		};
		const restoreLoadCapture = suppressCapture("load");
		const positioned = {
			asset: asset("reduced-paint", "Photo A", 1, {
				wallThumbnail: {
					assetId: "reduced-paint",
					kind: "wallThumbnail" as const,
					key: "reduced-paint",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		try {
			const screen = await render(
				<PhotoTile positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Photo A" });
			const initialFrameRequests = frameRequests;
			complete = true;
			naturalWidth = 320;
			image.element().dispatchEvent(new Event("load"));
			decodeGate.resolve();
			await expect.poll(() => frameRequests).toBe(initialFrameRequests + 1);
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();
			const frameId = [...frames.keys()].at(-1);
			if (frameId === undefined) throw new Error("missing paint frame");
			const frame = frames.get(frameId);
			if (!frame) throw new Error("missing paint frame callback");
			frames.delete(frameId);
			frame(performance.now());
			await expect
				.element(
					screen.getByRole("button", { name: "Open Photo A", exact: true }),
				)
				.toBeVisible();
			screen.unmount();
		} finally {
			frames.clear();
			window.requestAnimationFrame = originalRequestAnimationFrame;
			window.cancelAnimationFrame = originalCancelAnimationFrame;
			restoreLoadCapture();
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("cancels a stale reduced-motion frame when the thumbnail URL changes", async () => {
		const service = new ControlledWallService();
		service.setDerivativeUrl(
			"reduced-old",
			"/demo-photos/coast.jpg?reduced-old=1",
		);
		service.setDerivativeUrl(
			"reduced-new",
			"/demo-photos/forest.jpg?reduced-new=1",
		);
		const oldDecode = gate<void>();
		const newDecode = gate<void>();
		let oldComplete = false;
		let newComplete = false;
		const onOpen = vi.fn();
		const restoreImageRuntime = overrideImageRuntime({
			complete: (image) =>
				image.src.includes("reduced-new") ? newComplete : oldComplete,
			naturalWidth: (image) =>
				image.src.includes("reduced-new")
					? newComplete
						? 320
						: 0
					: oldComplete
						? 320
						: 0,
			decode: (image) =>
				image.src.includes("reduced-new")
					? newDecode.promise
					: oldDecode.promise,
		});
		const restoreReducedMotion = overrideReducedMotion(true);
		const originalRequestAnimationFrame = window.requestAnimationFrame;
		const originalCancelAnimationFrame = window.cancelAnimationFrame;
		const frames = new Map<number, FrameRequestCallback>();
		let nextFrame = 0;
		let frameRequests = 0;
		window.requestAnimationFrame = (callback) => {
			const id = ++nextFrame;
			frameRequests += 1;
			frames.set(id, callback);
			return id;
		};
		window.cancelAnimationFrame = (id) => {
			frames.delete(id);
		};
		const restoreLoadCapture = suppressCapture("load");
		function SwapHarness() {
			const [key, setKey] = useState("reduced-old");
			const positioned = {
				asset: asset("reduced-replacement", "Photo A", 1, {
					wallThumbnail: {
						assetId: "reduced-replacement",
						kind: "wallThumbnail" as const,
						key,
					},
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<>
					<button onClick={() => setKey("reduced-new")} type="button">
						Replace thumbnail
					</button>
					<PhotoTile
						onOpen={onOpen}
						positioned={positioned}
						service={service}
					/>
				</>
			);
		}
		try {
			const screen = await render(<SwapHarness />);
			const image = screen.getByRole("img", { name: "Photo A" });
			const initialFrameRequests = frameRequests;
			oldComplete = true;
			image.element().dispatchEvent(new Event("load"));
			oldDecode.resolve();
			await expect.poll(() => frameRequests).toBe(initialFrameRequests + 1);
			const oldFrameId = [...frames.keys()].at(-1);
			if (oldFrameId === undefined)
				throw new Error("missing stale paint frame");
			const oldFrame = frames.get(oldFrameId);
			if (!oldFrame) throw new Error("missing stale paint callback");

			await screen.getByRole("button", { name: "Replace thumbnail" }).click();
			await expect
				.poll(() => image.element().getAttribute("src"))
				.toContain("reduced-new=1");
			expect(frames.has(oldFrameId)).toBe(false);
			oldFrame(performance.now());
			expect(
				screen
					.getByRole("button", { name: "Open Photo A", exact: true })
					.query(),
			).toBeNull();

			const beforeNewDecode = frameRequests;
			newComplete = true;
			newDecode.resolve();
			await expect.poll(() => frameRequests).toBe(beforeNewDecode + 1);
			const newFrameId = [...frames.keys()].at(-1);
			if (newFrameId === undefined)
				throw new Error("missing current paint frame");
			const newFrame = frames.get(newFrameId);
			if (!newFrame) throw new Error("missing current paint callback");
			frames.delete(newFrameId);
			newFrame(performance.now());
			await expect
				.element(
					screen.getByRole("button", { name: "Open Photo A", exact: true }),
				)
				.toBeVisible();
			expect(onOpen).not.toHaveBeenCalled();
			screen.unmount();
		} finally {
			frames.clear();
			window.requestAnimationFrame = originalRequestAnimationFrame;
			window.cancelAnimationFrame = originalCancelAnimationFrame;
			restoreLoadCapture();
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("cancels a pending reduced-motion frame when the tile unmounts", async () => {
		const service = new ControlledWallService();
		service.setDerivativeUrl(
			"reduced-unmount",
			"/demo-photos/coast.jpg?reduced-unmount=1",
		);
		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const onOpen = vi.fn();
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			naturalWidth: () => naturalWidth,
			decode: () => decodeGate.promise,
		});
		const restoreReducedMotion = overrideReducedMotion(true);
		const originalRequestAnimationFrame = window.requestAnimationFrame;
		const originalCancelAnimationFrame = window.cancelAnimationFrame;
		const frames = new Map<number, FrameRequestCallback>();
		let nextFrame = 0;
		window.requestAnimationFrame = (callback) => {
			const id = ++nextFrame;
			frames.set(id, callback);
			return id;
		};
		window.cancelAnimationFrame = (id) => {
			frames.delete(id);
		};
		const restoreLoadCapture = suppressCapture("load");
		const positioned = {
			asset: asset("reduced-unmount", "Photo A", 1, {
				wallThumbnail: {
					assetId: "reduced-unmount",
					kind: "wallThumbnail" as const,
					key: "reduced-unmount",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		try {
			const screen = await render(
				<PhotoTile onOpen={onOpen} positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Photo A" });
			complete = true;
			naturalWidth = 320;
			image.element().dispatchEvent(new Event("load"));
			decodeGate.resolve();
			await expect.poll(() => frames.size).toBe(2);
			const frameId = [...frames.keys()].at(-1);
			if (frameId === undefined) throw new Error("missing unmount paint frame");
			const frame = frames.get(frameId);
			if (!frame) throw new Error("missing unmount paint callback");
			screen.unmount();
			expect(frames.has(frameId)).toBe(false);
			frame(performance.now());
			expect(onOpen).not.toHaveBeenCalled();
		} finally {
			frames.clear();
			window.requestAnimationFrame = originalRequestAnimationFrame;
			window.cancelAnimationFrame = originalCancelAnimationFrame;
			restoreLoadCapture();
			restoreImageRuntime();
			restoreReducedMotion();
		}
	});

	it("keeps a published wall thumbnail inert until its image is decoded", async () => {
		const service = new ControlledWallService();
		service.setDerivativeUrl(
			"screen-only-wall",
			"/demo-photos/coast.jpg?screen-only-wall=held",
		);
		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			naturalWidth: () => naturalWidth,
			decode: () => decodeGate.promise,
		});
		const restoreLoadCapture = suppressCapture("load");
		function PublicationHarness() {
			const [published, setPublished] = useState(false);
			const positioned = {
				asset: asset("screen-only", "Screen only", 1, {
					wallThumbnail: published
						? {
								assetId: "screen-only",
								kind: "wallThumbnail" as const,
								key: "screen-only-wall",
							}
						: null,
					screenPreview: {
						assetId: "screen-only",
						kind: "screenPreview" as const,
						key: "screen-only-preview",
					},
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<>
					<button onClick={() => setPublished(true)} type="button">
						Publish wall thumbnail
					</button>
					<PhotoTile positioned={positioned} service={service} />
				</>
			);
		}
		try {
			const screen = await render(<PublicationHarness />);
			expect(
				screen
					.getByRole("button", { name: "Open Screen only", exact: true })
					.query(),
			).toBeNull();
			await screen
				.getByRole("button", { name: "Publish wall thumbnail" })
				.click();
			const image = screen.getByRole("img", { name: "Screen only" });
			await expect
				.poll(() => image.element().getAttribute("src"))
				.toContain("screen-only-wall=held");
			expect(
				screen
					.getByRole("button", { name: "Open Screen only", exact: true })
					.query(),
			).toBeNull();
			expect(getComputedStyle(image.element()).opacity).toBe("0");

			complete = true;
			naturalWidth = 320;
			decodeGate.resolve();
			await expect
				.poll(() => getComputedStyle(image.element()).opacity)
				.toBe("1");
			await expect
				.element(
					screen.getByRole("button", { name: "Open Screen only", exact: true }),
				)
				.toBeVisible();
			screen.unmount();
		} finally {
			restoreLoadCapture();
			restoreImageRuntime();
		}
	});

	it("keeps a wall thumbnail tile inert after its image fails to load", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl(
			"screen-only-failure",
			"/demo-photos/missing-screen-only.jpg?screen-only-failure=1",
		);
		function PublicationHarness() {
			const [published, setPublished] = useState(false);
			const positioned = {
				asset: asset("screen-only", "Screen only", 1, {
					wallThumbnail: published
						? {
								assetId: "screen-only",
								kind: "wallThumbnail" as const,
								key: "screen-only-failure",
							}
						: null,
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<>
					<button onClick={() => setPublished(true)} type="button">
						Publish wall thumbnail
					</button>
					<PhotoTile
						onOpen={onOpen}
						positioned={positioned}
						service={service}
					/>
				</>
			);
		}
		const screen = await render(<PublicationHarness />);
		await screen
			.getByRole("button", { name: "Publish wall thumbnail" })
			.click();
		const image = screen.getByRole("img", { name: "Screen only" });
		await expect
			.poll(() => image.element().getAttribute("src"))
			.toContain("screen-only-failure=1");
		image.element().dispatchEvent(new Event("error"));
		await expect
			.poll(() =>
				screen
					.getByRole("button", { name: "Open Screen only", exact: true })
					.query(),
			)
			.toBeNull();
		expect(
			document.querySelector("[data-asset-id='screen-only']")?.tagName,
		).toBe("FIGURE");
		expect(onOpen).not.toHaveBeenCalled();
		screen.unmount();
	});

	it("reveals a delayed decode after the image load event is suppressed", async () => {
		const service = new ControlledWallService();
		service.setDerivativeUrl(
			"delayed-completion",
			"/demo-photos/coast.jpg?delayed-completion=1",
		);
		const positioned = {
			asset: asset("coast", "Coast", 1, {
				wallThumbnail: {
					assetId: "coast",
					kind: "wallThumbnail" as const,
					key: "delayed-completion",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		const decodeGate = gate<void>();
		let complete = false;
		let naturalWidth = 0;
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => complete,
			naturalWidth: () => naturalWidth,
			decode: () => decodeGate.promise,
		});
		const restoreLoadCapture = suppressCapture("load");
		try {
			const screen = await render(
				<PhotoTile positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Coast" });
			await new Promise((resolve) => window.setTimeout(resolve, 75));
			expect(getComputedStyle(image.element()).opacity).toBe("0");
			complete = true;
			naturalWidth = 320;
			decodeGate.resolve();
			await expect
				.poll(() => getComputedStyle(image.element()).opacity)
				.toBe("1");
			screen.unmount();
		} finally {
			restoreLoadCapture();
			restoreImageRuntime();
		}
	});

	it("keeps a zero-width completed image on the preview failure path", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl(
			"zero-width",
			"/demo-photos/missing-zero-width.jpg?zero-width=1",
		);
		const positioned = {
			asset: asset("coast", "Coast", 1, {
				wallThumbnail: {
					assetId: "coast",
					kind: "wallThumbnail" as const,
					key: "zero-width",
				},
			}),
			left: 0,
			width: 320,
			height: 220,
		};
		const restoreImageRuntime = overrideImageRuntime({
			complete: () => true,
			naturalWidth: () => 0,
		});
		const restoreErrorCapture = suppressCapture("error");
		try {
			const screen = await render(
				<PhotoTile onOpen={onOpen} positioned={positioned} service={service} />,
			);
			const image = screen.getByRole("img", { name: "Coast" });
			expect((image.element() as HTMLImageElement).complete).toBe(true);
			expect((image.element() as HTMLImageElement).naturalWidth).toBe(0);
			expect(getComputedStyle(image.element()).opacity).toBe("0");
			const tile = document.querySelector<HTMLElement>(
				"[data-asset-id='coast']",
			);
			tile?.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					isPrimary: true,
					pointerId: 41,
					pointerType: "mouse",
				}),
			);
			tile?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
			expect(
				screen.getByRole("button", { name: "Open Coast", exact: true }).query(),
			).toBeNull();
			expect(onOpen).not.toHaveBeenCalled();
			screen.unmount();
		} finally {
			restoreErrorCapture();
			restoreImageRuntime();
		}
	});

	it("does not let an old decode reveal a replacement thumbnail", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl("fenced-old", "/demo-photos/coast.jpg");
		service.setDerivativeUrl(
			"fenced-new",
			"/demo-photos/forest.jpg?fenced-replacement=1",
		);
		const oldDecode = gate<void>();
		const newDecode = gate<void>();
		let oldDecodeContinuationRan = false;
		const oldDecodeResult = oldDecode.promise.then(() => {
			oldDecodeContinuationRan = true;
		});
		const oldComplete = true;
		let newComplete = false;
		const restoreImageRuntime = overrideImageRuntime({
			complete: (image) =>
				image.src.includes("forest") ? newComplete : oldComplete,
			naturalWidth: (image) =>
				image.src.includes("forest")
					? newComplete
						? 320
						: 0
					: oldComplete
						? 320
						: 0,
			decode: (image) =>
				image.src.includes("forest") ? newDecode.promise : oldDecodeResult,
		});
		const restoreLoadCapture = suppressCapture("load");
		function SwapHarness() {
			const [key, setKey] = useState("fenced-old");
			const positioned = {
				asset: asset("coast", "Coast", 1, {
					wallThumbnail: {
						assetId: "coast",
						kind: "wallThumbnail" as const,
						key,
					},
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<div>
					<button onClick={() => setKey("fenced-new")} type="button">
						Swap thumbnail
					</button>
					<PhotoTile
						onOpen={onOpen}
						positioned={positioned}
						service={service}
					/>
				</div>
			);
		}
		try {
			const screen = await render(<SwapHarness />);
			const tile = screen
				.getByRole("img", { name: "Coast" })
				.element().parentElement;
			const suppressReplacementLoad = (event: Event) => {
				event.stopImmediatePropagation();
			};
			tile?.addEventListener("load", suppressReplacementLoad, true);
			await expect
				.element(screen.getByRole("img", { name: "Coast" }))
				.toBeVisible();
			await screen.getByRole("button", { name: "Swap thumbnail" }).click();
			const image = screen.getByRole("img", { name: "Coast" });
			await expect
				.poll(() => image.element().getAttribute("src"))
				.toContain("fenced-replacement=1");
			expect(getComputedStyle(image.element()).opacity).toBe("0");
			expect(
				screen.getByRole("button", { name: "Open Coast", exact: true }).query(),
			).toBeNull();
			await new Promise((resolve) =>
				window.requestAnimationFrame(() => resolve(undefined)),
			);
			newComplete = true;
			oldDecode.resolve();
			await expect.poll(() => oldDecodeContinuationRan).toBe(true);
			expect(getComputedStyle(image.element()).opacity).toBe("0");
			newDecode.resolve();
			await expect
				.element(
					screen.getByRole("button", { name: "Open Coast", exact: true }),
				)
				.toBeVisible();
			await expect
				.poll(
					() =>
						getComputedStyle(
							screen.getByRole("img", { name: "Coast" }).element(),
						).opacity,
				)
				.toBe("1");
			expect(onOpen).not.toHaveBeenCalled();
			tile?.removeEventListener("load", suppressReplacementLoad, true);
			screen.unmount();
		} finally {
			restoreLoadCapture();
			restoreImageRuntime();
		}
	});

	it("resets a replacement thumbnail until its own load event", async () => {
		const service = new ControlledWallService();
		const onOpen = vi.fn();
		service.setDerivativeUrl("swap-old", "/demo-photos/coast.jpg");
		service.setDerivativeUrl("swap-new", "/demo-photos/forest.jpg?swap=1");
		function SwapHarness() {
			const [key, setKey] = useState("swap-old");
			const positioned = {
				asset: asset("coast", "Coast", 1, {
					wallThumbnail: {
						assetId: "coast",
						kind: "wallThumbnail" as const,
						key,
					},
				}),
				left: 0,
				width: 320,
				height: 220,
			};
			return (
				<div>
					<button onClick={() => setKey("swap-new")} type="button">
						Swap thumbnail
					</button>
					<PhotoTile
						onOpen={onOpen}
						positioned={positioned}
						service={service}
					/>
				</div>
			);
		}
		const screen = await render(<SwapHarness />);
		await expect
			.element(screen.getByRole("img", { name: "Coast" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Swap thumbnail" }).click();
		const image = screen.getByRole("img", { name: "Coast" }).element();
		await expect.poll(() => image.getAttribute("src")).toContain("swap=1");
		expect(getComputedStyle(image).opacity).toBe("0");
		expect(
			screen.getByRole("button", { name: "Open Coast", exact: true }).query(),
		).toBeNull();
		image.dispatchEvent(new Event("load"));
		await expect
			.element(screen.getByRole("button", { name: "Open Coast", exact: true }))
			.toBeVisible();
		expect(onOpen).not.toHaveBeenCalled();
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
					rating: null,
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
		const tile = () =>
			screen
				.getByRole("region", { name: "Photos" })
				.element()
				.querySelector<HTMLElement>('[data-asset-id="broken"]');
		await expect
			.poll(() => tile()?.querySelector<HTMLImageElement>('img[alt="Broken"]'))
			.not.toBeNull();
		const image = tile()?.querySelector<HTMLImageElement>('img[alt="Broken"]');
		expect(image).not.toBeNull();
		image?.dispatchEvent(new Event("error"));
		await expect
			.poll(() =>
				tile()?.querySelector('[aria-label="Photo preview unavailable"]'),
			)
			.not.toBeNull();
		expect(tile()?.querySelector('[data-testid="photo-fallback"]')).toBeNull();
		expect(tile()?.textContent).not.toContain("File unavailable");
		expect(tile()?.textContent).not.toContain("/");
	});
});
