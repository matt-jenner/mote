import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { SourceUnavailableContext } from "../folders/SourceAvailabilityContext";
import type { PickListSnapshot } from "../picks/pickList";
import { createHttpPhotoService } from "../services/httpPhotoService";
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
import { PickRow } from "./PickRow";
import { PickToast } from "./PickToast";

const safeAreaProperties = [
	"--safe-area-top",
	"--safe-area-right",
	"--safe-area-bottom",
	"--safe-area-left",
] as const;

async function wcagViolations() {
	return (
		await axe.run(document, {
			runOnly: {
				type: "tag",
				values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"],
			},
		})
	).violations;
}

async function captureCopyState(state: string) {
	const previous = document.documentElement.dataset.theme;
	try {
		for (const theme of ["light", "dark"]) {
			document.documentElement.dataset.theme = theme;
			await page.screenshot({
				path: `../../.vitest-attachments/picks-${theme}-1440-copy-${state}.png`,
			});
		}
	} finally {
		if (previous === undefined) delete document.documentElement.dataset.theme;
		else document.documentElement.dataset.theme = previous;
	}
}

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
	warning: null,
	wallThumbnail: {
		assetId: "offline",
		kind: "wallThumbnail",
		key: "offline-wall",
	},
};

const staleSourceWarning: WallAsset = {
	...coast,
	id: "stale-source-warning",
	displayName: "IMG_3094.jpg",
	provisionalOrder: 2,
	warning: { code: "sourceUnavailable", retryable: true },
};

