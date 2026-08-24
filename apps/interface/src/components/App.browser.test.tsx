import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { AppShell } from "./AppShell";

const safeAreaProperties = [
	"--safe-area-top",
	"--safe-area-right",
	"--safe-area-bottom",
	"--safe-area-left",
] as const;

function seriousViolations(result: axe.AxeResults) {
	return result.violations.filter(
		(violation) =>
			violation.impact === "serious" || violation.impact === "critical",
	);
}

function contrastRatio(foreground: string, background: string): number {
	const luminance = (color: string) => {
		const channels = color.match(/[\d.]+/g)?.slice(0, 3);
		if (channels?.length !== 3) throw new Error(`Invalid color: ${color}`);
		const [red, green, blue] = channels.map((channel) => {
			const value = Number(channel) / 255;
			return value <= 0.04045
				? value / 12.92
				: ((value + 0.055) / 1.055) ** 2.4;
		}) as [number, number, number];
		return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
	};
	const foregroundLuminance = luminance(foreground);
	const backgroundLuminance = luminance(background);
	return (
		(Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
		(Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
	);
}

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

	afterEach(() => {
		for (const property of safeAreaProperties) {
			document.documentElement.style.removeProperty(property);
		}
		document.documentElement.dataset.theme = "system";
		document.documentElement.style.colorScheme = "light dark";
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
		await expect
			.poll(() => document.documentElement.dataset.theme)
			.toBe("dark");
	});

	it("returns focus to Appearance after choosing an override", async () => {
		const screen = await renderApp();
		const trigger = screen.getByRole("button", { name: "Appearance" });
		await trigger.click();
		await screen.getByRole("radio", { name: "Dark" }).click();
		expect(document.activeElement).toBe(trigger.element());
	});

	it("keeps the open appearance dialog and explicit Dark state accessible", async () => {
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Appearance" }).click();
		expect(seriousViolations(await axe.run(document))).toEqual([]);

		await screen.getByRole("radio", { name: "Dark" }).click();
		await expect
			.poll(() => document.documentElement.dataset.theme)
			.toBe("dark");
		const primary = screen.getByRole("button", { name: "Choose Folder" });
		await expect
			.poll(() => getComputedStyle(primary.element()).backgroundColor)
			.toBe("rgb(108, 158, 219)");
		const primaryStyle = getComputedStyle(primary.element());
		expect(
			contrastRatio(primaryStyle.color, primaryStyle.backgroundColor),
		).toBeGreaterThanOrEqual(4.5);
		expect(seriousViolations(await axe.run(document))).toEqual([]);
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

	it("keeps keyboard focus inside the phone drawer and restores it on Escape", async () => {
		await page.viewport(390, 844);
		const screen = await renderApp();
		const trigger = screen.getByRole("button", { name: "Open sources" });
		await trigger.click();

		const close = screen.getByRole("button", { name: "Close sources" });
		const folders = screen.getByRole("button", { name: "Folders" });
		const workspace = screen.getByRole("region", {
			name: "Photo workspace",
			includeHidden: true,
		});
		expect(document.activeElement).toBe(close.element());
		expect((workspace.element() as HTMLElement).inert).toBe(true);

		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(folders.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(close.element());

		await userEvent.keyboard("{Escape}");
		expect(
			screen.getByRole("dialog", { name: "Sources drawer" }).query(),
		).toBeNull();
		expect(document.activeElement).toBe(trigger.element());
		expect((workspace.element() as HTMLElement).inert).toBe(false);
	});

	it("keeps the phone canvas and drawer inside nonzero safe areas", async () => {
		await page.viewport(390, 844);
		document.documentElement.style.setProperty("--safe-area-top", "20px");
		document.documentElement.style.setProperty("--safe-area-right", "10px");
		document.documentElement.style.setProperty("--safe-area-bottom", "16px");
		document.documentElement.style.setProperty("--safe-area-left", "8px");
		const screen = await renderApp();

		const mainBounds = screen
			.getByRole("main")
			.element()
			.getBoundingClientRect();
		expect(mainBounds.top).toBe(84);
		expect(mainBounds.right).toBe(380);
		expect(mainBounds.bottom).toBeCloseTo(828, 0);
		expect(mainBounds.left).toBe(8);

		await screen.getByRole("button", { name: "Open sources" }).click();
		const drawerBounds = screen
			.getByRole("dialog", { name: "Sources drawer" })
			.element()
			.getBoundingClientRect();
		expect(drawerBounds.top).toBe(20);
		expect(drawerBounds.bottom).toBeCloseTo(828, 0);
		expect(drawerBounds.left).toBe(8);
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
		expect(seriousViolations(result)).toEqual([]);
	});
});
