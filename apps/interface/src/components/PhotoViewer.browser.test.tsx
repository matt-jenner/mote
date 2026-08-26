import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import {
	createInMemoryPhotoService,
	type InMemoryPhotoService,
} from "../services/inMemoryPhotoService";
import type { WallAsset } from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { AppShell } from "./AppShell";

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
		...Array.from({ length: 60 }, (_, index) =>
			asset(`photo-${index}`, `Photo ${index}`, index + 2),
		),
	];
	return createInMemoryPhotoService({
		selectedFolderName: "Iceland 2025",
		wallAssets: assets.map((item) => ({
			...item,
			wallThumbnailUrl: `/demo-photos/${item.id}.jpg`,
			screenPreviewUrl:
				item.id === "coast" ? "/demo-photos/missing.jpg" : undefined,
		})),
	});
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
	const tile = view.getByRole("button", { name: `Open ${name}` });
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
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		const back = view.getByRole("button", { name: "Back to photos" });
		await expect.element(back).toBeVisible();
		const folders = view.getByRole("button", { name: "Folders" });
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(back.element());
		expect(document.activeElement).not.toBe(folders.element());
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
});
