import { spawn } from "node:child_process";
import { once } from "node:events";
import { copyFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import { expect, type Page, test } from "@playwright/test";

const project = path.resolve(__dirname, "../..");
const webRoot = path.resolve(
	process.env.PHOTO_VIEWER_WEB_ROOT ??
		path.join(project, "apps/interface/dist"),
);

// These fixtures exercise the built hosted app and real Rust API. Source photos
// are disposable copies, with catalogue and cache outside the photo root.
async function hostedFixture(downloads: boolean) {
	await readFile(path.join(webRoot, "index.html"));
	const directory = await mkdtemp(path.join(os.tmpdir(), "mote-picks-"));
	for (const folder of ["A", "B"]) {
		await mkdir(path.join(directory, "photos", folder), { recursive: true });
		await copyFile(
			path.join(project, "runtime/test-photos/demo-photos/coast.jpg"),
			path.join(directory, "photos", folder, `${folder}.jpg`),
		);
	}
	const socket = createServer();
	socket.listen(0, "127.0.0.1");
	await once(socket, "listening");
	const address = socket.address();
	if (!address || typeof address === "string")
		throw new Error("No fixture port");
	const port = address.port;
	await new Promise<void>((resolve) => socket.close(() => resolve()));
	// JPEG-only host fixtures do not need the native HEIC libraries packaged
	// inside the smoke container. Keep their Cargo features explicit in both modes.
	const child = spawn(
		"cargo",
		[
			"run",
			"-p",
			"photo-server",
			"--no-default-features",
			"--features",
			"mote-defaults",
		],
		{
			cwd: project,
			env: {
				...process.env,
				PHOTO_VIEWER_DATA_DIR: path.join(directory, "data"),
				PHOTO_VIEWER_CACHE_DIR: path.join(directory, "cache"),
				PHOTO_VIEWER_SOURCE_ROOT: path.join(directory, "photos"),
				PHOTO_VIEWER_WEB_ROOT: webRoot,
				PHOTO_VIEWER_BIND: `127.0.0.1:${port}`,
				PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS: downloads ? "true" : "",
			},
			stdio: ["ignore", "pipe", "pipe"],
		},
	);
	let output = "";
	child.stdout.on("data", (chunk) => {
		output += String(chunk);
	});
	child.stderr.on("data", (chunk) => {
		output += String(chunk);
	});
	const url = `http://127.0.0.1:${port}`;
	async function close() {
		if (child.exitCode === null && child.signalCode === null) {
			child.kill("SIGTERM");
			// Open SSE streams can keep graceful shutdown waiting on the browser.
			const force = setTimeout(() => child.kill("SIGKILL"), 3_000);
			try {
				await once(child, "exit");
			} finally {
				clearTimeout(force);
			}
		}
		await rm(directory, { recursive: true, force: true });
	}
	try {
		await expect
			.poll(
				async () => {
					if (child.exitCode !== null) throw new Error(output);
					try {
						return (await fetch(`${url}/healthz`)).ok;
					} catch {
						return false;
					}
				},
				{ timeout: 120_000, message: "Hosted fixture did not become healthy" },
			)
			.toBe(true);
	} catch (error) {
		await close();
		throw new Error(`${String(error)}\n${output}`);
	}
	return { url, close, directory };
}

async function chooseFolder(page: Page, folder: "A" | "B") {
	await page.getByRole("button", { name: "Add folder", exact: true }).click();
	const dialog = page.getByRole("dialog", { name: "Choose a folder" });
	await expect(dialog).toHaveAttribute("aria-busy", "false");
	const back = dialog.getByRole("button", { name: "Back", exact: true });
	while (await back.isEnabled()) {
		await back.click();
		await expect(dialog).toHaveAttribute("aria-busy", "false");
	}
	await dialog.getByRole("button", { name: folder, exact: true }).click();
	await dialog.getByRole("button", { name: "Open this folder" }).click();
	await expect(dialog).toBeHidden();
	await expect(
		page.getByRole("button", { name: `Open ${folder}.jpg` }),
	).toBeEnabled();
}

test.describe("Picks hosted integration", () => {
	test.setTimeout(180_000);

	test("retains two-folder order through reload, shortcut removal, Clear/Undo and another tab", async ({
		page,
		context,
	}) => {
		const fixture = await hostedFixture(false);
		try {
			await page.goto(fixture.url);
			for (const folder of ["A", "B"] as const) {
				await chooseFolder(page, folder);
				await page
					.getByRole("button", { name: `Add ${folder}.jpg to picks` })
					.click();
			}
			await page.reload();
			await page.getByRole("button", { name: "Picks, 2 picks" }).click();
			const panel = page.getByRole("complementary", { name: "Picks" });
			await expect(
				panel.getByRole("button", { name: "Review A.jpg" }),
			).toBeVisible();
			await expect(
				panel.getByRole("button", { name: "Review B.jpg" }),
			).toBeVisible();
			await page.getByRole("button", { name: "Options for A" }).click();
			await page.getByRole("menuitem", { name: "Remove", exact: true }).click();
			await expect(
				page.getByRole("button", { name: "Options for A" }),
			).toHaveCount(0);
			await expect(
				page.getByRole("button", { name: "Picks, 2 picks" }),
			).toBeVisible();
			await expect(panel.getByRole("link")).toHaveCount(0);
			await panel
				.getByRole("button", { name: "Review picks", exact: true })
				.click();
			const viewer = page.getByRole("dialog", { name: "Photo viewer" });
			await expect(viewer.getByTestId("viewer-status")).toContainText(
				"A.jpg, Picks · 1 of 2",
			);
			await viewer.getByRole("button", { name: "Next photo" }).click();
			await expect(viewer.getByTestId("viewer-status")).toContainText(
				"B.jpg, Picks · 2 of 2",
			);
			await expect(
				viewer.getByRole("button", { name: "Next photo" }),
			).toBeDisabled();
			await viewer.getByRole("button", { name: "Back to photos" }).click();
			await panel.getByRole("button", { name: "Clear picks" }).click();
			await expect(
				panel.getByText("Add photos to picks as you browse."),
			).toBeVisible();
			await page.getByRole("button", { name: "Undo", exact: true }).click();
			await expect(
				panel.getByRole("button", { name: "Review A.jpg" }),
			).toBeVisible();
			await expect(
				panel.getByRole("button", { name: "Review B.jpg" }),
			).toBeVisible();
			const second = await context.newPage();
			await second.goto(fixture.url);
			await second.getByRole("button", { name: "Picks, 2 picks" }).click();
			await second
				.getByRole("complementary", { name: "Picks" })
				.getByRole("button", { name: "Remove A.jpg" })
				.click();
			await expect(
				panel.getByRole("button", { name: "Review A.jpg" }),
			).toHaveCount(0);
			await expect(
				page.getByRole("button", { name: "Picks, 1 pick" }),
			).toBeVisible();
			await page.reload();
			await page.getByRole("button", { name: "Picks, 1 pick" }).click();
			await expect(
				panel.getByRole("button", { name: "Review B.jpg" }),
			).toBeVisible();
		} finally {
			await fixture.close();
		}
	});

	test("downloads one original only after its enabled row link is selected", async ({
		page,
	}) => {
		const fixture = await hostedFixture(true);
		try {
			const originals: string[] = [];
			page.on("request", (request) => {
				if (new URL(request.url()).pathname.startsWith("/api/v1/originals/"))
					originals.push(request.url());
			});
			await page.goto(fixture.url);
			for (const folder of ["A", "B"] as const) {
				await chooseFolder(page, folder);
				await page
					.getByRole("button", { name: `Add ${folder}.jpg to picks` })
					.click();
			}
			await page.getByRole("button", { name: "Picks, 2 picks" }).click();
			const panel = page.getByRole("complementary", { name: "Picks" });
			await expect(
				panel.getByRole("link", { name: "Download original" }),
			).toHaveCount(2);
			await expect(
				panel.getByRole("button", { name: /Download|Copy/ }),
			).toHaveCount(0);
			await panel
				.getByRole("button", { name: "Review picks", exact: true })
				.click();
			await page.getByRole("button", { name: "Next photo" }).click();
			await page.getByRole("button", { name: "Back to photos" }).click();
			expect(originals).toEqual([]);
			const downloadEvent = page.waitForEvent("download");
			await panel
				.getByRole("listitem")
				.filter({ hasText: "A.jpg" })
				.getByRole("link", { name: "Download original" })
				.click();
			const download = await downloadEvent;
			expect(download.suggestedFilename()).toBe("A.jpg");
			const downloadPath = await download.path();
			if (!downloadPath) throw new Error("Original download did not complete");
			expect(await readFile(downloadPath)).toEqual(
				await readFile(path.join(fixture.directory, "photos/A/A.jpg")),
			);
			expect(originals).toHaveLength(1);
		} finally {
			await fixture.close();
		}
	});
});
