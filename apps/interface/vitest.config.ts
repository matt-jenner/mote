import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

const browserOptimizeDeps = {
	include: ["@tauri-apps/api/core", "lucide-react", "react-dom/client"],
};

export default defineConfig({
	plugins: [react()],
	test: {
		projects: [
			{
				test: {
					name: "unit",
					include: ["src/**/*.test.ts"],
					environment: "node",
				},
			},
			{
				plugins: [react()],
				optimizeDeps: browserOptimizeDeps,
				test: {
					name: "browser",
					include: ["src/**/*.browser.test.tsx"],
					browser: {
						enabled: true,
						provider: playwright({
							contextOptions: { reducedMotion: "reduce" },
						}),
						headless: true,
						instances: [{ browser: "webkit" }],
					},
				},
			},
			{
				plugins: [react()],
				optimizeDeps: browserOptimizeDeps,
				test: {
					name: "browser-motion",
					include: ["src/**/*.motion.test.tsx"],
					browser: {
						enabled: true,
						provider: playwright({
							contextOptions: { reducedMotion: "no-preference" },
						}),
						headless: true,
						instances: [{ browser: "webkit" }],
					},
				},
			},
			{
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
						}),
						headless: true,
						instances: [{ browser: "webkit" }],
					},
				},
			},
		],
	},
});