const unpicked: WallAsset = {
	...coast,
	id: "unpicked",
	displayName: "DSC_9999.jpg",
	provisionalOrder: 3,
	wallThumbnail: {
		assetId: "unpicked",
		kind: "wallThumbnail",
		key: "unpicked-wall",
	},
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
		cancelCopy?: PhotoService["cancelOriginalCopy"];
		showFolder?: () => Promise<void>;
		originalAction?: "download" | "none";
		duplicateFilenames?: boolean;
	} = {},
): Promise<{
	screen: Awaited<ReturnType<typeof render>>;
	service: InMemoryPhotoService;
}> {
	const service = createInMemoryPhotoService({
		selectedFolderName: "Family",
		wallAssets: [
			{ ...coast, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			{
				...unavailable,
				displayName: options.duplicateFilenames
					? coast.displayName
					: unavailable.displayName,
				wallThumbnailUrl: "/demo-photos/coast.jpg",
			},
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
		service.cancelOriginalCopy = options.cancelCopy ?? (async () => {});
		service.showLastCopyDestination = options.showFolder ?? (async () => {});
	}
	if (options.originalAction) {
		const hosted = createHttpPhotoService({
			fetch: async () =>
				Response.json({
					rootId: null,
					capabilities: {
						folderBrowser: true,
						video: false,
						originalDownloads: options.originalAction === "download",
					},
					sourceAvailable: true,
				}),
		});
		await hosted.getBootstrapState();
		service.capabilities.originalAction = hosted.capabilities.originalAction;
		service.originalDownloadUrl = hosted.originalDownloadUrl;
		hosted.dispose();
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
	it("enters cancellable zero progress through the native full-copy path", async () => {
		let requested: readonly string[] | null | undefined;
		let finish!: (result: CopyResult) => void;
		const { screen } = await renderPicksApp({
			copy: (ids, listener) => {
				requested = ids;
				listener({ completed: 0, total: 2, item: null });
				return new Promise((resolve) => {
					finish = resolve;
				});
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });

		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();

		expect(requested).toBeNull();
		await expect
			.element(panel.getByText("0 of 2", { exact: true }))
			.toBeVisible();
		await expect
			.element(panel.getByRole("button", { name: "Cancel", exact: true }))
			.toBeEnabled();
		expect(
			panel.getByRole("button", { name: "Clear picks" }).query(),
		).toBeNull();
		finish({ kind: "copyCancelled" });
	});

	it("owns full-width copy progress and cancellation inside the drawer", async () => {
		let progress!: (event: CopyProgress) => void;
		let finish!: (result: CopyResult) => void;
		let finishCancel!: () => void;
		const { screen } = await renderPicksApp({
			copy: (_ids, listener) => {
				progress = listener;
				return new Promise((resolve) => {
					finish = resolve;
				});
			},
			cancelCopy: () =>
				new Promise((resolve) => {
					finishCancel = resolve;
				}),
		});
		const trigger = screen.getByRole("button", { name: "Picks, 2 picks" });
		await trigger.click();
		let panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		progress({ completed: 2, total: 6, item: null });
		const bar = panel.getByRole("progressbar", { name: "Copy originals" });
		await expect
			.element(panel.getByText("2 of 6", { exact: true }))
			.toBeVisible();
		expect(trigger.element().textContent).toBe("Picks2");
		expect(trigger.element().querySelector("progress")).toBeNull();
		expect(panel.element().querySelectorAll("progress")).toHaveLength(1);
		const row = bar.element().parentElement;
		if (!row) throw new Error("copy progress row is missing");
		const bounds = row.getBoundingClientRect();
		const countBounds = panel
			.getByText("2 of 6", { exact: true })
			.element()
			.getBoundingClientRect();
		expect(getComputedStyle(row).display).toBe("grid");
		expect(Math.abs(countBounds.right - bounds.right)).toBeLessThan(1);
		expect(bar.element().getBoundingClientRect().width).toBeGreaterThan(
			bounds.width * 0.65,
		);
		expect(
			panel.getByRole("button", { name: "Clear picks" }).query(),
		).toBeNull();
		await panel.getByRole("button", { name: "Cancel", exact: true }).click();
		await expect
			.element(panel.getByRole("button", { name: "Cancelling..." }))
			.toBeDisabled();
		await panel.getByRole("button", { name: "Close picks" }).click();
		await trigger.click();
		panel = screen.getByRole("complementary", { name: "Picks" });
		await expect
			.element(panel.getByRole("button", { name: "Cancelling..." }))
			.toBeDisabled();
		finish({ kind: "copyCancelled" });
		await expect
			.element(panel.getByRole("button", { name: "Cancelling..." }))
			.toBeDisabled();
		finishCancel();
		await expect
			.element(panel.getByRole("button", { name: "Copy 2 originals…" }))
			.toBeEnabled();
		expect(panel.element().textContent).not.toContain("Copy cancelled");
		await expect
			.poll(() => document.querySelector("[data-toast-id]")?.textContent)
			.toBe("Copy cancelled");
	});

	it("clearing completed copies removes messages and Show folder", async () => {
		const { screen } = await renderPicksApp({
			copy: async () => ({
				kind: "complete",
				copiedCount: 6,
				failedCount: 0,
				warningCode: null,
				items: ["coast", "offline", "three", "four", "five", "six"].map(
					(assetId) => ({
						assetId,
						status: "copied",
						destinationName: `${assetId}.jpg`,
						errorCode: null,
					}),
				),
			}),
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await expect.element(panel.getByText("Copied 6 originals")).toBeVisible();
		await panel.getByRole("button", { name: "Clear picks" }).click();
		expect(panel.element().textContent).not.toContain("Copied 6 originals");
		expect(
			panel.getByRole("button", { name: "Show folder" }).query(),
		).toBeNull();
		expect(panel.element().querySelectorAll("li")).toHaveLength(0);
	});

	it("removes toast motion when reduced motion is requested", async () => {
		const screen = await render(
			<PickToast
				toast={{ id: 1, message: "Copy cancelled", phase: "exiting" }}
			/>,
		);
		const toast = screen.getByText("Copy cancelled").element().parentElement;
		if (!toast) throw new Error("toast container is missing");
		expect(toast.getAttribute("data-phase")).toBe("exiting");
		expect(getComputedStyle(toast).transitionDuration).toBe("0s");
	});
	for (const theme of ["light", "dark"] as const) {
		for (const width of [1440, 768, 390]) {
			it(`keeps Picks accessible at ${width}px in ${theme} appearance`, async () => {
				await page.viewport(width, 844);
				const previousTheme = document.documentElement.dataset.theme;
				const { screen } = await renderPicksApp({
					originalAction: "download",
					previewWarning: true,
				});
				document.documentElement.dataset.theme = theme;
				try {
					await expect
						.element(
							screen.getByRole("button", {
								name: "Remove IMG_4172.jpg from picks",
							}),
						)
						.toHaveAttribute("aria-pressed", "true");
					const wallPick = screen
						.getByRole("button", { name: "Remove IMG_4172.jpg from picks" })
						.element();
					expect(getComputedStyle(wallPick).transitionDuration).toBe("0s");
					expect(await wcagViolations()).toEqual([]);
					await page.screenshot({
						path: `../../.vitest-attachments/picks-${theme}-${width}-wall.png`,
					});
					const trigger = screen.getByRole("button", {
						name: "Picks, 3 picks",
					});
					await trigger.click();
					const panel = screen.getByRole(
						width >= 900 ? "complementary" : "dialog",
						{ name: "Picks" },
					);
					const remove = panel
						.getByRole("button", { name: "Remove DSC_8421.jpg" })
						.element();
					const row = remove.closest("li");
					if (!row) throw new Error("Missing pick row");
					expect(remove.getBoundingClientRect().left).toBeGreaterThan(
						row.getBoundingClientRect().left +
							row.getBoundingClientRect().width / 2,
					);
					expect(await wcagViolations()).toEqual([]);
					await page.screenshot({
						path: `../../.vitest-attachments/picks-${theme}-${width}-panel.png`,
					});
					if (width < 900) {
						for (const control of panel
							.element()
							.querySelectorAll("button, a")) {
							const bounds = control.getBoundingClientRect();
							expect(
								bounds.width,
								control.getAttribute("aria-label") ??
									control.textContent ??
									"control",
							).toBeGreaterThanOrEqual(44);
							expect(
								bounds.height,
								control.getAttribute("aria-label") ??
									control.textContent ??
									"control",
							).toBeGreaterThanOrEqual(44);
						}
						for (const element of [
							panel.element(),
							...panel.element().querySelectorAll("*"),
						]) {
							expect(getComputedStyle(element).transitionDuration).toBe("0s");
						}
					}
					await panel.getByRole("button", { name: "Review picks" }).click();
					const viewer = screen.getByRole("dialog", { name: "Photo viewer" });
					await expect.element(viewer).toBeVisible();
					expect(await wcagViolations()).toEqual([]);
					await page.screenshot({
						path: `../../.vitest-attachments/picks-${theme}-${width}-review.png`,
					});
					await viewer.getByRole("button", { name: "Back to photos" }).click();
					await expect
						.poll(() => document.activeElement)
						.toBe(
							panel.getByRole("button", { name: "Review picks" }).element(),
						);
					await panel.getByRole("button", { name: "Clear picks" }).click();
					await expect
						.element(panel.getByText("Add photos to picks as you browse."))
						.toBeVisible();
					expect(
						[...document.querySelectorAll("[aria-live='polite']")].some(
							(region) => region.textContent?.includes("Picks cleared"),
						),
					).toBe(true);
					await page.screenshot({
						path: `../../.vitest-attachments/picks-${theme}-${width}-empty.png`,
					});
				} finally {
					if (previousTheme === undefined)
						delete document.documentElement.dataset.theme;
					else document.documentElement.dataset.theme = previousTheme;
				}
			});
		}
	}

	it("offers one hosted original link per available row, including preview failures", async () => {
		const { screen } = await renderPicksApp({
			originalAction: "download",
			previewWarning: true,
		});
		await screen.getByRole("button", { name: "Picks, 3 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		const links = panel.element().querySelectorAll<HTMLAnchorElement>("a");
		expect(
			Array.from(links, (link) => [
				link.textContent,
				link.getAttribute("href"),
			]),
		).toEqual([
			["Download original", "/api/v1/originals/coast"],
			["Download original", "/api/v1/originals/preview-warning"],
		]);
		expect(
			panel.getByRole("button", { name: /Copy|Download/i }).query(),
		).toBeNull();
		await expect
			.element(panel.getByRole("button", { name: "Review picks" }))
			.toBeEnabled();
	});

	it("keeps hosted review enabled with one quiet note and no original actions when disabled", async () => {
		const { screen } = await renderPicksApp({ originalAction: "none" });
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		expect(panel.element().querySelectorAll("a")).toHaveLength(0);
		expect(
			panel.getByRole("button", { name: /Copy|Download/i }).query(),
		).toBeNull();
		await expect
			.element(panel.getByText("This site does not offer original downloads."))
			.toBeVisible();
		await panel.getByRole("button", { name: "Review picks" }).click();
		await expect
			.element(screen.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
	});

	it("encodes hosted original identifiers and returns no URL without download capability", async () => {
		const hosted = createHttpPhotoService({
			fetch: async () =>
				Response.json({
					rootId: null,
					capabilities: {
						folderBrowser: true,
						video: false,
						originalDownloads: true,
					},
					sourceAvailable: true,
				}),
		});
		expect(hosted.originalDownloadUrl("photo/with ?#%")).toBeNull();
		await hosted.getBootstrapState();
		expect(hosted.originalDownloadUrl("photo/with ?#%")).toBe(
			"/api/v1/originals/photo%2Fwith%20%3F%23%25",
		);
		hosted.dispose();
	});
	it("retires removed failures and copies the current picks after a new pick is added", async () => {
		const batches: Array<readonly string[] | null> = [];
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
									destinationName: "coast.jpg",
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
					: { kind: "selectionCancelled" };
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await panel.getByRole("button", { name: "Remove IMG_3094.jpg" }).click();
		await service.addPick({
			assetId: "unpicked",
			sourceFolderId: "Family",
			sourceLabel: "Family",
		});
		await expect
			.element(panel.getByRole("button", { name: "Copy 2 originals…" }))
			.toBeEnabled();
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		expect(batches).toEqual([null, null]);
	});

	it("keeps new picks out of retry and retires retry after Clear", async () => {
		const batches: Array<readonly string[] | null> = [];
		const { screen, service } = await renderPicksApp({
			copy: async (ids) => {
				batches.push(ids);
				return batches.length === 1
					? {
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
						}
					: { kind: "selectionCancelled" };
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await service.addPick({
			assetId: "unpicked",
			sourceFolderId: "Family",
			sourceLabel: "Family",
		});
		await panel.getByRole("button", { name: "Retry 2 originals…" }).click();
		expect(batches).toEqual([null, ["coast", "offline"]]);
		await panel.getByRole("button", { name: "Clear picks" }).click();
		await expect
			.element(panel.getByText("Add photos to picks as you browse."))
			.toBeVisible();
		expect(panel.getByRole("button", { name: /Retry/ }).query()).toBeNull();
	});

	it("preserves Undo through copy completion then shows the queued folder action without moving focus", async () => {
		let finish!: (result: CopyResult) => void;
		let shown = 0;
		const { screen, service } = await renderPicksApp({
			copy: () =>
				new Promise((resolve) => {
					finish = resolve;
				}),
			showFolder: async () => {
				shown += 1;
			},
		});
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		const savedPicks = service.getPicks().items;
		await panel.getByRole("button", { name: "Clear picks" }).click();
		const toast = document.querySelector("[data-toast-id]");
		await service.restorePicks(savedPicks);
		await panel.getByRole("button", { name: "Copy 2 originals…" }).click();
		await panel.getByRole("button", { name: "Close picks" }).click();
		const trigger = screen.getByRole("button", { name: /Picks, 2 picks/ });
		trigger.element().focus();
		await new Promise<void>((resolve) => window.setTimeout(resolve, 1_000));
		finish({
			kind: "complete",
			copiedCount: 2,
			failedCount: 0,
			warningCode: null,
			items: ["coast", "offline"].map((assetId) => ({
				assetId,
				status: "copied" as const,
				destinationName: `${assetId}.jpg`,
				errorCode: null,
			})),
		});
		await expect
			.poll(() =>
				[...document.querySelectorAll("[aria-live='polite']")].some((region) =>
					region.textContent?.includes("Copied 2 originals"),
				),
			)
			.toBe(true);
		expect(document.querySelector("[data-toast-id]")).toBe(toast);
		await expect
			.element(screen.getByRole("button", { name: "Undo", exact: true }))
			.toBeVisible();
		await new Promise<void>((resolve) => window.setTimeout(resolve, 3_200));
		await expect
			.element(screen.getByRole("button", { name: "Undo", exact: true }))
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Show folder", exact: true }))
			.toBeVisible();
		expect(document.activeElement).toBe(trigger.element());
		await screen
			.getByRole("button", { name: "Show folder", exact: true })
			.click();
		expect(shown).toBe(1);
	});

	it("shows bounded failure reasons on their matching rows", async () => {
		const { screen } = await renderPicksApp({
			previewWarning: true,
			copy: async () => ({
				kind: "complete",
				copiedCount: 0,
				failedCount: 3,
				warningCode: null,
				items: [
					{
						assetId: "coast",
						status: "failed",
						destinationName: null,
						errorCode: "destination_unavailable",
					},
					{
						assetId: "offline",
						status: "failed",
						destinationName: null,
						errorCode: "source_unavailable",
					},
					{
						assetId: "preview-warning",
						status: "failed",
						destinationName: null,
						errorCode: "/private/raw-error",
					},
				],
			}),
		});
		await screen.getByRole("button", { name: "Picks, 3 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await panel.getByRole("button", { name: "Copy 3 originals…" }).click();
		await expect
			.element(panel.getByText("Destination unavailable", { exact: true }))
			.toBeVisible();
		expect(
			panel
				.getByText("Destination unavailable", { exact: true })
				.element()
				.closest("li")?.textContent,
		).toContain("DSC_8421.jpg");
		await expect
			.element(panel.getByText("Original unavailable", { exact: true }))
			.toBeVisible();
		expect(
			panel
				.getByText("Original unavailable", { exact: true })
				.element()
				.closest("li")?.textContent,
		).toContain("IMG_3094.jpg");
		await expect
			.element(panel.getByText("Couldn't copy original", { exact: true }))
			.toBeVisible();
		expect(panel.element().textContent).not.toContain("/private/");
	});
	it("keeps the frozen copy running when the drawer closes and a pick is removed", async () => {
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
		expect(batches).toEqual([null]);
		progress({ completed: 0, total: 2, item: null });
		await expect
			.element(panel.getByRole("progressbar", { name: "Copy originals" }))
			.toHaveAttribute("value", "0");
		await captureCopyState("progress");
		await panel.getByRole("button", { name: "Remove DSC_8421.jpg" }).click();
		expect(
			panel.getByRole("button", { name: "Clear picks" }).query(),
		).toBeNull();
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
			.element(screen.getByRole("button", { name: "Picks, 1 pick" }))
			.toBeVisible();
		expect(service.getPicks().items).toHaveLength(1);
		expect(batches).toEqual([null]);
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
					: { kind: "selectionCancelled" };
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
		await expect.element(panel.getByText("Original unavailable")).toBeVisible();
		await captureCopyState("partial");
		await expect
			.poll(() => document.querySelector("[data-toast-id]")?.textContent)
			.toContain("Copied 1 of 2");
		await panel.getByRole("button", { name: "Show folder" }).click();
		expect(shown).toBe(1);
		await panel.getByRole("button", { name: "Retry 1 originals…" }).click();
		await expect
			.element(panel.getByRole("button", { name: "Retry 1 originals…" }))
			.toBeEnabled();
		expect(batches).toEqual([null, ["offline"]]);
		expect(service.getPicks().items).toHaveLength(2);
	});

	it("returns cancelled copies to idle and offers no folder for all-failed copies", async () => {
		let attempt = 0;
		const { screen } = await renderPicksApp({
			copy: async () =>
				++attempt === 1
					? { kind: "selectionCancelled" }
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
		await expect
			.element(trigger)
			.toHaveAttribute("aria-controls", "picks-panel-desktop");
		const closedBackground = getComputedStyle(
			trigger.element(),
		).backgroundColor;
		const wall = screen.getByTestId("photo-wall").element();
		const beforeWidth = wall.getBoundingClientRect().width;

		trigger.element().focus();
		await trigger.click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await expect.element(panel).toBeVisible();
		await expect.element(panel).toHaveAttribute("id", "picks-panel-desktop");
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
		await expect.poll(() => document.activeElement).toBe(trigger.element());
		await trigger.click();
		panel.getByRole("button", { name: "Clear picks" }).element().focus();
		await userEvent.keyboard("{Escape}");
		expect(
			screen.getByRole("complementary", { name: "Picks" }).query(),
		).toBeNull();
		await expect.poll(() => document.activeElement).toBe(trigger.element());
		await expect.element(trigger).toHaveAttribute("aria-expanded", "false");
		await expect.element(trigger).toHaveTextContent("2");
	});

	for (const width of [1440, 390]) {
		it(`keeps every Picks control out of reach during wall and pick review at ${width}px`, async () => {
			await page.viewport(width, 844);
			const { screen, service } = await renderPicksApp();
			await service.finishFixtureScan();
			const trigger = screen.getByRole("button", { name: "Picks, 2 picks" });
			const triggerElement = trigger.element() as HTMLButtonElement;
			await screen.getByRole("button", { name: "Open DSC_8421.jpg" }).click();
			await expect
				.element(screen.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			expect(triggerElement.inert).toBe(true);
			expect(triggerElement.getAttribute("aria-hidden")).toBe("true");
			await screen
				.getByRole("dialog", { name: "Photo viewer" })
				.getByRole("button", { name: "Back to photos" })
				.click();
			await expect.poll(() => triggerElement.inert).toBe(false);

			await trigger.click();
			const panel = screen.getByRole(
				width >= 900 ? "complementary" : "dialog",
				{ name: "Picks" },
			);
			const panelElement = panel.element() as HTMLElement;
			await panel.getByRole("button", { name: "Review DSC_8421.jpg" }).click();
			await expect
				.element(screen.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			expect(triggerElement.inert).toBe(true);
			expect(triggerElement.getAttribute("aria-hidden")).toBe("true");
			if (width >= 900) {
				expect(panelElement.inert).toBe(true);
				expect(panelElement.getAttribute("aria-hidden")).toBe("true");
			} else {
				expect(
					screen.getByRole("dialog", { name: "Picks" }).query(),
				).toBeNull();
			}
			await screen
				.getByRole("dialog", { name: "Photo viewer" })
				.getByRole("button", { name: "Back to photos" })
				.click();
			await expect.poll(() => triggerElement.inert).toBe(width < 900);
			await expect
				.element(
					screen.getByRole(width >= 900 ? "complementary" : "dialog", {
						name: "Picks",
					}),
				)
				.toBeVisible();
		});
	}

	it("keeps a preview warning distinct from an unavailable source", async () => {
		const { screen } = await renderPicksApp({ previewWarning: true });
		await screen.getByRole("button", { name: "Picks, 3 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		await expect.element(panel.getByText("Preview unavailable")).toBeVisible();
		await expect.element(panel.getByText("Source unavailable")).toBeVisible();
	});

	it("hides a stale pick-row source warning after the folder becomes available", async () => {
		const screen = await render(
			<PhotoServiceProvider service={createInMemoryPhotoService()}>
				<SourceUnavailableContext value={false}>
					<ul>
						<PickRow
							item={{
								asset: staleSourceWarning,
								assetId: staleSourceWarning.id,
								sourceFolderId: "Archive",
								sourceLabel: "Mountain archive",
							}}
							onRemove={() => undefined}
						/>
					</ul>
				</SourceUnavailableContext>
			</PhotoServiceProvider>,
		);

		expect(screen.getByText("Source unavailable").query()).toBeNull();
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
		const previous = viewer.getByRole("button", { name: "Previous photo" });
		await previous.click();
		await expect.element(previous).toBeDisabled();
		// Complete the disabled-control blur that WebKit may defer until after a keypress.
		(previous.element() as HTMLButtonElement).blur();
		await expect
			.poll(() => viewer.element().contains(document.activeElement))
			.toBe(true);
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

	it("returns mobile review focus to the same asset when filenames repeat", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp({ duplicateFilenames: true });
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		const launchers = sheet
			.element()
			.querySelectorAll<HTMLButtonElement>(
				'button[aria-label="Review DSC_8421.jpg"]',
			);
		expect(launchers).toHaveLength(2);
		launchers[1]?.click();
		const viewer = screen.getByRole("dialog", { name: "Photo viewer" });
		await expect.element(viewer).toBeVisible();
		await viewer.getByRole("button", { name: "Back to photos" }).click();
		await expect
			.poll(() => screen.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		await expect
			.poll(() => document.activeElement)
			.toBe(
				screen
					.getByRole("dialog", { name: "Picks" })
					.element()
					.querySelectorAll<HTMLButtonElement>(
						'button[aria-label="Review DSC_8421.jpg"]',
					)[1],
			);
	});

	it("uses each hydrated row thumbnail as its accessible review launcher", async () => {
		const { screen } = await renderPicksApp();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const panel = screen.getByRole("complementary", { name: "Picks" });
		const thumbnail = panel.getByRole("button", {
			name: "Review DSC_8421.jpg",
		});
		expect(thumbnail.element().querySelector("img")).not.toBeNull();
		await thumbnail.click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("DSC_8421.jpg, Picks · 1 of 2");
	});

	it("removes a retained pick in the viewer after its active folder is removed", async () => {
		const { screen, service } = await renderPicksApp();
		const activeEntryId = service.getSavedFolders().activeEntryId;
		if (!activeEntryId) throw new Error("Missing active saved folder");
		await service.removeSavedFolder(activeEntryId);
		await expect
			.element(screen.getByRole("button", { name: "Picks, 2 picks" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		await screen
			.getByRole("complementary", { name: "Picks" })
			.getByRole("button", { name: "Review DSC_8421.jpg" })
			.click();
		const remove = screen
			.getByRole("dialog", { name: "Photo viewer" })
			.getByRole("button", { name: "Remove DSC_8421.jpg from picks" });
		await expect.element(remove).toBeEnabled();
		await remove.click();
		await expect
			.element(screen.getByTestId("viewer-status"))
			.toHaveTextContent("IMG_3094.jpg, Picks · 1 of 1");
		expect(service.getPicks().items.map((item) => item.assetId)).toEqual([
			"offline",
		]);
	});

	it("opens Picks from the add confirmation without changing membership", async () => {
		const { screen, service } = await renderPicksApp();
		await service.finishFixtureScan();
		await screen
			.getByRole("button", { name: "Add DSC_9999.jpg to picks" })
			.click();
		await expect
			.element(screen.getByRole("button", { name: "View", exact: true }))
			.toBeVisible();
		const before = service.getPicks();
		await screen.getByRole("button", { name: "View", exact: true }).click();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
		expect(service.getPicks()).toEqual(before);
	});

	it("keeps desktop Picks open when Escape closes a covering viewer", async () => {
		const { screen, service } = await renderPicksApp();
		await service.finishFixtureScan();
		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		await screen.getByRole("button", { name: "Open DSC_8421.jpg" }).click();
		await expect
			.element(screen.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();

		await userEvent.keyboard("{Escape}");

		expect(
			screen.getByRole("dialog", { name: "Photo viewer" }).query(),
		).toBeNull();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
	});

	it("leaves immersive review and reveals Picks from the View confirmation", async () => {
		const { screen, service } = await renderPicksApp();
		await service.finishFixtureScan();
		await service.removePick("coast");
		await expect
			.element(screen.getByRole("button", { name: "Picks, 1 pick" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Open DSC_8421.jpg" }).click();
		const viewer = screen.getByRole("dialog", { name: "Photo viewer" });
		await viewer
			.getByRole("button", { name: "Add DSC_8421.jpg to picks" })
			.click();
		await screen.getByRole("button", { name: "View", exact: true }).click();

		expect(viewer.query()).toBeNull();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();
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
		const barElement = bar.element() as HTMLButtonElement;
		await expect
			.element(bar)
			.toHaveAttribute("aria-controls", "picks-sheet-mobile");
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

		barElement.focus();
		await bar.click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		expect(barElement.inert).toBe(true);
		expect(barElement.getAttribute("aria-hidden")).toBe("true");
		expect(
			screen.getByRole("button", { name: "Dismiss picks" }).query(),
		).toBeNull();
		expect(
			document
				.querySelector("[data-testid='picks-dismiss']")
				?.getAttribute("aria-hidden"),
		).toBe("true");
		await expect.element(sheet).toHaveAttribute("aria-modal", "true");
		await expect.element(sheet).toHaveAttribute("id", "picks-sheet-mobile");
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
		expect(document.activeElement).toBe(barElement);
	});

	it("dismisses the mobile sheet by backdrop, platform back, and downward drag", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp();
		const bar = screen.getByRole("button", { name: "Picks, 2 picks" });
		await bar.click();
		await expect.poll(() => window.history.state?.picksSheet).toBe(true);
		document
			.querySelector<HTMLElement>("[data-testid='picks-dismiss']")
			?.click();
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

	it("keeps the mobile status and hosted availability note visually readable", async () => {
		await page.viewport(390, 844);
		const { screen } = await renderPicksApp({ originalAction: "none" });
		const status = screen.getByRole("status").element();
		status.textContent = "2 photos ready";
		expect(status.scrollWidth).toBeLessThanOrEqual(status.clientWidth);

		await screen.getByRole("button", { name: "Picks, 2 picks" }).click();
		const note = screen
			.getByText("This site does not offer original downloads.")
			.element();
		const style = getComputedStyle(note);
		expect(Number.parseFloat(style.paddingLeft)).toBeGreaterThanOrEqual(16);
		expect(Number.parseFloat(style.paddingRight)).toBeGreaterThanOrEqual(16);
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
