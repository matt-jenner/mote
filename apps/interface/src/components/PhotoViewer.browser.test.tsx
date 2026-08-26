import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import {
	createInMemoryPhotoService,
	type InMemoryPhotoService,
} from "../services/inMemoryPhotoService";
import type {
	DerivativeRequest,
	PhotoService,
	WallAsset,
	WallPage,
	WallQueryRequest,
} from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { useViewerPreview } from "../viewer/useViewerPreview";
import { AppShell } from "./AppShell";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerStage } from "./ViewerStage";

function asset(id: string, displayName: string, order: number): WallAsset {
	return {
		id,
		displayName,
		mediaKind: "jpeg",
		provisionalOrder: order,
		capturedAtUtc: `2024-01-${String(order).padStart(2, "0")}T12:00:00Z`,
		dateState: "settled",
		width: 1200,
		height: 800,
		representativeRgb: 0x225670,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: {
			assetId: id,
			kind: "wallThumbnail",
			key: `${id}-wall`,
		},
		screenPreview: null,
		rating: null,
	};
}

function serviceWithReadyPhotos(
	brokenScreenPreview = false,
	photoCount = 60,
	readyScreenPreview = false,
): InMemoryPhotoService {
	const assets = [
		{
			...asset("coast", "Coast", 1),
			screenPreview:
				brokenScreenPreview || readyScreenPreview
					? {
							assetId: "coast",
							kind: "screenPreview" as const,
							key: "coast-screen",
						}
					: null,
		},
		...Array.from({ length: photoCount }, (_, index) =>
			asset(`photo-${index}`, `Photo ${index}`, index + 2),
		),
	];
	const datedAssets = assets.map((item, index) => ({
		...item,
		capturedAtUtc: new Date(Date.UTC(2024, 0, index + 1)).toISOString(),
	}));
	return createInMemoryPhotoService({
		selectedFolderName: "Iceland 2025",
		wallAssets: datedAssets.map((item) => ({
			...item,
			wallThumbnailUrl: `/demo-photos/${item.id}.jpg`,
			screenPreviewUrl:
				item.id === "coast"
					? brokenScreenPreview
						? "/demo-photos/missing.jpg"
						: readyScreenPreview
							? "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=="
							: undefined
					: undefined,
		})),
	});
}

interface PageGate {
	request: WallQueryRequest;
	resolve: (page: WallPage) => void;
}

function installVisualViewportDouble(width: number, height: number) {
	const viewport = new EventTarget() as EventTarget & {
		width: number;
		height: number;
		setSize: (nextWidth: number, nextHeight: number) => void;
	};
	viewport.width = width;
	viewport.height = height;
	viewport.setSize = (nextWidth, nextHeight) => {
		viewport.width = nextWidth;
		viewport.height = nextHeight;
	};
	const descriptor = Object.getOwnPropertyDescriptor(window, "visualViewport");
	Object.defineProperty(window, "visualViewport", {
		configurable: true,
		value: viewport,
	});
	return () => {
		if (descriptor) Object.defineProperty(window, "visualViewport", descriptor);
		else delete (window as { visualViewport?: VisualViewport }).visualViewport;
	};
}

function delayedPaginationService() {
	const service = serviceWithReadyPhotos(false, 120);
	const queryRequests: WallQueryRequest[] = [];
	const queryWall = service.queryWall.bind(service);
	let heldPage: PageGate | null = null;
	service.queryWall = (request) => {
		queryRequests.push(request);
		if (request.cursor === "100" && heldPage === null) {
			return new Promise<WallPage>((resolve) => {
				heldPage = { request, resolve };
			});
		}
		return queryWall(request);
	};
	return {
		service,
		queryRequests,
		releaseNextPage: async () => {
			if (!heldPage) throw new Error("No delayed page is pending");
			const page = await queryWall(heldPage.request);
			heldPage.resolve(page);
			heldPage = null;
		},
	};
}

async function openManyAsset(name: string) {
	const service = serviceWithReadyPhotos(false, 120);
	return openAssetWithService(service, name);
}

