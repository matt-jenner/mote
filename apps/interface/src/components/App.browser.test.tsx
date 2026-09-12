import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { emptySavedFolders } from "../folders/savedFolders";
import {
	createInMemoryPhotoService,
	type InMemoryPhotoService,
} from "../services/inMemoryPhotoService";
import type {
	BootstrapState,
	FolderBreadcrumb,
	FolderListing,
	PhotoService,
} from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { AppShell } from "./AppShell";
import { NavigationRail } from "./NavigationRail";

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
	service: PhotoService = createInMemoryPhotoService({
		selectedFolderName: "Iceland 2025",
	}),
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

function hostedFolderService(): PhotoService {
	const memory = createInMemoryPhotoService({ cancelFolderPicker: true });
	return {
		...memory,
		capabilities: {
			chooseFolder: true,
			folderSelection: "hosted",
			locateFolder: false,
			originalAction: "none",
		},
		chooseFolder: async () => {
			throw new Error("Hosted mode must not open the native picker");
		},
		listFolders: async () => ({
			path: "",
			breadcrumbs: [],
			children: [{ name: "Trips", path: "Trips" }],
			imageCount: 0,
		}),
	};
}

function hostedGalleryService(): {
	service: PhotoService;
	memory: InMemoryPhotoService;
} {
	const memory = createInMemoryPhotoService({
		cancelFolderPicker: true,
		wallAssets: [
			{
				id: "aurora",
				displayName: "Aurora",
				mediaKind: "jpeg",
				provisionalOrder: 1,
				capturedAtUtc: "2025-01-01T12:00:00Z",
				dateState: "settled",
				width: 1200,
				height: 800,
				representativeRgb: 0x3d536b,
				shapeState: "ready",
				availability: "available",
				warning: null,
				wallThumbnail: {
					assetId: "aurora",
					kind: "wallThumbnail",
					key: "aurora-wall",
				},
				screenPreview: null,
				rating: null,
				wallThumbnailUrl: "/demo-photos/coast.jpg",
			},
		],
	});
	let breadcrumbs: FolderBreadcrumb[] = [];
	let state: BootstrapState = {
		savedFolders: emptySavedFolders(),
		settings: { appearance: "system", galleryScope: "includeSubfolders" },
		activeSource: null,
	};
	const listings: Record<string, FolderListing> = {
		"": {
			path: "",
			breadcrumbs: [],
			children: [{ name: "Trips", path: "Trips" }],
			imageCount: 0,
		},
		Trips: {
			path: "Trips",
			breadcrumbs: [{ name: "Trips", path: "Trips" }],
			children: [{ name: "Iceland", path: "Trips/Iceland" }],
			imageCount: 0,
		},
		"Trips/Iceland": {
			path: "Trips/Iceland",
			breadcrumbs: [
				{ name: "Trips", path: "Trips" },
				{ name: "Iceland", path: "Trips/Iceland" },
			],
			children: [{ name: "Processed", path: "Trips/Iceland/Processed" }],
			imageCount: 1,
		},
	};
	const service: PhotoService = {
		...memory,
		capabilities: {
			chooseFolder: true,
			folderSelection: "hosted",
			locateFolder: false,
			originalAction: "none",
		},
		getBootstrapState: async () => structuredClone(state),
		chooseFolder: async () => {
			throw new Error("Hosted mode must not open the native picker");
		},
		updateAppearance: async (appearance) => {
			state = { ...state, settings: { ...state.settings, appearance } };
			return structuredClone(state);
		},
		updateGalleryScope: async (galleryScope) => {
			state = { ...state, settings: { ...state.settings, galleryScope } };
			return structuredClone(state);
		},
		listFolders: async (path) => {
			const listing = listings[path];
			if (!listing) throw new Error("That folder is unavailable.");
			return structuredClone(listing);
		},
		selectFolder: async (path) => {
			const listing = listings[path];
			if (!listing) throw new Error("That folder is unavailable.");
			breadcrumbs = structuredClone([...listing.breadcrumbs]);
			state = {
				...state,
				activeSource: {
					id: "memory-source",
					selectionId: "memory-selection-1",
					displayName: listing.breadcrumbs.at(-1)?.name ?? "Photos",
					availability: "available",
				},
			};
			return { kind: "selected", state: structuredClone(state) };
		},
		folderBrowserState: () => ({
			breadcrumbs: structuredClone(breadcrumbs),
			initialPath: breadcrumbs.at(-1)?.path ?? "",
		}),
	};
	return { service, memory };
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
		delete document.documentElement.dataset.accent;
		document.documentElement.style.removeProperty("--accent-color");
		document.documentElement.style.removeProperty("--accent-color-text");
		document.documentElement.style.colorScheme = "light dark";
	});

	it("opens a folder and shows its persisted display name", async () => {
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Choose Folder" }).click();
		await expect
			.element(
				screen
					.getByRole("region", { name: "Photo workspace" })
					.getByText("Iceland 2025"),
			)
			.toBeVisible();
		await expect.element(screen.getByText("Folder ready")).toBeVisible();
	});

	it("presents the approved Mote identity in the empty library", async () => {
		const screen = await renderApp();

		await expect
			.element(screen.getByRole("img", { name: "Mote" }))
			.toBeVisible();
		const heading = screen.getByRole("heading", {
			name: "A simple space for your photos.",
		});
		await expect.element(heading).toBeVisible();
		await expect
			.element(
				screen.getByText(
					"Open a folder to browse your photos without importing or reorganising them.",
				),
			)
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Choose folder" }))
			.toBeVisible();
		expect(getComputedStyle(heading.element()).fontFamily).toContain("Fredoka");
		expect(heading.element().getBoundingClientRect().height).toBeLessThan(60);
	});

	it("keeps keyboard focus and selected rail hover visibly contrasted", async () => {
		const screen = await renderApp();
		const focusSwatch = document.createElement("span");
		focusSwatch.style.color = "var(--focus-ring)";
		document.body.append(focusSwatch);
		const focusRing = getComputedStyle(focusSwatch).color;
		focusSwatch.remove();
		expect(
			contrastRatio(
				focusRing,
				getComputedStyle(screen.getByRole("main").element()).backgroundColor,
			),
		).toBeGreaterThanOrEqual(3);
		expect(
			contrastRatio(
				focusRing,
				getComputedStyle(
					screen.getByRole("navigation", { name: "Sources" }).element(),
				).backgroundColor,
			),
		).toBeGreaterThanOrEqual(3);
		const focusRule = Array.from(document.styleSheets)
			.flatMap((styleSheet) => Array.from(styleSheet.cssRules))
			.find(
				(rule) =>
					rule instanceof CSSStyleRule &&
					rule.selectorText.includes("button:focus-visible"),
			) as CSSStyleRule | undefined;
		expect(focusRule?.style.outline).toBe("3px solid var(--focus-ring)");

		const folders = screen.getByRole("button", {
			name: "Add folder",
			exact: true,
		});
		await folders.hover();
		const hoveredStyle = getComputedStyle(folders.element());
		expect(hoveredStyle.backgroundColor).toBe("rgba(255, 255, 255, 0.07)");
		expect(
			contrastRatio(hoveredStyle.color, "rgb(30, 32, 34)"),
		).toBeGreaterThanOrEqual(4.5);
	});

	it("opens the contained folder browser instead of the native picker in hosted mode", async () => {
		const screen = await renderApp(hostedFolderService());
		await screen
			.getByRole("button", { name: "Add folder", exact: true })
			.click();

		await expect
			.element(screen.getByRole("dialog", { name: "Choose a folder" }))
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Trips" }))
			.toBeVisible();
	});

	it("selects a hosted folder, restores its breadcrumbs, and opens the existing viewer", async () => {
		const { service, memory } = hostedGalleryService();
		const screen = await renderApp(service);
		await screen
			.getByRole("button", { name: "Add folder", exact: true })
			.click();
		await screen.getByRole("button", { name: "Trips" }).click();
		await screen.getByRole("button", { name: "Iceland" }).click();
		await screen.getByRole("button", { name: "Open this folder" }).click();

		expect(
			screen.getByRole("dialog", { name: "Choose a folder" }).query(),
		).toBeNull();
		await expect
			.element(
				screen
					.getByRole("region", { name: "Photo workspace" })
					.getByText("Iceland", { exact: true }),
			)
			.toBeVisible();

		await memory.finishFixtureScan();
		const photo = screen.getByRole("button", {
			name: "Open Aurora",
			exact: true,
		});
		await expect.element(photo).toBeVisible();
		await photo.click();
		await expect
			.element(screen.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Back to photos" }).click();

		await screen
			.getByRole("button", { name: "Add folder", exact: true })
			.click();
		await expect
			.element(
				screen
					.getByRole("navigation", { name: "Folder path" })
					.getByText("Iceland", { exact: true }),
			)
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Processed" }))
			.toBeVisible();
		expect(screen.getByText("Locate Folder").query()).toBeNull();
	});

	it("opens the hosted sheet from the phone sources drawer and restores its trigger", async () => {
		await page.viewport(390, 844);
		const screen = await renderApp(hostedFolderService());
		const sources = screen.getByRole("button", { name: "Open sources" });
		await sources.click();
		await screen
			.getByRole("button", { name: "Add folder", exact: true })
			.click();

		const dialog = screen.getByRole("dialog", { name: "Choose a folder" });
		await expect.element(dialog).toBeVisible();
		expect(
			screen.getByRole("dialog", { name: "Sources drawer" }).query(),
		).toBeNull();
		const bounds = dialog.element().getBoundingClientRect();
		expect(bounds.width).toBeCloseTo(390, 0);
		expect(bounds.height).toBeCloseTo(844, 0);

		await userEvent.keyboard("{Escape}");
		await expect.poll(() => document.activeElement).toBe(sources.element());
	});

	it("completes hosted selection into the wall and viewer at phone width", async () => {
		await page.viewport(390, 844);
		const { service, memory } = hostedGalleryService();
		const screen = await renderApp(service);

		await screen.getByRole("button", { name: "Open sources" }).click();
		await screen
			.getByRole("button", { name: "Add folder", exact: true })
			.click();
		await screen.getByRole("button", { name: "Trips" }).click();
		await screen.getByRole("button", { name: "Iceland" }).click();
		await screen.getByRole("button", { name: "Open this folder" }).click();

		expect(
			screen.getByRole("dialog", { name: "Choose a folder" }).query(),
		).toBeNull();
		await expect
			.element(
				screen
					.getByRole("region", { name: "Photo workspace" })
					.getByText("Iceland", { exact: true }),
			)
			.toBeVisible();

		await memory.finishFixtureScan();
		const photo = screen.getByRole("button", {
			name: "Open Aurora",
			exact: true,
		});
		await expect.element(photo).toBeVisible();
		await photo.click();
		await expect
			.element(screen.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await screen.getByRole("button", { name: "Back to photos" }).click();
		await expect.element(photo).toBeVisible();
	});

	it("applies an explicit dark override", async () => {
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Appearance" }).click();
		await screen.getByRole("radio", { name: "Dark" }).click();
		await expect
			.poll(() => document.documentElement.dataset.theme)
			.toBe("dark");
	});

	it("applies the hosted runtime accent to primary controls", async () => {
		const memory = createInMemoryPhotoService({ cancelFolderPicker: true });
		const state = await memory.getBootstrapState();
		const service: PhotoService = {
			...memory,
			getBootstrapState: async () => ({
				...state,
				accentColor: "#777777",
			}),
		};
		const screen = await renderApp(service);
		const primary = screen.getByRole("button", { name: "Choose Folder" });

		await expect
			.poll(() => document.documentElement.dataset.accent)
			.toBe("custom");
		expect(getComputedStyle(primary.element()).backgroundColor).toBe(
			"rgb(119, 119, 119)",
		);
		expect(
			contrastRatio(
				getComputedStyle(primary.element()).color,
				getComputedStyle(primary.element()).backgroundColor,
			),
		).toBeGreaterThanOrEqual(4.5);
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
			.toBe("rgb(52, 125, 82)");
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
		const picks = screen.getByRole("button", { name: "Picks, 0 picks" });
		await expect.element(picks).toHaveAttribute("aria-expanded", "false");
		await picks.click();
		const sheet = screen.getByRole("dialog", { name: "Picks" });
		await expect
			.element(sheet.getByText("Add photos to picks as you browse."))
			.toBeVisible();
		await sheet.getByRole("button", { name: "Close picks" }).click();
		await expect.element(picks).toHaveFocus();
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
		const folders = screen.getByRole("button", { name: "Add folder" });
		const workspace = screen.getByRole("region", {
			name: "Photo workspace",
			includeHidden: true,
		});
		expect(document.activeElement).toBe(close.element());
		expect((workspace.element() as HTMLElement).inert).toBe(true);
		await close.hover();
		expect(getComputedStyle(close.element()).backgroundColor).toBe(
			"rgba(255, 255, 255, 0.12)",
		);

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

	it("closes the drawer when the viewport grows to desktop width", async () => {
		await page.viewport(390, 844);
		const screen = await renderApp();
		await screen.getByRole("button", { name: "Open sources" }).click();
		const workspace = screen.getByRole("region", {
			name: "Photo workspace",
			includeHidden: true,
		});
		expect((workspace.element() as HTMLElement).inert).toBe(true);

		await page.viewport(1000, 1194);

		await expect
			.poll(() =>
				screen
					.getByRole("dialog", { name: "Sources drawer", includeHidden: true })
					.query(),
			)
			.toBeNull();
		expect((workspace.element() as HTMLElement).inert).toBe(false);
		await expect
			.element(
				screen.getByRole("navigation", {
					name: "Sources",
					includeHidden: true,
				}),
			)
			.toBeVisible();
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
			expect(bounds.top).toBe(width < 900 ? 64 : 72);
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

it("renames a saved entry and removing it clears the active folder", async () => {
	await page.viewport(1200, 800);
	const service = createInMemoryPhotoService({
		selectedFolderName: "Iceland 2025",
	});
	renderApp(service);
	await page
		.getByRole("button", { name: "Choose folder", exact: true })
		.click();
	await page.getByRole("button", { name: "Options for Iceland 2025" }).click();
	await page.getByRole("menuitem", { name: "Rename" }).click();
	const input = page.getByRole("textbox", { name: "Folder label" });
	await input.fill("Holiday");
	await userEvent.keyboard("{Enter}");
	await expect
		.element(page.getByRole("button", { name: "Holiday", exact: true }))
		.toBeVisible();
	const options = page.getByRole("button", { name: "Options for Holiday" });
	await options.click();
	await userEvent.keyboard("{Escape}");
	await expect.element(options).toHaveFocus();
	await expect
		.element(page.getByRole("button", { name: "Holiday", exact: true }))
		.toHaveAttribute("title", "Holiday\n/Photos/Iceland 2025");
	await options.click();
	await page.getByRole("menuitem", { name: "Remove" }).click();
	await expect
		.element(page.getByRole("heading", { name: "Select a folder" }))
		.toBeVisible();
	expect(service.getSavedFolders().entries).toEqual([]);
});

it("truncates a long saved-folder label without displacing its menu", async () => {
	await page.viewport(1200, 800);
	const longLabel =
		"A very long folder label that must stay inside the navigation sidebar";
	const snapshot = {
		...emptySavedFolders(),
		entries: [
			{
				id: "saved-long",
				folderId: "folder-long",
				name: "Original",
				displayPath: "/Photos/Original",
				customLabel: longLabel,
			},
		],
		access: {
			"folder-long": {
				folderId: "folder-long",
				state: "available" as const,
				generation: 1,
				retryAfterMs: 0,
			},
		},
	};
	await render(
		<div style={{ display: "grid", height: 400, width: 288 }}>
			<NavigationRail
				savedFolders={snapshot}
				onChooseFolder={() => {}}
				chooseFolderAvailable
				folderSelection="native"
			/>
		</div>,
	);

	const folderButton = page
		.getByRole("button", { name: longLabel, exact: true })
		.element();
	const label = folderButton.querySelector<HTMLElement>("span");
	const menuButton = page
		.getByRole("button", { name: `Options for ${longLabel}` })
		.element();
	const row = folderButton.closest("li");
	const rail = page.getByRole("navigation", { name: "Sources" }).element();
	if (!label || !row) throw new Error("Saved folder row was not rendered");

	const labelStyle = getComputedStyle(label);
	expect(label.scrollWidth).toBeGreaterThan(label.clientWidth);
	expect(labelStyle.textOverflow).toBe("ellipsis");
	expect(labelStyle.whiteSpace).toBe("nowrap");
	expect(folderButton.getBoundingClientRect().right).toBeLessThanOrEqual(
		menuButton.getBoundingClientRect().left,
	);
	expect(row.getBoundingClientRect().right).toBeLessThanOrEqual(
		rail.getBoundingClientRect().right,
	);
});

it("keeps the folder header and last-row menu visible in a long sidebar", async () => {
	await page.viewport(1200, 800);
	const entries = Array.from({ length: 20 }, (_, i) => ({
		id: `saved-${i}`,
		folderId: `folder-${i}`,
		name: `Album ${i + 1}`,
		displayPath: `/Photos/Album ${i + 1}`,
		customLabel: null,
	}));
	const snapshot = {
		...emptySavedFolders(),
		entries,
		access: Object.fromEntries(
			entries.map((e) => [
				e.folderId,
				{
					folderId: e.folderId,
					state: "available" as const,
					generation: 1,
					retryAfterMs: 0,
				},
			]),
		),
	};
	await render(
		<div style={{ display: "grid", height: 400, width: 288 }}>
			<NavigationRail
				savedFolders={snapshot}
				onChooseFolder={() => {}}
				chooseFolderAvailable
				folderSelection="native"
			/>
		</div>,
	);
	const last = page.getByRole("button", { name: "Options for Album 20" });
	await last.click();
	const popup = page.getByRole("menu").element().getBoundingClientRect();
	const rail = page
		.getByRole("navigation", { name: "Sources" })
		.element()
		.getBoundingClientRect();
	expect(popup.bottom).toBeLessThanOrEqual(rail.bottom);
	expect(popup.top).toBeGreaterThanOrEqual(rail.top);
	const add = page
		.getByRole("button", { name: "Add folder" })
		.element()
		.getBoundingClientRect();
	expect(add.top).toBeGreaterThanOrEqual(rail.top);
	await userEvent.keyboard("{Escape}");
	await expect.element(last).toHaveFocus();
});
