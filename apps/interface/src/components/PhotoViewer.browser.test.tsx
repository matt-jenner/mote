import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
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
): InMemoryPhotoService {
	const assets = [
		{
			...asset("coast", "Coast", 1),
			screenPreview: brokenScreenPreview
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
				item.id === "coast" ? "/demo-photos/missing.jpg" : undefined,
		})),
	});
}

interface PageGate {
	request: WallQueryRequest;
	resolve: (page: WallPage) => void;
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

async function openAsset(name: string, brokenScreenPreview = false) {
	const service = serviceWithReadyPhotos(brokenScreenPreview);
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