async function openAssetWithService(
	service: InMemoryPhotoService,
	name: string,
) {
	const view = await renderViewerWall(service);
	await view.getByRole("button", { name: "Choose Folder" }).click();
	await service.finishFixtureScan();
	const tile = view.getByRole("button", { name: `Open ${name}`, exact: true });
	await expect.element(tile).toBeVisible();
	(tile.element() as HTMLButtonElement).click();
	await expect
		.element(view.getByRole("dialog", { name: "Photo viewer" }))
		.toBeVisible();
	return { service, view };
}

function renderViewerWall(service: InMemoryPhotoService) {
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	return render(
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
}

function previewAsset(id: string, order: number): WallAsset {
	return {
		...asset(id, id.toUpperCase(), order),
		screenPreview: {
			assetId: id,
			kind: "screenPreview",
			key: `${id}-screen`,
		},
	};
}

function previewService(
	requestDerivatives: PhotoService["requestDerivatives"],
): PhotoService {
	return {
		capabilities: { chooseFolder: false, locateFolder: false },
		getBootstrapState: async () => ({
			settings: { appearance: "system" },
			activeSource: null,
		}),
		chooseFolder: async () => ({ kind: "cancelled" }),
		updateAppearance: async (appearance) => ({
			settings: { appearance },
			activeSource: null,
		}),
		queryWall: async (): Promise<WallPage> => ({
			items: [],
			nextCursor: null,
			orderState: "settled",
			sourceWarnings: [],
		}),
		requestDerivatives,
		setWallInteraction: async () => undefined,
		watchWallUpdates: () => () => undefined,
		derivativeUrl: (reference) =>
			`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==#${reference.assetId}-${reference.kind}`,
	};
}

function PreviewHarness({
	assets,
	service,
}: {
	assets: readonly WallAsset[];
	service: PhotoService;
}) {
	const [currentIndex, setCurrentIndex] = useState(0);
	const [generation, setGeneration] = useState(1);
	const preview = useViewerPreview({
		assets,
		currentIndex,
		previewGeneration: generation,
	});
	const current = assets[currentIndex];
	if (!current) return null;
	return (
		<>
			<button
				data-testid="switch-preview"
				onClick={() => {
					setCurrentIndex(1);
					setGeneration(2);
				}}
				type="button"
			>
				Switch
			</button>
			<ViewerStage
				asset={current}
				baseUrl={preview.baseUrl}
				currentUrl={preview.currentUrl}
				largePreviewUnavailable={preview.largePreviewUnavailable}
				previewGeneration={generation}
				service={service}
			/>
		</>
	);
}

async function openAsset(
	name: string,
	brokenScreenPreview = false,
	readyScreenPreview = false,
) {
	const service = serviceWithReadyPhotos(
		brokenScreenPreview,
		60,
		readyScreenPreview,
	);
	const view = await renderViewerWall(service);
	await view.getByRole("button", { name: "Choose Folder" }).click();
	await service.finishFixtureScan();
	if (brokenScreenPreview) {
		service.emitForTest({
			kind: "derivativesReady",
			selectionId: "memory-selection-1",
			derivatives: [
				{
					assetId: "coast",
					kind: "screenPreview",
					key: "coast-screen",
				},
			],
		});
	}
	const tile = view.getByRole("button", { name: `Open ${name}`, exact: true });
	await expect.element(tile).toBeVisible();
	return { service, view, tile };
}

describe("immersive photo viewer checkpoint", () => {
	beforeEach(async () => {
		await page.viewport(1440, 1024);
	});

	afterEach(() => {
		document.documentElement.dataset.theme = "system";
		document.documentElement.style.colorScheme = "light dark";
	});

	it("opens the selected tile above the mounted wall and returns to its exact position", async () => {
		const { view, tile } = await openAsset("Coast");
		const wall = view.getByRole("region", { name: "Photos" });
		wall.element().scrollTop = 420;

		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		expect(document.querySelector("[data-testid='photo-wall']")).not.toBeNull();

		await view.getByRole("button", { name: "Back to photos" }).click();
		expect(wall.element().scrollTop).toBe(420);
		await expect.element(tile).toHaveFocus();
	});

	it("uses the wall thumbnail for the first frame", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const image = document.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		expect(image?.src).toContain("/demo-photos/coast.jpg");
		await expect
			.poll(() => (image?.complete ? image.naturalWidth : 0))
			.toBeGreaterThan(0);
	});

	it("requests the visible photo as a screen preview", async () => {
		const { service, view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) =>
						request.kind === "screenPreview" &&
						request.priority === "visible" &&
						request.assetIds.includes("coast"),
				),
			)
			.toBe(true);
	});

	it("navigates with a horizontal touch swipe and toggles chrome on a tap", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const dispatchTouch = (type: string, clientX: number, clientY: number) =>
			overlay.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX,
					clientY,
					isPrimary: true,
					pointerId: 7,
					pointerType: "touch",
				}),
			);
		dispatchTouch("pointerdown", 620, 400);
		dispatchTouch("pointerup", 540, 410);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const before = controls?.getAttribute("aria-hidden");
		dispatchTouch("pointerdown", 620, 400);
		dispatchTouch("pointerup", 620, 400);
		await expect
			.poll(() => controls?.getAttribute("aria-hidden"))
			.not.toBe(before);
	});

	it("coalesces visual viewport rotation without resetting the viewer", async () => {
		const restoreViewport = installVisualViewportDouble(390, 844);
		const originalScrollIntoView = Element.prototype.scrollIntoView;
		let scrollCalls = 0;
		Element.prototype.scrollIntoView = () => {
			scrollCalls += 1;
		};
		try {
			const { service, view, tile } = await openAsset("Coast", false, true);
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const overlay = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			await expect
				.poll(() =>
					overlay.querySelector(
						"[data-viewer-layer='screenPreview'][data-ready='true']",
					),
				)
				.not.toBeNull();
			const screenPreview = overlay.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview'][data-ready='true']",
			);
			const portraitCapacity = Number(
				overlay
					.querySelector("[data-filmstrip-capacity]")
					?.getAttribute("data-filmstrip-capacity"),
			);
			const beforeScrollCalls = scrollCalls;
			await view.getByRole("button", { name: "Photo information" }).click();
			await expect
				.element(view.getByRole("complementary", { name: "Photo information" }))
				.toBeVisible();
			await expect
				.poll(() =>
					service.derivativeRequests.some(
						(request) =>
							request.kind === "screenPreview" &&
							request.assetIds.includes("coast"),
					),
				)
				.toBe(true);
			const beforeRevision = Number(overlay.dataset.viewportRevision);
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			window.dispatchEvent(new Event("orientationchange"));
			expect(Number(overlay.dataset.viewportRevision)).toBe(beforeRevision);
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			await expect
				.poll(() => Number(overlay.dataset.viewportRevision))
				.toBe(beforeRevision + 1);
			expect(
				overlay
					.querySelector("[data-filmstrip-capacity]")
					?.getAttribute("data-filmstrip-capacity"),
			).toBe("11");
			expect(Number(portraitCapacity)).toBe(5);
			expect(scrollCalls).toBeGreaterThan(beforeScrollCalls);
			expect(
				overlay.querySelector("[data-current-asset='coast']"),
			).not.toBeNull();
			expect(
				overlay.querySelector("[data-testid='photo-info-drawer']"),
			).not.toBeNull();
			expect(
				overlay
					.querySelector("[aria-current='true']")
					?.getAttribute("aria-label"),
			).toBe("Coast");
			expect(
				overlay.querySelector(
					"[data-viewer-layer='screenPreview'][data-ready='true']",
				),
			).toBe(screenPreview);
		} finally {
			Element.prototype.scrollIntoView = originalScrollIntoView;
			restoreViewport();
		}
	});

	it("keeps vertical drawer movement scrolling and horizontal drawer swipes navigating once", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		drawer.style.maxHeight = "120px";
		await expect
			.poll(() => drawer.scrollHeight > drawer.clientHeight)
			.toBe(true);
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const controlsBefore = controls?.getAttribute("aria-hidden");
		const dispatch = (type: string, x: number, y: number) =>
			drawer.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId: 11,
					pointerType: "touch",
				}),
			);
		drawer.scrollTop = 80;
		expect(drawer.scrollTop).toBeGreaterThan(0);
		dispatch("pointerdown", 500, 300);
		dispatch("pointerup", 518, 390);
		expect(controls?.getAttribute("aria-hidden")).toBe(controlsBefore);
		expect(
			view.getByTestId("viewer-stage").element().dataset.currentAsset,
		).toBe("coast");
		dispatch("pointerdown", 500, 300);
		dispatch("pointerup", 420, 310);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(
			view.getByTestId("viewer-stage").element().dataset.currentAsset,
		).toBe("photo-0");
	});

	it("cancels an active swipe when the viewport revision changes", async () => {
		const restoreViewport = installVisualViewportDouble(390, 844);
		try {
			const { view, tile } = await openAsset("Coast");
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const overlay = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			overlay.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					clientX: 620,
					clientY: 400,
					isPrimary: true,
					pointerId: 13,
					pointerType: "touch",
				}),
			);
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			window.dispatchEvent(new Event("orientationchange"));
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			overlay.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					clientX: 520,
					clientY: 410,
					isPrimary: true,
					pointerId: 13,
					pointerType: "touch",
				}),
			);
			expect(
				view.getByTestId("viewer-stage").element().dataset.currentAsset,
			).toBe("coast");
		} finally {
			restoreViewport();
		}
	});

	it("opens information only from Info and keeps it open during navigation", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await view.getByTestId("viewer-stage").click();
		expect(
			view.getByRole("complementary", { name: "Photo information" }).query(),
		).toBeNull();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect.element(view.getByText("Unrated")).toBeVisible();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
	});

	it("closes information on the first Escape and returns to the wall on the second", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		await userEvent.keyboard("{Escape}");
		expect(
			view.getByRole("complementary", { name: "Photo information" }).query(),
		).toBeNull();
		await userEvent.keyboard("{Escape}");
		expect(
			view.getByRole("dialog", { name: "Photo viewer" }).query(),
		).toBeNull();
	});

	it("announces the current position and loads more when pagination remains", async () => {
		const { view } = await openManyAsset("Coast");
		const tile = view.getByRole("button", { name: "Open Coast", exact: true });
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByTestId("viewer-status"))
			.toHaveTextContent("Coast, photo 1 of 100 loaded");
	});

	it("uses quiet mouse timers and toggles touch controls with the stage", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			'[aria-hidden="false"]',
		);
		const stage = view.getByTestId("viewer-stage").element();
		vi.useFakeTimers();
		try {
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect.element(view.getByTestId("viewer-stage")).toBeVisible();
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 1024,
					pointerType: "mouse",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();

			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();
			await vi.advanceTimersByTimeAsync(3499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.poll(() =>
					view.getByRole("group", { name: "Photo filmstrip" }).query(),
				)
				.toBeNull();
		} finally {
			vi.useRealTimers();
		}
	});

	it("starts idle hiding on open and resumes after control focus leaves", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const next = view.getByRole("button", { name: "Next photo" }).element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("false");
			next.focus();
			await vi.advanceTimersByTimeAsync(3000);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			next.blur();
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
		} finally {
			vi.useRealTimers();
		}
	});

	it("starts the desktop inactivity timer when the viewer opens", async () => {
		const { view, tile } = await openAsset("Coast");
		vi.useFakeTimers();
		try {
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const controls = document.querySelector<HTMLElement>(
				"[data-viewer-controls]",
			);
			await vi.advanceTimersByTimeAsync(2500);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			const hiddenNavigation = controls?.querySelectorAll("button") ?? [];
			expect([...hiddenNavigation].every((button) => button.tabIndex < 0)).toBe(
				true,
			);
		} finally {
			vi.useRealTimers();
		}
	});

	it("cycles only visible controls when primary controls are hidden", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const back = view.getByRole("button", { name: "Back to photos" }).element();
		const next = view.getByRole("button", { name: "Next photo" }).element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			back.focus();
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(next);
			expect(document.activeElement).toBe(
				view.getByRole("button", { name: "Photo information" }).element(),
			);
			await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
			expect(document.activeElement).toBe(back);
			expect(
				overlay.querySelector('[data-viewer-controls] [tabindex="-1"]'),
			).not.toBeNull();
		} finally {
			vi.useRealTimers();
		}
	});

	it("cycles visible drawer controls while primary controls are hidden", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const back = view.getByRole("button", { name: "Back to photos" }).element();
		const previous = overlay.querySelector<HTMLButtonElement>(
			'[data-viewer-controls] button[aria-label="Previous photo"]',
		);
		const next = overlay.querySelector<HTMLButtonElement>(
			'[data-viewer-controls] button[aria-label="Next photo"]',
		);
		const close = view
			.getByRole("button", { name: "Close photo information" })
			.element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			back.focus();
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(previous);
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(next);
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(previous);
			close.focus();
			await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
			expect(document.activeElement).not.toBe(
				view.getByRole("button", { name: "Next photo" }).element(),
			);
		} finally {
			vi.useRealTimers();
		}
	});

	it("retries a rejected screen-preview request", async () => {
		const requests: string[][] = [];
		let attempts = 0;
		const service = previewService(async (request) => {
			if (request.assetIds.includes("a")) {
				requests.push([...request.assetIds]);
				if (attempts++ === 0) throw new Error("temporary failure");
			}
		});
		await render(
			<PhotoServiceProvider service={service}>
				<PreviewHarness
					assets={[asset("a", "A", 1), asset("b", "B", 2)]}
					service={service}
				/>
			</PhotoServiceProvider>,
		);

		await expect
			.poll(() => requests.filter((ids) => ids.includes("a")).length)
			.toBe(2);
	});

	it("promotes a near-viewport request to visible after switching assets", async () => {
		const requests: Array<{ assetIds: string[]; priority: string }> = [];
		const service = previewService(async (request) => {
			requests.push({
				assetIds: [...request.assetIds],
				priority: request.priority,
			});
		});
		const assets = [asset("a", "A", 1), asset("b", "B", 2), asset("c", "C", 3)];
		const view = await render(
			<PhotoServiceProvider service={service}>
				<PreviewHarness assets={assets} service={service} />
			</PhotoServiceProvider>,
		);
		await expect
			.poll(() =>
				requests.some(
					(request) =>
						request.assetIds.includes("b") &&
						request.priority === "nearViewport",
				),
			)
			.toBe(true);
		await view.getByTestId("switch-preview").click();
		await expect
			.poll(() =>
				requests.some(
					(request) =>
						request.assetIds.length === 1 &&
						request.assetIds[0] === "b" &&
						request.priority === "visible",
				),
			)
			.toBe(true);
	});

	it("fences an older decode completion when the current asset changes", async () => {
		const pending = new Map<string, () => void>();
		const originalDecode = HTMLImageElement.prototype.decode;
		HTMLImageElement.prototype.decode = function () {
			const id = this.src.includes("a-screenPreview") ? "a" : "b";
			return new Promise<void>((resolve) => pending.set(id, resolve));
		};
		try {
			const service = previewService(async () => undefined);
			const assets = [previewAsset("a", 1), previewAsset("b", 2)];
			const view = await render(
				<PhotoServiceProvider service={service}>
					<PreviewHarness assets={assets} service={service} />
				</PhotoServiceProvider>,
			);
			await expect.poll(() => pending.has("a")).toBe(true);
			// Keep A pending while changing the current asset and generation.
			await view.getByTestId("switch-preview").click();
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-current-asset", "b");
			await expect
				.poll(
					() =>
						document.querySelector<HTMLImageElement>(
							"[data-viewer-layer='screenPreview']",
						)?.dataset.ready,
				)
				.toBe("false");
			await expect.poll(() => pending.has("b")).toBe(true);
			pending.get("b")?.();
			await expect
				.poll(
					() =>
						document.querySelector<HTMLImageElement>(
							"[data-viewer-layer='screenPreview']",
						)?.dataset.ready,
				)
				.toBe("true");
			// A completes after B is ready; this stale completion must be ignored.
			pending.get("a")?.();
			await new Promise((resolve) => setTimeout(resolve, 0));
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-current-asset", "b");
			const screen = document.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview']",
			);
			expect(screen?.src).toContain("b-screenPreview");
			expect(screen?.dataset.ready).toBe("true");
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("advances from a broken screen preview to a decoding wall thumbnail", async () => {
		const { view, tile } = await openAsset("Coast", true);
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect
			.poll(
				() =>
					document.querySelector<HTMLImageElement>(
						"[data-viewer-layer='wallThumbnail']",
					)?.src ?? "",
			)
			.toContain("/demo-photos/coast.jpg");
		const image = document.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		await expect
			.poll(() => (image?.complete ? image.naturalWidth : 0))
			.toBeGreaterThan(0);
	});

	it("keeps keyboard focus inside the viewer instead of covered source controls", async () => {
		const { view, tile } = await openAsset("Photo 1");
		(tile.element() as HTMLButtonElement).click();
		const back = view.getByRole("button", { name: "Back to photos" });
		await expect.element(back).toBeVisible();
		const previous = view.getByRole("button", { name: "Previous photo" });
		const next = view.getByRole("button", { name: "Next photo" });
		const folders = view.getByRole("button", { name: "Folders" });
		const firstThumb = view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Coast", exact: true });
		await back.element().focus();
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(previous.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(next.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(firstThumb.element());
		expect(document.activeElement).not.toBe(folders.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(next.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(previous.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(back.element());
	});

	it("closes when the current asset disappears during an open viewer", async () => {
		const { service, view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		service.emitForTest({
			kind: "resyncRequired",
			selectionId: "memory-selection-1",
		});
		await expect
			.poll(() => view.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		const workspace = view.getByRole("region", { name: "Photo workspace" });
		await expect
			.poll(() => (workspace.element() as HTMLElement).inert)
			.toBe(false);
	});

	it("navigates in wall order without wrapping and bounds the filmstrip", async () => {
		const { view } = await openManyAsset("Coast");
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await userEvent.keyboard("{ArrowLeft}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "coast");
		await expect
			.element(view.getByRole("button", { name: "Previous photo" }))
			.toBeDisabled();
		const filmstrip = view.getByRole("group", { name: "Photo filmstrip" });
		expect(
			filmstrip.element().querySelectorAll("button").length,
		).toBeLessThanOrEqual(31);
		expect(
			filmstrip
				.element()
				.querySelector("button[aria-current='true']")
				?.getAttribute("aria-label"),
		).toBe("Coast");
	});

	it("holds the current photo while loading and then continues into the next page", async () => {
		const delayed = delayedPaginationService();
		const { view } = await openAssetWithService(delayed.service, "Photo 96");
		await expect
			.poll(
				() =>
					delayed.queryRequests.filter((request) => request.cursor === "100")
						.length,
			)
			.toBe(1);
		const next = view.getByRole("button", { name: "Next photo" });
		await expect.element(next).toBeEnabled();
		await next.click();
		await next.click();
		await expect.element(next).toBeDisabled();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-98");
		await delayed.releaseNextPage();
		await expect.element(next).toBeEnabled();
		await next.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-99");
		expect(
			delayed.queryRequests.filter((request) => request.cursor === "100"),
		).toHaveLength(1);
	});

	it("routes visible controls and filmstrip selection through ordered navigation", async () => {
		const { view } = await openManyAsset("Photo 1");
		await view.getByRole("button", { name: "Previous photo" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await view.getByRole("button", { name: "Next photo" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-1");
		await view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 2", exact: true })
			.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
	});

	it("requests missing filmstrip thumbnails near the viewport and scrolls immediately for reduced motion", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
		});
		const assets = [1, 2, 3, 4].map((order) => ({
			...asset(`film-${order}`, `Film ${order}`, order),
			wallThumbnail: null,
		}));
		const scrollCalls: ScrollIntoViewOptions[] = [];
		const originalScrollIntoView = Element.prototype.scrollIntoView;
		const originalMatchMedia = window.matchMedia;
		Element.prototype.scrollIntoView = (options) => {
			scrollCalls.push(options as ScrollIntoViewOptions);
		};
		window.matchMedia = (() =>
			({ matches: true }) as MediaQueryList) as typeof window.matchMedia;
		try {
			await render(
				<ViewerFilmstrip
					assets={assets}
					currentIndex={1}
					onRequestNearViewportDerivatives={(ids) =>
						service.requestDerivatives({
							assetIds: [...ids],
							kind: "wallThumbnail",
							priority: "nearViewport",
						})
					}
					onSelectAsset={() => undefined}
					service={service}
				/>,
			);
			await expect.poll(() => requests.length).toBeGreaterThan(0);
			expect(requests[0]).toMatchObject({
				kind: "wallThumbnail",
				priority: "nearViewport",
			});
			expect(scrollCalls[0]).toMatchObject({
				behavior: "auto",
				block: "nearest",
				inline: "center",
			});
		} finally {
			Element.prototype.scrollIntoView = originalScrollIntoView;
			window.matchMedia = originalMatchMedia;
		}
	});
});
