import { mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
	type Browser,
	type BrowserContext,
	expect,
	type Page,
	test,
} from "@playwright/test";

const preferencesKey = "photo-viewer.hosted.v1";
const clientKey = "photo-viewer.client.v1";
const phase = process.env.PHOTO_VIEWER_PHASE ?? "beforeRestart";
const stateDirectory = path.resolve(
	process.env.PHOTO_VIEWER_STATE_DIR ??
		path.join(os.tmpdir(), "photo-viewer-hosted-state"),
);
const recordPath = path.join(stateDirectory, "hosted-record.json");

type Appearance = "dark" | "light";
type Scope = "currentFolder" | "includeSubfolders";
type SortDirection = "oldestFirst" | "newestFirst";

interface BrowserRecord {
	storageState: string;
	selectionId: string;
	clientId: string;
	folder: "A" | "B";
	appearance: Appearance;
	scope: Scope;
	sort: SortDirection;
	firstFilename: string;
	wallDerivativeUrl: string;
	wallEtag: string;
	viewerDerivativeUrl: string;
	viewerEtag: string;
}

interface HostedRecord {
	a: BrowserRecord;
	b: BrowserRecord;
}

interface StoredPreferences {
	selectionId: string | null;
	appearance: string;
	galleryScope: string;
	sortDirection: string;
}

interface BrowserConfiguration {
	folder: "A" | "B";
	appearance: Appearance;
	scope: Scope;
	sort: SortDirection;
	firstFilename: string;
}

interface WallAssetResponse {
	id: string;
	displayName: string;
	availability: string;
	wallThumbnail: DerivativeReferenceResponse | null;
	screenPreview: DerivativeReferenceResponse | null;
}

interface DerivativeReferenceResponse {
	assetId: string;
	kind: "wallThumbnail" | "screenPreview";
	key: string;
}

interface WallResponse {
	items: WallAssetResponse[];
}

async function newContext(
	browser: Browser,
	baseURL: string,
	storageState?: string,
): Promise<BrowserContext> {
	return browser.newContext({
		baseURL,
		storageState,
		viewport: { width: 1280, height: 800 },
		reducedMotion: "reduce",
	});
}

async function storedPreferences(page: Page): Promise<StoredPreferences> {
	return page.evaluate((key) => {
		const value = localStorage.getItem(key);
		if (value === null) throw new Error("hosted preferences are missing");
		return JSON.parse(value) as StoredPreferences;
	}, preferencesKey);
}

async function clientId(page: Page): Promise<string> {
	return page.evaluate((key) => {
		const value = sessionStorage.getItem(key);
		if (!value) throw new Error("session client ID is missing");
		return value;
	}, clientKey);
}

async function setPressed(page: Page, name: string, pressed: boolean) {
	const button = page.getByRole("button", { name, exact: true });
	await expect(button).toBeVisible();
	if ((await button.getAttribute("aria-pressed")) !== String(pressed)) {
		await button.click();
	}
	await expect(button).toHaveAttribute("aria-pressed", String(pressed));
}

async function setAppearance(page: Page, appearance: Appearance) {
	await page.getByRole("button", { name: "Appearance", exact: true }).click();
	await page
		.getByRole("radio", {
			name: appearance === "dark" ? "Dark" : "Light",
			exact: true,
		})
		.click();
	await expect(page.locator("html")).toHaveAttribute("data-theme", appearance);
}

async function seedPreferences(
	page: Page,
	config: BrowserConfiguration,
): Promise<void> {
	await page.addInitScript(
		({ key, preferences }) => {
			if (localStorage.getItem(key) === null) {
				localStorage.setItem(key, JSON.stringify(preferences));
			}
		},
		{
			key: preferencesKey,
			preferences: {
				selectionId: null,
				breadcrumbs: [],
				appearance: config.appearance,
				galleryScope: config.scope,
				sortDirection: config.sort,
			},
		},
	);
}

