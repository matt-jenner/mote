import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import type { PickListSnapshot } from "../picks/pickList";
import {
	createInMemoryPhotoService,
	type InMemoryPhotoService,
} from "../services/inMemoryPhotoService";
import type {
	CopyProgress,
	CopyResult,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import "../styles/global.css";
import "../styles/tokens.css";
import { AppShell } from "./AppShell";

const safeAreaProperties = [
	"--safe-area-top",
	"--safe-area-right",
	"--safe-area-bottom",
	"--safe-area-left",
] as const;

const coast: WallAsset = {
	id: "coast",
	displayName: "DSC_8421.jpg",
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
	wallThumbnail: { assetId: "coast", kind: "wallThumbnail", key: "coast-wall" },
	screenPreview: null,
	rating: null,
};

const unavailable: WallAsset = {
	...coast,
	id: "offline",
	displayName: "IMG_3094.jpg",
	provisionalOrder: 2,
	availability: "missing",
	warning: { code: "sourceUnavailable", retryable: true },
	wallThumbnail: {
		assetId: "offline",
		kind: "wallThumbnail",
		key: "offline-wall",
	},
};

const unpicked: WallAsset = {
	...coast,
	id: "unpicked",
	displayName: "DSC_9999.jpg",
	provisionalOrder: 3,
};

const longWall = Array.from(
	{ length: 30 },
	(_, index): WallAsset => ({
		...coast,
		id: `wall-only-${index}`,
		displayName: `WALL_${index}.jpg`,
		provisionalOrder: index + 10,
	}),
);

const previewWarning: WallAsset = {
	...coast,
	id: "preview-warning",
	displayName: "IMG_4172.jpg",
	provisionalOrder: 3,
	warning: { code: "previewUnavailable", retryable: true },
};

async function renderPicksApp(
	options: {
		previewWarning?: boolean;
		longWall?: boolean;
		copy?: PhotoService["copyPickedOriginals"];
		showFolder?: () => Promise<void>;
	} = {},
): Promise<{
	screen: Awaited<ReturnType<typeof render>>;
	service: InMemoryPhotoService;
}> {
	const service = createInMemoryPhotoService({
		selectedFolderName: "Family",
		wallAssets: [
			{ ...coast, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			{ ...unavailable, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			{ ...unpicked, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			...(options.longWall
				? longWall.map((asset) => ({
						...asset,
						wallThumbnailUrl: "/demo-photos/coast.jpg",
					}))
				: []),
			...(options.previewWarning
				? [{ ...previewWarning, wallThumbnailUrl: "/demo-photos/coast.jpg" }]
				: []),
		],
	});
	await service.addPick({
		assetId: coast.id,
		sourceFolderId: "Family",
		sourceLabel: "Family",
	});
	if (options.previewWarning)
		await service.addPick({
			assetId: previewWarning.id,
			sourceFolderId: "Family",
			sourceLabel: "Family",
		});
	await service.addPick({
		assetId: unavailable.id,
		sourceFolderId: "Archive",
		sourceLabel: "Mountain archive",
	});
	if (options.copy) {
		service.capabilities.originalAction = "copy";
		service.copyPickedOriginals = options.copy;
		service.showLastCopyDestination = options.showFolder ?? (async () => {});
	}
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	const screen = await render(
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
	await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
	await screen.getByRole("button", { name: "Choose Folder" }).click();
	return { screen, service };
}

function createControllablePicksService(initial: PickListSnapshot): {
	service: PhotoService;
	publish(snapshot: PickListSnapshot): void;
} {
	const memory = createInMemoryPhotoService({
		selectedFolderName: "Family",
		wallAssets: [
			{ ...coast, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			{ ...unavailable, wallThumbnailUrl: "/demo-photos/coast.jpg" },
		],
	});
	let snapshot = initial;
	const listeners = new Set<(next: PickListSnapshot) => void>();
	return {
		service: {
			...memory,
			getPicks: () => snapshot,
			loadPicks: async () => snapshot,
			watchPicks: (listener) => {
				listeners.add(listener);
				return () => listeners.delete(listener);
			},
		},
		publish(next) {
			snapshot = next;
			for (const listener of listeners) listener(snapshot);
		},
	};
}

async function renderControllablePicksApp(initial: PickListSnapshot): Promise<{
	screen: Awaited<ReturnType<typeof render>>;
	publish(snapshot: PickListSnapshot): void;
}> {
	const { service, publish } = createControllablePicksService(initial);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	const screen = await render(
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
	await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
	await screen.getByRole("button", { name: "Choose Folder" }).click();
	return { screen, publish };
}

describe("responsive Picks panel", () => {
	it("keeps native copy progress in the trigger after closing and clearing picks", async () => {
		let progress!: (event: CopyProgress) => void;
		let finish!: (result: CopyResult) => void;
		const batches: Array<readonly string[] | null> = [];
		const { screen, service } = await renderPicksApp({
			copy: (ids, listener) => {
				batches.push(ids);
				progress = listener;
				return new Promise((resolve) => {
					finish = resolve;
				});
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await expect
			.element(panel.getByRole("button", { name: "Choosing destination…" }))
			.toBeDisabled();
		expect(batches).toEqual([["coast", "offline"]]);
		progress({ completed: 0, total: 2, item: null });
		await expect
			.element(panel.getByRole("progressbar", { name: "Copy originals" }))
			.toHaveAttribute("value", "0");
		await panel.getByRole("button", { name: "Remove DSC_8421.jpg" }).click();
		await panel.getByRole("button", { name: "Clear picks" }).click();
		await expect
			.element(panel.getByRole("progressbar", { name: "Copy originals" }))
			.toHaveAttribute("max", "2");
		await panel.getByRole("button", { name: "Close picks" }).click();
		progress({
			completed: 1,
			total: 2,
			item: {
				assetId: "coast",
				status: "copied",
				destinationName: "one.jpg",
				errorCode: null,
			},
		});
		await expect
			.element(
				screen.getByRole("button", { name: /Picks, 0 picks, Copying 1 \/ 2/ }),
			)
			.toBeVisible();
		expect(service.getPicks().items).toEqual([]);
		expect(batches).toEqual([["coast", "offline"]]);
		finish({
			kind: "complete",
			copiedCount: 2,
			failedCount: 0,
			warningCode: null,
			items: [
				{
					assetId: "coast",
					status: "copied",
					destinationName: "one.jpg",
					errorCode: null,
				},
				{
					assetId: "offline",
					status: "copied",
					destinationName: "two.jpg",
					errorCode: null,
				},
			],
		});
		await expect
			.poll(() => document.querySelector("[data-toast-id]")?.textContent)
			.toContain("Copied 2 originals");
		await expect
			.poll(() =>
				[...document.querySelectorAll("[aria-live='polite']")]
					.map((region) => region.textContent)
					.join(" "),
			)
			.toContain("Copied 2 originals");
	});

	it("marks failed rows and retries only failed originals through a new picker attempt", async () => {
		const batches: Array<readonly string[] | null> = [];
		let shown = 0;
		const { screen, service } = await renderPicksApp({
			copy: async (ids) => {
				batches.push(ids);
				return batches.length === 1
					? {
							kind: "complete",
							copiedCount: 1,
							failedCount: 1,
							warningCode: null,
							items: [
								{
									assetId: "coast",
									status: "copied",
									destinationName: "one.jpg",
									errorCode: null,
								},
								{
									assetId: "offline",
									status: "failed",
									destinationName: null,
									errorCode: "source_unavailable",
								},
							],
						}
					: { kind: "cancelled" };
			},
			showFolder: async () => {
				shown += 1;
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		expect(
			panel.getByRole("button", { name: "Show folder" }).query(),
		).toBeNull();
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await expect.element(panel.getByText("Copy failed")).toBeVisible();
		await expect
			.poll(() => document.querySelector("[data-toast-id]")?.textContent)
			.toContain("Copied 1 of 2");
		await panel.getByRole("button", { name: "Show folder" }).click();
		expect(shown).toBe(1);
		await panel.getByRole("button", { name: "Retry 1 originals…" }).click();
		await expect
			.element(panel.getByRole("button", { name: "Retry 1 originals…" }))
			.toBeEnabled();
		expect(batches).toEqual([["coast", "offline"], ["offline"]]);
		expect(service.getPicks().items).toHaveLength(2);
	});

	it("returns cancelled copies to idle and offers no folder for all-failed copies", async () => {
		let attempt = 0;
		const { screen } = await renderPicksApp({
			copy: async () =>
				++attempt === 1
					? { kind: "cancelled" }
					: {
							kind: "complete",
							copiedCount: 0,
							failedCount: 2,
							warningCode: null,
							items: ["coast", "offline"].map((assetId) => ({
								assetId,
								status: "failed" as const,
								destinationName: null,
								errorCode: "copy_failed",
							})),
						},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await expect
			.element(panel.getByRole("button", { name: "Copy 2 originals…" }))
			.toBeEnabled();
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await expect
			.element(panel.getByRole("button", { name: "Retry 2 originals…" }))
			.toBeVisible();
		expect(
			panel.getByRole("button", { name: "Show folder" }).query(),
		).toBeNull();
	});
	beforeEach(async () => {
		await page.viewport(1440, 1024);
	});

	afterEach(() => {
		for (const property of safeAreaProperties)
			document.documentElement.style.removeProperty(property);
	});

	it("docks a non-modal panel without clearing retained picks", async () => {
		const { screen } = await renderPicksApp();
		const trigger = screen.getByRole("button", { name: "Picks, 2 picks" });
		await expect.element(trigger).toHaveAttribute("aria-expanded", "false");
		const closedBackground = getComputedStyle(
			trigger.element(),
		).backgroundColor;
		const wall = screen.getByTestId("photo-wall").element();
		const beforeWidth = wall.getBoundingClientRect().width;

		trigger.element().focus();
		await trigger.click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await expect.element(panel).toBeVisible();
		expect(getComputedStyle(trigger.element()).backgroundColor).not.toBe(
			closedBackground,
		);
		expect(panel.element().getBoundingClientRect().width).toBeCloseTo(320, -1);
		expect(wall.getBoundingClientRect().width).toBeLessThan(beforeWidth - 250);
		expect(document.activeElement).toBe(trigger.element());
		expect(
			(
				screen
					.getByRole("region", { name: "Photo workspace" })
					.element() as HTMLElement
			).inert,
		).toBe(false);
		await expect.element(screen.getByText("DSC_8421.jpg")).toBeVisible();
		await expect.element(screen.getByText("Mountain archive")).toBeVisible();
		await expect.element(screen.getByText("Source unavailable")).toBeVisible();
		expect(
			getComputedStyle(screen.getByTestId("picks-actions").element()).position,
		).toBe("sticky");

		await panel.getByRole("button", { name: "Close picks" }).click();
		expect(
			screen.getByRole("complementary", { name: "Picks" }).query(),
		).toBeNull();
		await trigger.click();
		await userEvent.keyboard("{Escape}");
		expect(
			screen.getByRole("complementary", { name: "Picks" }).query(),
		).toBeNull();
		await expect.element(trigger).toHaveAttribute("aria-expanded", "false");
		await expect.element(trigger).toHaveTextContent("2");
	});

	it("keeps a preview warning distinct from an unavailable source", async () => {
		const { screen } = await renderPicksApp({ previewWarning: true });
		await screen.getByRole("button", { name: "Picks, 3 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await expect.element(panel.getByText("Preview unavailable")).toBeVisible();
		await expect.element(panel.getByText("Source unavailable")).toBeVisible();
	});

	it("opens an immersive review from the ordered pick list", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		const review = panel.getByRole("button", { name: "Review picks" });

		expect((review.element() as HTMLButtonElement).disabled).toBe(false);
		await review.click();
		await expect
			.element(screen.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("Picks · 1 of 2");
	});

	it("keeps cross-folder review navigation and its filmstrip to picks", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Review picks" }).click();
		const viewer = screen.getByRole("dialog", { name: "Photo viewer" });

		await viewer.getByRole("button", { name: "Next photo" }).click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 2 of 2");
		await viewer.getByRole("button", { name: "Previous photo" }).click();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 2 of 2");
		await viewer
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "DSC_8421.jpg" })
			.click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("DSC_8421.jpg, Picks · 1 of 2");
		expect(
			viewer
				.getByRole("group", { name: "Photo filmstrip" })
				.getByRole("button", { name: "DSC_9999.jpg" })
				.query(),
		).toBeNull();
	});

	it("keeps touch-swipe review navigation within the pick sequence", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		await sheet.getByRole("button", { name: "Review picks" }).click();
		const viewer = screen
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		viewer.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				clientX: 310,
				clientY: 420,
				pointerId: 1,
				pointerType: "touch",
			}),
		);
		viewer.dispatchEvent(
			new PointerEvent("pointerup", {
				bubbles: true,
				clientX: 70,
				clientY: 420,
				pointerId: 1,
				pointerType: "touch",
			}),
		);
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 2 of 2");
		const filmstrip = viewer.querySelector("[aria-label='Photo filmstrip']");
		if (!filmstrip) throw new Error("Missing pick review filmstrip");
		await expect
			.element(
				filmstrip.querySelector<HTMLElement>("[aria-label='IMG_3094.jpg']"),
			)
			.toBeVisible();
		expect(filmstrip.querySelector("[aria-label='DSC_9999.jpg']")).toBeNull();
	});

	it("returns to the Pick panel launcher without moving the wall scroll", async () => {
		const { screen } = await renderPicksApp({ longWall: true });
		const wall = screen
			.getByTestId("photo-wall")
			.element()
			.querySelector<HTMLElement>("[aria-label='Photos']");
		if (!wall) throw new Error("Missing scrollable photo wall");
		wall.scrollTop = 160;
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		const review = panel.getByRole("button", { name: "Review picks" });
		await review.click();
		await screen
			.getByRole("dialog", { name: "Photo viewer" })
			.getByRole("button", { name: "Back to photos" })
			.click();
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		await expect.poll(() => document.activeElement).toBe(review.element());
		expect(wall.scrollTop).toBe(160);
	});

	it("moves to the next pick when removing the current reviewed pick", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		await screen
			.getByRole("complementary", { name: "Picks" })
			.getByRole("button", { name: "Review picks" })
			.click();
		await screen
			.getByRole("dialog", { name: "Photo viewer" })
			.getByRole("button", { name: "Remove DSC_8421.jpg from picks" })
			.click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 1 of 1");
	});

	it("moves to the previous pick, then closes the sole remaining review", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Review picks" }).click();
		const viewer = screen.getByRole("dialog", { name: "Photo viewer" });
		await viewer.getByRole("button", { name: "Next photo" }).click();
		await viewer
			.getByRole("button", { name: "Remove IMG_3094.jpg from picks" })
			.click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("DSC_8421.jpg, Picks · 1 of 1");
		await viewer
			.getByRole("button", { name: "Remove DSC_8421.jpg from picks" })
			.click();
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		await expect
			.element(panel.getByText("Add photos to picks as you browse."))
			.toBeVisible();
	});

	it("does not offer stale, asset-less picks as review targets", async () => {
		const { screen } = await renderControllablePicksApp({
			revision: 1,
			items: [
				{
					assetId: "stale-photo",
					sourceFolderId: "Missing",
					sourceLabel: "Disconnected folder",
					asset: null,
				},
			],
			persistenceError: null,
		});
		await screen.getByRole("button", { name: "Picks, 1 pick" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		expect(
			panel.getByRole("button", { name: "Review stale-photo" }).query(),
		).toBeNull();
		expect(
			(
				panel
					.getByRole("button", { name: "Review picks" })
					.element() as HTMLButtonElement
			).disabled,
		).toBe(true);
	});

	it("moves a rehydrated-away reviewed pick to the next pick, then closes", async () => {
		const initial: PickListSnapshot = {
			revision: 1,
			items: [
				{
					assetId: coast.id,
					sourceFolderId: "Family",
					sourceLabel: "Family",
					asset: coast,
				},
				{
					assetId: unavailable.id,
					sourceFolderId: "Archive",
					sourceLabel: "Mountain archive",
					asset: unavailable,
				},
			],
			persistenceError: null,
		};
		const [coastPick, unavailablePick] = initial.items;
		if (!coastPick || !unavailablePick)
			throw new Error("Missing controlled review picks");
		const { screen, publish } = await renderControllablePicksApp(initial);
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		await screen
			.getByRole("complementary", { name: "Picks" })
			.getByRole("button", { name: "Review picks" })
			.click();
		publish({
			...initial,
			revision: 2,
			items: [{ ...coastPick, asset: null }, unavailablePick],
		});
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 1 of 1");
		publish({
			...initial,
			revision: 3,
			items: initial.items.map((item) => ({ ...item, asset: null })),
		});
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
	});

	it("clears immediately without closing and publishes the shared Undo toast", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Clear picks" }).click();
		await expect
			.element(panel.getByText("Add photos to picks as you browse."))
			.toBeVisible();
		const toast = document.querySelector<HTMLElement>("[data-toast-id]");
		expect(toast?.textContent).toContain("Picks cleared");
		expect(toast?.querySelector("button")?.textContent).toBe("Undo");
	});

	it("removes an individual pick and shows only guidance when the list is empty", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Remove DSC_8421.jpg" }).click();
		await expect
			.element(screen.getByRole("button", { name: "Picks, 1 pick" }))
			.toBeVisible();
		await panel.getByRole("button", { name: "Remove IMG_3094.jpg" }).click();
		await expect
			.element(panel.getByText("Add photos to picks as you browse."))
			.toBeVisible();
		expect(
			panel.getByRole("button", { name: /Review picks/i }).query(),
		).toBeNull();
		expect(
			panel.getByRole("button", { name: /Clear picks/i }).query(),
		).toBeNull();
		expect(
			panel.getByRole("button", { name: /Copy|Download/i }).query(),
		).toBeNull();
	});

	it("uses a safe-area-aware modal sheet that restores focus on mobile dismissal", async () => {
		await page.viewport(390, 844);
		document.documentElement.style.setProperty("--safe-area-bottom", "16px");
		const { screen } = await renderPicksApp();
		const bar = screen.getByRole("button", { name: "Picks, 2 picks" });
		const barBounds = bar.element().getBoundingClientRect();
		expect(barBounds.height).toBeGreaterThanOrEqual(56);
		expect(barBounds.bottom).toBeCloseTo(828, 0);
		const wall = screen.getByTestId("photo-wall").element();
		const wallContent = wall.querySelector<HTMLElement>(
			"[aria-label='Photos'] > div",
		);
		if (!wallContent) throw new Error("Missing photo wall content");
		expect(
			Number.parseFloat(getComputedStyle(wallContent).paddingBottom),
		).toBeGreaterThanOrEqual(72);

		bar.element().focus();
		await bar.click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		await expect.element(sheet).toHaveAttribute("aria-modal", "true");
		const sheetBounds = sheet.element().getBoundingClientRect();
		expect(sheetBounds.height).toBeGreaterThan(680);
		expect(sheetBounds.height).toBeLessThan(730);
		expect(
			(
				screen
					.getByRole("region", { name: "Photo workspace" })
					.element() as HTMLElement
			).inert,
		).toBe(true);
		const close = sheet.getByRole("button", { name: "Close picks" });
		await expect.poll(() => document.activeElement).toBe(close.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(
			sheet.getByRole("button", { name: "Clear picks" }).element(),
		);
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(close.element());

		await userEvent.keyboard("{Escape}");
		expect(screen.getByRole("dialog", { name: "Picks" }).query()).toBeNull();
		expect(document.activeElement).toBe(bar.element());
	});

	it("dismisses the mobile sheet by backdrop, platform back, and downward drag", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp();
		const bar = screen.getByRole("button", { name: "Picks, 2 picks" });
		await bar.click();
		await expect.poll(() => window.history.state?.picksSheet).toBe(true);
		(
			screen
				.getByRole("button", { name: "Dismiss picks" })
				.element() as HTMLButtonElement
		).click();
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Picks" }).query())
			.toBeNull();

		await bar.click();
		window.history.back();
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Picks" }).query())
			.toBeNull();

		await bar.click();
		const sheet = screen.getByRole("dialog", { name: "Picks" }).element();
		const handle = sheet.querySelector<HTMLElement>("[aria-hidden='true']");
		if (!handle) throw new Error("Missing Picks drag handle");
		handle.dispatchEvent(
			new PointerEvent("pointerdown", { bubbles: true, clientY: 420 }),
		);
		handle.dispatchEvent(
			new PointerEvent("pointerup", { bubbles: true, clientY: 520 }),
		);
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Picks" }).query())
			.toBeNull();
	});

	it("unwinds mobile sheet history when the viewport switches to desktop", async () => {
		await page.viewport(390, 844);
		window.history.pushState({ picksViewportTest: true }, "");
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		await expect.poll(() => window.history.state?.picksSheet).toBe(true);

		await page.viewport(900, 844);
		await expect.poll(() => window.history.state?.picksViewportTest).toBe(true);
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
		window.history.back();
		await expect
			.poll(() => window.history.state?.picksViewportTest)
			.toBeUndefined();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
	});

	it("restores sheet focus after removing the focused row and clearing the focused action", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp();
		const bar = screen.getByRole("button", { name: "Picks, 2 picks" });
		await bar.click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		const remove = sheet.getByRole("button", { name: "Remove DSC_8421.jpg" });
		remove.element().focus();
		await remove.click();
		const close = sheet.getByRole("button", { name: "Close picks" });
		await expect.poll(() => document.activeElement).toBe(close.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(
			sheet.getByRole("button", { name: "Clear picks" }).element(),
		);

		sheet.getByRole("button", { name: "Clear picks" }).element().focus();
		await sheet.getByRole("button", { name: "Clear picks" }).click();
		await expect.poll(() => document.activeElement).toBe(close.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(close.element());
	});
});
