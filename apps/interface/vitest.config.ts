import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

const browserOptimizeDeps = {
	include: [
		"@tauri-apps/api/core",
		"@tauri-apps/plugin-os",
		"lucide-react",
		"react-dom/client",
	],
};
const testPublicDir = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../../runtime/test-photos",
);
const browser =
	process.env.MOTE_TEST_BROWSER === "chromium" || process.platform !== "darwin"
		? "chromium"
		: "webkit";
const chromiumExecutablePath =
	browser === "chromium"
		? (process.env.MOTE_CHROMIUM_PATH ??
			(process.platform === "linux" &&
			existsSync("/etc/arch-release") &&
			existsSync("/usr/bin/chromium")
				? "/usr/bin/chromium"
				: undefined))
		: undefined;

export default defineConfig({
	plugins: [react()],
	test: {
		projects: [
			{
				test: {
					name: "unit",
					include: ["src/**/*.test.{ts,tsx}"],
					exclude: ["src/**/*.{browser,motion,contrast}.test.tsx"],
					environment: "node",
				},
			},
			{
				publicDir: testPublicDir,
				plugins: [react()],
				optimizeDeps: browserOptimizeDeps,
				test: {
					name: "browser",
					include: ["src/**/*.browser.test.tsx"],
					browser: {
						enabled: true,
						provider: playwright({
							contextOptions: { reducedMotion: "reduce" },
							launchOptions: chromiumExecutablePath
								? { executablePath: chromiumExecutablePath }
								: undefined,
						}),
						headless: true,
						instances: [{ browser }],
					},
				},
			},
			{
				publicDir: testPublicDir,
				plugins: [react()],
				optimizeDeps: browserOptimizeDeps,
				test: {
					name: "browser-motion",
					include: ["src/**/*.motion.test.tsx"],
					browser: {
						enabled: true,
						provider: playwright({
							contextOptions: { reducedMotion: "no-preference" },
							launchOptions: chromiumExecutablePath
								? { executablePath: chromiumExecutablePath }
								: undefined,
						}),
						headless: true,
						instances: [{ browser }],
					},
				},
			},
			{
				publicDir: testPublicDir,
				plugins: [react()],
				optimizeDeps: browserOptimizeDeps,
				test: {
					name: "browser-contrast",
					include: ["src/**/*.contrast.test.tsx"],
					browser: {
						enabled: true,
						provider: playwright({
							contextOptions: {
								forcedColors: "active",
								reducedMotion: "reduce",
							},
							launchOptions: chromiumExecutablePath
								? { executablePath: chromiumExecutablePath }
								: undefined,
						}),
						headless: true,
						instances: [{ browser }],
					},
				},
			},
		],
	},
});
