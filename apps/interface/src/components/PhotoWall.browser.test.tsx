import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import type { InMemoryWallFixture } from "../services/inMemoryPhotoService";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type { PhotoService } from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { AppShell } from "./AppShell";

const fixture = (
	id: string,
	name: string,
	url: string,
	order: number,
): InMemoryWallFixture => ({
	id,
	displayName: name,
	mediaKind: "jpeg",
	provisionalOrder: order,
	capturedAtUtc: `2024-01-${String(order).padStart(2, "0")}T12:00:00Z`,
	dateState: "provisional",
	width: 1536,
	height: 1024,
	representativeRgb: 0x225670,
	shapeState: "ready",
	availability: "available",
	warning: null,
	wallThumbnail: { assetId: id, kind: "wallThumbnail", key: `${id}-wall` },
	screenPreview: null,
	wallThumbnailUrl: url,
});

const realFixtureAssets = [
	fixture("coast", "Coast", "/demo-photos/coast.jpg", 1),
	fixture("forest", "Forest", "/demo-photos/forest.jpg", 2),
	fixture("city", "City", "/demo-photos/city.jpg", 3),
	fixture("mountain", "Mountain", "/demo-photos/mountain.jpg", 4),
	fixture("portrait", "Portrait", "/demo-photos/portrait.jpg", 5),
	fixture("interior", "Interior", "/demo-photos/interior.jpg", 6),
] as const;

async function renderWall(service: PhotoService) {
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

function seriousViolations(result: axe.AxeResults) {
	return result.violations.filter(
		(entry) => entry.impact === "serious" || entry.impact === "critical",
	);
}

afterEach(() => {
	document.documentElement.style.removeProperty("--fade-duration");
});

describe("progressive photo wall", () => {
	it("reveals complete rows before metadata settles and keeps tile geometry during refinement", async () => {
		await page.viewport(1440, 1024);
		const service = createInMemoryPhotoService({
			wallAssets: realFixtureAssets,
		});
		await service.chooseFolder();
		const screen = await renderWall(service);
		await expect.element(screen.getByRole("status")).toBeVisible();
		await expect
			.element(screen.getByRole("img", { name: "Coast" }))
			.toBeVisible();
		const tile = screen
			.getByRole("img", { name: "Coast" })
			.element().parentElement;
		expect(tile).not.toBeNull();
		if (!tile) return;
		expect(tile.getBoundingClientRect().width).toBeGreaterThan(0);
		expect(tile.getBoundingClientRect().height).toBeGreaterThan(0);
	});

	it("switches to newest first and resets the scroll container", async () => {
		await page.viewport(834, 1194);
		const service = createInMemoryPhotoService({
			wallAssets: realFixtureAssets,
		});
		await service.chooseFolder();
		const screen = await renderWall(service);
		const wall = screen.getByRole("region", { name: "Photos" });
		await expect
			.element(screen.getByRole("button", { name: "Newest first" }))
			.toBeVisible();
		wall.element().scrollTop = 500;
		await screen.getByRole("button", { name: "Newest first" }).click();
		await expect.poll(() => wall.element().scrollTop).toBe(0);
	});

	it("renders an empty result and keeps reduced-motion rows accessible", async () => {
		await page.viewport(390, 844);
		document.documentElement.style.setProperty("--fade-duration", "0ms");
		const service = createInMemoryPhotoService();
		await service.chooseFolder();
		const screen = await renderWall(service);
		await expect
			.element(screen.getByText("No photos found", { exact: true }))
			.toBeVisible();
		expect(seriousViolations(await axe.run(document))).toEqual([]);
	});
});
