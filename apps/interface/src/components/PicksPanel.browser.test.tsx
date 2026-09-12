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

const previewWarning: WallAsset = {
	...coast,
	id: "preview-warning",
	displayName: "IMG_4172.jpg",
	provisionalOrder: 3,
	warning: { code: "previewUnavailable", retryable: true },
};

async function renderPicksApp(
	options: { previewWarning?: boolean } = {},
): Promise<{
	screen: Awaited<ReturnType<typeof render>>;
	service: InMemoryPhotoService;
}> {
	const service = createInMemoryPhotoService({
		selectedFolderName: "Family",
		wallAssets: [
			{ ...coast, wallThumbnailUrl: "/demo-photos/coast.jpg" },
			{ ...unavailable, wallThumbnailUrl: "/demo-photos/coast.jpg" },
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

describe("responsive Picks panel", () => {
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