async function chooseFolder(page: Page, folder: "A" | "B") {
	await page.goto("/", { waitUntil: "domcontentloaded" });
	const folders = page.getByRole("button", { name: "Folders", exact: true });
	await expect(folders).toBeEnabled();
	await folders.click();
	const dialog = page.getByRole("dialog", { name: "Choose a folder" });
	await expect(dialog).toBeVisible();
	await dialog.getByRole("button", { name: folder, exact: true }).click();
	await dialog
		.getByRole("button", { name: "Open this folder", exact: true })
		.click();
	await expect(dialog).toBeHidden();
	await expect(
		page.locator("header").getByText(folder, { exact: true }),
	).toBeVisible();
}

async function firstTile(page: Page, filename: string) {
	const row = page.getByTestId("photo-row-0");
	const tile = row.locator("figure[data-asset-id]").first();
	const image = tile.getByAltText(filename, { exact: true });
	await expect(image).toBeVisible();
	await expect
		.poll(() =>
			image.evaluate(
				(node) => node instanceof HTMLImageElement && node.complete,
			),
		)
		.toBe(true);
	await expect
		.poll(() =>
			image.evaluate((node) =>
				node instanceof HTMLImageElement ? node.naturalWidth : 0,
			),
		)
		.toBeGreaterThan(0);
	await expect(
		tile.getByRole("button", { name: `Open ${filename}` }),
	).toBeVisible();
	return tile;
}

async function visibleTile(page: Page, filename: string) {
	const image = page.getByAltText(filename, { exact: true });
	await expect(image).toBeVisible();
	await expect
		.poll(() =>
			image.evaluate(
				(node) => node instanceof HTMLImageElement && node.complete,
			),
		)
		.toBe(true);
	await expect
		.poll(() =>
			image.evaluate((node) =>
				node instanceof HTMLImageElement ? node.naturalWidth : 0,
			),
		)
		.toBeGreaterThan(0);
}

