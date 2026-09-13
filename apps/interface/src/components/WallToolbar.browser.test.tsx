import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type { WallAsset } from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { AppShell } from "./AppShell";
import { WallToolbar } from "./WallToolbar";

const labels = ["Include subfolders", "Oldest first", "Newest first"] as const;

const toolbarAsset: WallAsset = {
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
	wallThumbnail: {
		assetId: "coast",
		kind: "wallThumbnail",
		key: "coast-wall",
	},
	screenPreview: null,
	rating: null,
};

function narrowShellService() {
	return createInMemoryPhotoService({
		selectedFolderName: "A folder name that must truncate",
		wallAssets: [
			{ ...toolbarAsset, wallThumbnailUrl: "/demo-photos/coast.jpg" },
		],
	});
}

function renderShell(service: ReturnType<typeof narrowShellService>) {
	const queryClient = new QueryClient({
		defaultOptions: {
			mutations: { retry: false },
			queries: { retry: false },
		},
	});
	return render(
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
}

function tooltipStyle(control: Element) {
	return getComputedStyle(control, "::after");
}

describe("desktop wall toolbar", () => {
	beforeEach(async () => {
		await page.viewport(1024, 768);
	});

	afterEach(() => {
		document.documentElement.dataset.theme = "system";
	});

	it("uses accessible icon-only controls with hover and focus tooltips", async () => {
		const screen = await render(
			<WallToolbar
				direction="oldestFirst"
				galleryScope="includeSubfolders"
				onDirectionChange={vi.fn()}
				onGalleryScopeChange={vi.fn()}
				status="42 photos"
			/>,
		);
		const controls = labels.map((label) =>
			screen.getByRole("button", { name: label, exact: true }),
		);

		for (const [index, control] of controls.entries()) {
			const element = control.element();
			expect(element.textContent).toBe("");
			expect(element.children).toHaveLength(1);
			expect(element.firstElementChild?.tagName.toLowerCase()).toBe("svg");
			expect(element).toHaveAttribute("aria-label", labels[index]);
			expect(element).toHaveAttribute("data-tooltip", labels[index]);
			const bounds = element.getBoundingClientRect();
			expect(bounds.width).toBeGreaterThanOrEqual(44);
			expect(bounds.height).toBeGreaterThanOrEqual(44);

			await control.hover();
			expect(tooltipStyle(element).content).toBe(`"${labels[index]}"`);
			expect(tooltipStyle(element).visibility).toBe("visible");
		}

		expect(controls[0]?.element()).toHaveAttribute("aria-pressed", "true");
		expect(controls[1]?.element()).toHaveAttribute("aria-pressed", "true");
		expect(controls[2]?.element()).toHaveAttribute("aria-pressed", "false");

		for (const control of controls) {
			control.element().focus();
			expect(document.activeElement).toBe(control.element());
			expect(control.element().matches(":focus-visible")).toBe(true);
			expect(tooltipStyle(control.element()).visibility).toBe("visible");
		}
	});

	it.each([900, 1024, 1440])(
		"keeps view controls on one row at %ipx",
		async (width) => {
			await page.viewport(width, 768);
			const screen = await render(
				<WallToolbar
					direction="newestFirst"
					galleryScope="includeSubfolders"
					onDirectionChange={vi.fn()}
					onGalleryScopeChange={vi.fn()}
					onRetry={vi.fn()}
					progress={{ busy: true, max: 100, status: "indexing", value: 42 }}
					retryable
					status="Indexing a folder with a deliberately long but concise progress description that must truncate before the view controls wrap"
				/>,
			);
			const controls = labels.map((label) =>
				screen.getByRole("button", { name: label, exact: true }).element(),
			);
			const tops = controls.map((control) =>
				Math.round(control.getBoundingClientRect().top),
			);

			expect(new Set(tops).size).toBe(1);
			if (width === 900)
				expect(
					screen.getByRole("status").element().scrollWidth,
				).toBeGreaterThan(screen.getByRole("status").element().clientWidth);
		},
	);

	it("uses compact view options without overflowing the 900px shell when Picks is open", async () => {
		await page.viewport(900, 768);
		const service = narrowShellService();
		const screen = await renderShell(service);
		await screen.getByRole("button", { name: "Choose Folder" }).click();
		await service.finishFixtureScan();
		await screen.getByRole("button", { name: "Picks, 0 picks" }).click();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();

		const workspace = screen
			.getByRole("region", { name: "Photo workspace" })
			.element();
		const header = workspace.querySelector<HTMLElement>("header");
		if (!header) throw new Error("Missing workspace header");
		expect(header.scrollWidth).toBeLessThanOrEqual(header.clientWidth);
		const viewOptions = screen.getByRole("button", { name: "View options" });
		await expect.element(viewOptions).toBeVisible();
		const viewOptionsBounds = viewOptions.element().getBoundingClientRect();
		const headerBounds = header.getBoundingClientRect();
		expect(viewOptionsBounds.width).toBeGreaterThanOrEqual(44);
		expect(viewOptionsBounds.height).toBeGreaterThanOrEqual(44);
		expect(viewOptionsBounds.left).toBeGreaterThanOrEqual(headerBounds.left);
		expect(viewOptionsBounds.right).toBeLessThanOrEqual(headerBounds.right);

		await viewOptions.click();
		const menu = screen.getByRole("dialog", { name: "View options" });
		await expect
			.element(menu.getByRole("button", { name: "Include subfolders" }))
			.toBeVisible();
		await expect
			.element(menu.getByRole("button", { name: "Newest first" }))
			.toBeVisible();
	});

	it("keeps retry and compact view options inside the 900px shell with Picks open", async () => {
		await page.viewport(900, 768);
		const service = narrowShellService();
		service.queryWall = async () => {
			throw new Error("Wall temporarily unavailable");
		};
		const screen = await renderShell(service);
		await screen.getByRole("button", { name: "Choose Folder" }).click();
		const retry = screen.getByRole("button", { name: "Retry" });
		await expect.element(retry).toBeVisible();
		await screen.getByRole("button", { name: "Picks, 0 picks" }).click();
		await expect
			.element(screen.getByRole("complementary", { name: "Picks" }))
			.toBeVisible();

		const workspace = screen
			.getByRole("region", { name: "Photo workspace" })
			.element();
		const header = workspace.querySelector<HTMLElement>("header");
		if (!header) throw new Error("Missing workspace header");
		expect(header.scrollWidth).toBeLessThanOrEqual(header.clientWidth);
		const viewOptions = screen.getByRole("button", { name: "View options" });
		const headerBounds = header.getBoundingClientRect();
		for (const control of [
			retry.element(),
			viewOptions.element(),
			screen.getByRole("button", { name: "Picks, 0 picks" }).element(),
			screen.getByRole("button", { name: "Appearance" }).element(),
		]) {
			const bounds = control.getBoundingClientRect();
			expect(bounds.left).toBeGreaterThanOrEqual(headerBounds.left);
			expect(bounds.right).toBeLessThanOrEqual(headerBounds.right);
		}
		expect(
			retry.element().getBoundingClientRect().height,
		).toBeGreaterThanOrEqual(44);
		await expect.element(viewOptions).toBeVisible();
	});
});
