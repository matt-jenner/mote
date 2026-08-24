import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { beforeEach, describe, expect, it } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import { AppShell } from "./AppShell";

function renderApp(
	service = createInMemoryPhotoService({ selectedFolderName: "Iceland 2025" }),
) {
	const queryClient = new QueryClient({
		defaultOptions: {
			queries: { retry: false },
			mutations: { retry: false },
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

describe("open and return shell", () => {
	beforeEach(async () => {
		await page.viewport(1440, 1024);
	});

	it("opens a folder and shows its persisted display name", async () => {
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Choose Folder" }).click();
		await expect.element(screen.getByText("Iceland 2025")).toBeVisible();
		await expect.element(screen.getByText("Folder ready")).toBeVisible();
	});

	it("applies an explicit dark override", async () => {
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Appearance" }).click();
		await screen.getByRole("radio", { name: "Dark" }).click();
		expect(document.documentElement.dataset.theme).toBe("dark");
	});

	it("uses a drawer trigger instead of a permanent rail on a phone", async () => {
		await page.viewport(390, 844);
		const screen = await renderApp();
		await expect
			.element(screen.getByRole("button", { name: "Open sources" }))
			.toBeVisible();
		await expect
			.element(
				screen.getByRole("navigation", {
					name: "Sources",
					includeHidden: true,
				}),
			)
			.not.toBeVisible();
	});

	it("renders its canvas at the approved desktop, tablet, and phone sizes", async () => {
		for (const [width, height] of [
			[1440, 1024],
			[834, 1194],
			[390, 844],
		] as const) {
			await page.viewport(width, height);
			const screen = await renderApp();
			const main = screen.getByRole("main");
			await expect.element(main).toBeVisible();
			const bounds = main.element().getBoundingClientRect();
			expect(bounds.top).toBe(width < 640 ? 64 : 72);
			expect(bounds.bottom).toBeCloseTo(height, 0);
			screen.unmount();
		}
	});

	it("has no serious accessibility violations", async () => {
		await renderApp();
		const result = await axe.run(document);
		expect(
			result.violations.filter(
				(violation) =>
					violation.impact === "serious" || violation.impact === "critical",
			),
		).toEqual([]);
	});
});
