import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { expect, test } from "@playwright/test";

const phase = process.env.PHOTO_VIEWER_PHASE ?? "beforeRestart";
const enabled = process.env.PHOTO_VIEWER_HEIC_MODE !== "disabled";
const filename = "iphone-8bit.heic";
const recordPath = path.join(
	process.env.PHOTO_VIEWER_STATE_DIR ?? "",
	"heic-record.json",
);

test(`HEIC hosted ${enabled ? "enabled" : "disabled"} ${phase}`, async ({
	page,
	request,
}) => {
	const catalogPath = process.env.PHOTO_VIEWER_CATALOG_PATH;
	expect(catalogPath).toBeTruthy();
	const catalog = new DatabaseSync(catalogPath ?? "", { readOnly: true });
	try {
		if (phase === "offline") {
			if (enabled) {
				const record = JSON.parse(await readFile(recordPath, "utf8"));
				for (const { url, etag } of record) {
					const response = await request.get(url);
					expect(response.status()).toBe(200);
					expect(response.headers()["content-type"]).toBe("image/jpeg");
					expect(response.headers().etag).toBe(etag);
				}
			}
			return;
		}
		await page.goto("/");
		await page.getByRole("button", { name: "Add folder", exact: true }).click();
		const dialog = page.getByRole("dialog", { name: "Choose a folder" });
		await dialog.getByRole("button", { name: "HEIC", exact: true }).click();
		await dialog
			.getByRole("button", { name: "Open this folder", exact: true })
			.click();
		await expect(dialog).toBeHidden();
		// The API exposes photo views, so inspect inventory through the read-only
		// host-mounted catalogue to prove disabled HEIF is still indexed.
		await expect
			.poll(
				() =>
					catalog
						.prepare(
							"SELECT s.completed_at IS NOT NULL AS completed FROM scan_generations s JOIN folder_groups g ON g.id = s.folder_group_id WHERE g.display_path = 'HEIC' ORDER BY s.generation DESC LIMIT 1",
						)
						.get()?.completed,
			)
			.toBe(1);
		const inventory = catalog
			.prepare(
				"SELECT count(*) AS totalAssets, lower(hex(id)) AS id FROM assets WHERE media_kind = 'heif'",
			)
			.get();
		expect(inventory?.totalAssets).toBe(1);
		const preferences = await page.evaluate(() =>
			JSON.parse(localStorage.getItem("photo-viewer.hosted.v1") ?? "null"),
		);
		const selectionId = preferences.selectionId;
		const wallUrl = `/api/v1/selections/${selectionId}/wall?scope=currentFolder&direction=oldestFirst&limit=100`;
		const wallResponse = await request.get(wallUrl);
		expect(wallResponse.status()).toBe(200);
		const wall = await wallResponse.json();
		const totalPhotos = wall.totalCount;
		expect(totalPhotos).toBe(enabled ? 1 : 0);
		expect(wall.indexedCount).toBe(enabled ? 1 : 0);
		if (!enabled) {
			expect(wall.items).toEqual([]);
			expect(wall.previewCounts).toEqual({
				wallReady: 0,
				screenReady: 0,
				wallFailed: 0,
				screenFailed: 0,
			});
			await expect(page.getByAltText(filename, { exact: true })).toHaveCount(0);
			for (const kind of ["wallThumbnail", "screenPreview"]) {
				const response = await request.post(
					`/api/v1/selections/${selectionId}/derivatives`,
					{
						data: {
							scope: "currentFolder",
							request: { assetIds: [inventory?.id], priority: "visible", kind },
						},
					},
				);
				expect(response.status()).toBeGreaterThanOrEqual(400);
				expect(response.status()).toBeLessThan(500);
			}
			for (const table of ["derivatives", "derivative_failures"])
				expect(
					catalog
						.prepare(
							`SELECT count(*) AS count FROM ${table} d JOIN assets a ON a.id = d.asset_id WHERE a.media_kind = 'heif'`,
						)
						.get()?.count,
				).toBe(0);
			return;
		}
		expect(wall.items[0].displayName).toBe(filename);
		expect([wall.items[0].width, wall.items[0].height]).toEqual([29, 100]);
		const wallImage = page.getByAltText(filename, { exact: true });
		await expect(wallImage).toBeVisible();
		await expect
			.poll(() =>
				wallImage.evaluate(
					(element) => (element as HTMLImageElement).naturalWidth,
				),
			)
			.toBe(29);
		const wallDerivativeUrl = await wallImage.getAttribute("src");
		await page
			.getByRole("button", { name: `Open ${filename}`, exact: true })
			.click();
		const viewer = page.getByRole("dialog", { name: "Photo viewer" });
		const screen = viewer.locator('[data-viewer-layer="screenPreview"]');
		await expect(screen).toHaveAttribute("data-ready", "true");
		const screenDerivativeUrl = await screen.getAttribute("src");
		const record = [];
		for (const url of [wallDerivativeUrl, screenDerivativeUrl]) {
			expect(url).toMatch(/^\/api\/v1\/derivatives\//);
			const response = await request.get(url ?? "");
			expect(response.status()).toBe(200);
			expect(response.headers()["content-type"]).toBe("image/jpeg");
			expect((await response.body()).subarray(0, 2)).toEqual(
				Buffer.from([0xff, 0xd8]),
			);
			record.push({ url, etag: response.headers().etag });
		}
		await viewer.getByRole("button", { name: "Back to photos" }).click();
		await page
			.getByRole("button", { name: `Open ${filename}`, exact: true })
			.click();
		await expect(screen).toHaveAttribute("src", screenDerivativeUrl ?? "");
		await expect(screen).toHaveAttribute("data-ready", "true");
		if (phase === "afterRestart")
			expect(record).toEqual(JSON.parse(await readFile(recordPath, "utf8")));
		else await writeFile(recordPath, JSON.stringify(record));
		for (const { url, etag } of record) {
			const response = await request.get(url ?? "", {
				headers: { "if-none-match": etag },
			});
			expect(response.status()).toBe(304);
		}
	} finally {
		catalog.close();
	}
});
