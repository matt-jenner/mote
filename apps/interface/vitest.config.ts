import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

export default defineConfig({
	plugins: [react()],
	optimizeDeps: {
		include: ["lucide-react"],
	},
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
		],
	},
});
