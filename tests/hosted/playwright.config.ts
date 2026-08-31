import os from "node:os";
import path from "node:path";
import { defineConfig } from "@playwright/test";

const stateDirectory = path.resolve(
	process.env.PHOTO_VIEWER_STATE_DIR ??
		path.join(os.tmpdir(), "photo-viewer-hosted-state"),
);

export default defineConfig({
	testDir: ".",
	testMatch: "hosted.spec.ts",
	fullyParallel: false,
	workers: 1,
	retries: 0,
	timeout: 90_000,
	expect: { timeout: 30_000 },
	outputDir: path.join(stateDirectory, "playwright-output"),
	reporter: [["line"]],
	use: {
		baseURL: process.env.PHOTO_VIEWER_BASE_URL ?? "http://127.0.0.1:18080",
		browserName: "chromium",
		headless: true,
		viewport: { width: 1280, height: 800 },
		contextOptions: { reducedMotion: "reduce" },
		trace: "retain-on-failure",
	},
});