async function originRelativeUrl(
	page: Page,
	rawUrl: string,
	fallbackBaseURL?: string,
): Promise<string> {
	expect(rawUrl).toMatch(/^\/api\/v1\/derivatives\//);
	const currentUrl = page.url();
	const baseURL = currentUrl.startsWith("http") ? currentUrl : fallbackBaseURL;
	expect(baseURL).toBeTruthy();
	const absolute = new URL(rawUrl, baseURL);
	expect(absolute.origin).toBe(new URL(baseURL ?? "").origin);
	return absolute.href;
}

async function etagFor(page: Page, rawUrl: string): Promise<string> {
	const response = await page.request.get(
		await originRelativeUrl(page, rawUrl),
	);
	expect(response.status()).toBe(200);
	const etag = response.headers().etag;
	expect(etag).toMatch(/^"[^"]+"$/);
	await response.dispose();
	return etag;
}

async function openViewerAndExercise(
	page: Page,
	filename: string,
): Promise<{ url: string; etag: string }> {
	await page.getByRole("button", { name: `Open ${filename}` }).click();
	const viewer = page.getByRole("dialog", { name: "Photo viewer" });
	await expect(viewer).toBeVisible();
	await expect(viewer.getByTestId("viewer-status")).toContainText(filename);
	const filmstrip = viewer.getByRole("group", { name: "Photo filmstrip" });
	await expect(filmstrip).toBeVisible();
	await expect(
		filmstrip.getByRole("button", { name: filename }),
	).toHaveAttribute("aria-current", "true");

	const preview = viewer.locator('[data-viewer-layer="screenPreview"]');
	await expect(preview).toHaveAttribute("data-ready", "true");
	const rawUrl = await preview.getAttribute("src");
	expect(rawUrl).not.toBeNull();
	const etag = await etagFor(page, rawUrl ?? "");

	const layer = viewer.getByTestId("viewer-transform-layer");
	await viewer.getByRole("button", { name: "Zoom in" }).click();
	await expect(layer).toHaveAttribute("data-viewer-mode", "zoomed");
	const beforePan = await layer.getAttribute("style");
	const stage = viewer.getByTestId("viewer-stage");
	const box = await stage.boundingBox();
	expect(box).not.toBeNull();
	if (box) {
		await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
		await page.mouse.down();
		await page.mouse.move(box.x + box.width / 2 - 100, box.y + box.height / 2, {
			steps: 4,
		});
		await page.mouse.up();
	}
	await expect.poll(() => layer.getAttribute("style")).not.toBe(beforePan);
	await viewer.getByRole("button", { name: "Back to photos" }).click();
	await expect(viewer).toBeHidden();
	return { url: rawUrl ?? "", etag };
}

async function configureBrowser(page: Page, config: BrowserConfiguration) {
	await seedPreferences(page, config);
	await chooseFolder(page, config.folder);
	await setAppearance(page, config.appearance);
	await setPressed(
		page,
		"Include subfolders",
		config.scope === "includeSubfolders",
	);
	await setPressed(
		page,
		config.sort === "oldestFirst" ? "Oldest first" : "Newest first",
		true,
	);
	const tile = await firstTile(page, config.firstFilename);
	const wallUrl = await tile
		.getByAltText(config.firstFilename)
		.getAttribute("src");
	expect(wallUrl).not.toBeNull();
	const wallEtag = await etagFor(page, wallUrl ?? "");
	const viewer = await openViewerAndExercise(page, config.firstFilename);
	const preferences = await storedPreferences(page);
	expect(preferences.appearance).toBe(config.appearance);
	expect(preferences.galleryScope).toBe(config.scope);
	expect(preferences.sortDirection).toBe(config.sort);
	expect(preferences.selectionId).not.toBeNull();
	return {
		selectionId: preferences.selectionId ?? "",
		clientId: await clientId(page),
		wallDerivativeUrl: wallUrl ?? "",
		wallEtag,
		viewerDerivativeUrl: viewer.url,
		viewerEtag: viewer.etag,
	};
}

async function assertChildStartsUncached(
	page: Page,
	selectionId: string,
): Promise<void> {
	const wallUrl = `/api/v1/selections/${encodeURIComponent(
		selectionId,
	)}/wall?scope=includeSubfolders&direction=oldestFirst&limit=100`;
	await expect
		.poll(async () => {
			const response = await page.request.get(wallUrl);
			expect(response.status()).toBe(200);
			const wall = (await response.json()) as WallResponse;
			await response.dispose();
			return wall.items.some(
				(item) => item.displayName === "a-child-uncached.jpg",
			);
		})
		.toBe(true);
	const response = await page.request.get(wallUrl);
	expect(response.status()).toBe(200);
	const wall = (await response.json()) as WallResponse;
	await response.dispose();
	const child = wall.items.find(
		(item) => item.displayName === "a-child-uncached.jpg",
	);
	expect(child).toBeTruthy();
	expect(child?.availability).toBe("available");
	expect(child?.wallThumbnail).toBeNull();
	expect(child?.screenPreview).toBeNull();
}

async function assertRestored(page: Page, record: BrowserRecord) {
	await page.goto("/", { waitUntil: "domcontentloaded" });
	await expect(
		page.locator("header").getByText(record.folder, { exact: true }),
	).toBeVisible();
	await expect(page.locator("html")).toHaveAttribute(
		"data-theme",
		record.appearance,
	);
	await expect(
		page.getByRole("button", { name: "Include subfolders" }),
	).toHaveAttribute(
		"aria-pressed",
		String(record.scope === "includeSubfolders"),
	);
	await expect(
		page.getByRole("button", {
			name: record.sort === "oldestFirst" ? "Oldest first" : "Newest first",
		}),
	).toHaveAttribute("aria-pressed", "true");
	await firstTile(page, record.firstFilename);
	const preferences = await storedPreferences(page);
	expect(preferences.selectionId).toBe(record.selectionId);
}

async function assertStableServerCache(
	page: Page,
	record: BrowserRecord,
	fallbackBaseURL?: string,
) {
	const selection = await page.request.get(
		`/api/v1/selections/${encodeURIComponent(record.selectionId)}`,
	);
	expect(selection.status()).toBe(200);
	await selection.dispose();

	for (const [url, etag] of [
		[record.wallDerivativeUrl, record.wallEtag],
		[record.viewerDerivativeUrl, record.viewerEtag],
	] as const) {
		const response = await page.request.get(
			await originRelativeUrl(page, url, fallbackBaseURL),
		);
		expect(response.status()).toBe(200);
		expect(response.headers().etag).toBe(etag);
		await response.dispose();
		const conditional = await page.request.get(
			await originRelativeUrl(page, url, fallbackBaseURL),
			{ headers: { "if-none-match": etag } },
		);
		expect(conditional.status()).toBe(304);
		await conditional.dispose();
	}
}

async function assertOfflineServerCache(
	page: Page,
	record: BrowserRecord,
	baseURL: string,
) {
	const response = await page.request.get(
		`/api/v1/selections/${encodeURIComponent(
			record.selectionId,
		)}/wall?scope=${record.scope}&direction=${record.sort}&limit=100`,
	);
	expect(response.status()).toBe(200);
	const wall = (await response.json()) as WallResponse;
	await response.dispose();
	const asset = wall.items.find(
		(item) => item.displayName === record.firstFilename,
	);
	expect(asset).toBeTruthy();
	expect(asset?.availability).toBe("rootOffline");
	expect(asset?.wallThumbnail).not.toBeNull();
	expect(asset?.screenPreview).not.toBeNull();
	expect(record.wallDerivativeUrl).toBe(
		`/api/v1/derivatives/${encodeURIComponent(
			asset?.wallThumbnail?.key ?? "missing-wall-reference",
		)}`,
	);
	expect(record.viewerDerivativeUrl).toBe(
		`/api/v1/derivatives/${encodeURIComponent(
			asset?.screenPreview?.key ?? "missing-screen-reference",
		)}`,
	);
	await assertStableServerCache(page, record, baseURL);
}

async function readRecord(): Promise<HostedRecord> {
	return JSON.parse(await readFile(recordPath, "utf8")) as HostedRecord;
}

async function restoredContexts(
	browser: Browser,
	baseURL: string,
	record: HostedRecord,
) {
	const contextA = await newContext(browser, baseURL, record.a.storageState);
	const contextB = await newContext(browser, baseURL, record.b.storageState);
	return {
		contextA,
		contextB,
		pageA: await contextA.newPage(),
		pageB: await contextB.newPage(),
	};
}

test(`hosted lifecycle phase: ${phase}`, async ({ browser, baseURL }) => {
	expect(baseURL).toBeTruthy();
	await mkdir(stateDirectory, { recursive: true });

	if (phase === "beforeRestart") {
		const contextA = await newContext(browser, baseURL ?? "");
		const contextB = await newContext(browser, baseURL ?? "");
		try {
			const pageA = await contextA.newPage();
			const pageB = await contextB.newPage();
			const [a, b] = await Promise.all([
				configureBrowser(pageA, {
					folder: "A",
					appearance: "dark",
					scope: "currentFolder",
					sort: "oldestFirst",
					firstFilename: "a-01.jpg",
				}),
				configureBrowser(pageB, {
					folder: "B",
					appearance: "light",
					scope: "includeSubfolders",
					sort: "newestFirst",
					firstFilename: "b-01.jpg",
				}),
			]);
			await assertChildStartsUncached(pageA, a.selectionId);
			expect(a.selectionId).not.toBe(b.selectionId);
			expect(a.clientId).not.toBe(b.clientId);
			const aState = path.join(stateDirectory, "browser-a.json");
			const bState = path.join(stateDirectory, "browser-b.json");
			await contextA.storageState({ path: aState });
			await contextB.storageState({ path: bState });
			const record: HostedRecord = {
				a: {
					...a,
					storageState: aState,
					folder: "A",
					appearance: "dark",
					scope: "currentFolder",
					sort: "oldestFirst",
					firstFilename: "a-01.jpg",
				},
				b: {
					...b,
					storageState: bState,
					folder: "B",
					appearance: "light",
					scope: "includeSubfolders",
					sort: "newestFirst",
					firstFilename: "b-01.jpg",
				},
			};
			await writeFile(recordPath, `${JSON.stringify(record, null, 2)}\n`);
		} finally {
			await contextA.close();
			await contextB.close();
		}
		return;
	}

	const record = await readRecord();
	const { contextA, contextB, pageA, pageB } = await restoredContexts(
		browser,
		baseURL ?? "",
		record,
	);
	try {
		if (phase === "offline") {
			await Promise.all([
				assertOfflineServerCache(pageA, record.a, baseURL ?? ""),
				assertOfflineServerCache(pageB, record.b, baseURL ?? ""),
			]);
		}
		await Promise.all([
			assertRestored(pageA, record.a),
			assertRestored(pageB, record.b),
		]);
		const [newClientA, newClientB] = await Promise.all([
			clientId(pageA),
			clientId(pageB),
		]);
		expect(newClientA).not.toBe(record.a.clientId);
		expect(newClientB).not.toBe(record.b.clientId);
		expect(newClientA).not.toBe(newClientB);

		if (phase === "afterRestart") {
			await Promise.all([
				assertStableServerCache(pageA, record.a),
				assertStableServerCache(pageB, record.b),
			]);
			await Promise.all([
				openViewerAndExercise(pageA, record.a.firstFilename),
				openViewerAndExercise(pageB, record.b.firstFilename),
			]);
			return;
		}

		if (phase !== "offline") {
			throw new Error(`unknown PHOTO_VIEWER_PHASE: ${phase}`);
		}
		await Promise.all([
			openViewerAndExercise(pageA, record.a.firstFilename),
			openViewerAndExercise(pageB, record.b.firstFilename),
		]);

		await setPressed(pageA, "Include subfolders", true);
		const child = await expect
			.poll(async () => {
				const response = await pageA.request.get(
					`/api/v1/selections/${encodeURIComponent(
						record.a.selectionId,
					)}/wall?scope=includeSubfolders&direction=oldestFirst&limit=100`,
				);
				expect(response.status()).toBe(200);
				const wall = (await response.json()) as WallResponse;
				await response.dispose();
				return wall.items.find(
					(item) => item.displayName === "a-child-uncached.jpg",
				);
			})
			.toBeTruthy();
		void child;

		const response = await pageA.request.get(
			`/api/v1/selections/${encodeURIComponent(
				record.a.selectionId,
			)}/wall?scope=includeSubfolders&direction=oldestFirst&limit=100`,
		);
		expect(response.status()).toBe(200);
		const wall = (await response.json()) as WallResponse;
		await response.dispose();
		const childAsset = wall.items.find(
			(item) => item.displayName === "a-child-uncached.jpg",
		);
		expect(childAsset).toBeTruthy();
		expect(childAsset?.availability).toBe("rootOffline");
		expect(childAsset?.wallThumbnail).toBeNull();
		expect(childAsset?.screenPreview).toBeNull();
		const childTile = pageA.locator(
			`figure[data-asset-id="${childAsset?.id ?? "missing"}"]`,
		);
		await expect(childTile.getByText("File unavailable")).toBeVisible();
		await expect(
			childTile.getByRole("button", { name: "Open a-child-uncached.jpg" }),
		).toHaveCount(0);
	} finally {
		await contextA.close();
		await contextB.close();
	}
});

test("mounted root allows contained child selection and scoped browsing", async ({
	browser,
	baseURL,
}) => {
	test.skip(
		phase !== "beforeRestart",
		"The mounted-folder journey runs before retained-volume restart",
	);
	expect(baseURL).toBeTruthy();
	const context = await newContext(browser, baseURL ?? "");
	try {
		const page = await context.newPage();
		await page.goto("/", { waitUntil: "domcontentloaded" });
		const folders = page.getByRole("button", { name: "Folders", exact: true });
		await expect(folders).toBeEnabled();
		await folders.click();
		const dialog = page.getByRole("dialog", { name: "Choose a folder" });
		await expect(dialog.getByRole("button", { name: "Back" })).toBeDisabled();
		const nested = dialog.getByRole("button", {
			name: "Nested",
			exact: true,
		});
		await expect(nested).toBeVisible({ timeout: 5_000 });
		await nested.click();
		await dialog.getByRole("button", { name: "Album", exact: true }).click();
		await dialog
			.getByRole("button", { name: "Open this folder", exact: true })
			.click();
		await expect(dialog).toBeHidden();
		await expect(
			page.locator("header").getByText("Album", { exact: true }),
		).toBeVisible();

		await setPressed(page, "Include subfolders", false);
		await visibleTile(page, "album-current.jpg");
		await expect(
			page.getByAltText("album-descendant.jpg", { exact: true }),
		).toHaveCount(0);

		await setPressed(page, "Include subfolders", true);
		await visibleTile(page, "album-descendant.jpg");
	} finally {
		await context.close();
	}
});
