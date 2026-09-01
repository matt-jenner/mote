import axe from "axe-core";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type {
	ChooseFolderResult,
	FolderBreadcrumb,
	FolderListing,
	PhotoService,
} from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { HostedFolderBrowser } from "./HostedFolderBrowser";

interface Gate<T> {
	promise: Promise<T>;
	resolve(value: T): void;
	reject(reason?: unknown): void;
}

function gate<T>(): Gate<T> {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((nextResolve, nextReject) => {
		resolve = nextResolve;
		reject = nextReject;
	});
	return { promise, resolve, reject };
}

const rootListing: FolderListing = {
	path: "",
	breadcrumbs: [],
	children: [{ name: "Trips", path: "Trips" }],
	imageCount: 0,
};

function folderService(overrides: Partial<PhotoService> = {}): PhotoService {
	const memory = createInMemoryPhotoService({ cancelFolderPicker: true });
	return {
		...memory,
		capabilities: {
			chooseFolder: true,
			folderSelection: "hosted",
			locateFolder: false,
		},
		listFolders: async () => structuredClone(rootListing),
		selectFolder: async () => ({ kind: "cancelled" }),
		...overrides,
	};
}

function installMediaMatches(matches: Partial<Record<string, boolean>>) {
	const original = window.matchMedia;
	window.matchMedia = ((query: string) => {
		const list = original.call(window, query);
		if (!(query in matches)) return list;
		return new Proxy(list, {
			get(target, property, _receiver) {
				if (property === "matches") return matches[query];
				const value = Reflect.get(target, property, target);
				return typeof value === "function" ? value.bind(target) : value;
			},
		});
	}) as typeof window.matchMedia;
	return () => {
		window.matchMedia = original;
	};
}

function seriousViolations(result: axe.AxeResults) {
	return result.violations.filter(
		(violation) =>
			violation.impact === "serious" || violation.impact === "critical",
	);
}

function FocusHarness({ service }: { service: PhotoService }) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<button onClick={() => setOpen(true)} type="button">
				Folders
			</button>
			{open ? (
				<HostedFolderBrowser
					initialBreadcrumbs={[]}
					onClose={() => setOpen(false)}
					onSelected={() => undefined}
					service={service}
				/>
			) : null}
		</>
	);
}

describe("hosted folder browser", () => {
	beforeEach(async () => {
		await page.viewport(1440, 1024);
	});

	afterEach(() => {
		for (const property of [
			"--safe-area-top",
			"--safe-area-right",
			"--safe-area-bottom",
			"--safe-area-left",
		]) {
			document.documentElement.style.removeProperty(property);
		}
	});

	it("renders a centred desktop modal with contained path navigation", async () => {
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={folderService()}
			/>,
		);

		const dialog = screen.getByRole("dialog", { name: "Choose a folder" });
		await expect.element(dialog).toBeVisible();
		await expect
			.element(screen.getByRole("navigation", { name: "Folder path" }))
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Back" }))
			.toBeDisabled();
		await expect
			.element(screen.getByRole("button", { name: "Trips" }))
			.toBeVisible();
		await expect
			.element(screen.getByRole("button", { name: "Open this folder" }))
			.toBeEnabled();

		const bounds = dialog.element().getBoundingClientRect();
		expect(bounds.width).toBeLessThanOrEqual(640);
		expect(bounds.height).toBeLessThanOrEqual(1024 * 0.7 + 1);
		expect(bounds.left).toBeCloseTo((1440 - bounds.width) / 2, 0);
		expect(bounds.top).toBeCloseTo((1024 - bounds.height) / 2, 0);
	});

	it("shows the direct image count when a folder has no child folders", async () => {
		const service = folderService({
			listFolders: async (path) =>
				path === "Trips"
					? {
							path,
							breadcrumbs: [{ name: "Trips", path }],
							children: [],
							imageCount: 27,
						}
					: structuredClone(rootListing),
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);

		await screen.getByRole("button", { name: "Trips" }).click();

		await expect
			.element(screen.getByText("27 photos in this folder"))
			.toBeVisible();
		await expect.element(screen.getByText("No subfolders")).toBeVisible();
	});

	it("uses a safe-area bounded full-height sheet on a phone", async () => {
		await page.viewport(390, 844);
		document.documentElement.style.setProperty("--safe-area-top", "20px");
		document.documentElement.style.setProperty("--safe-area-right", "10px");
		document.documentElement.style.setProperty("--safe-area-bottom", "16px");
		document.documentElement.style.setProperty("--safe-area-left", "8px");
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={folderService()}
			/>,
		);

		const bounds = screen
			.getByRole("dialog", { name: "Choose a folder" })
			.element()
			.getBoundingClientRect();
		expect(bounds.top).toBe(20);
		expect(bounds.right).toBe(380);
		expect(bounds.bottom).toBeCloseTo(828, 0);
		expect(bounds.left).toBe(8);
	});

	it("uses the sheet layout for a coarse pointer at a wider width", async () => {
		await page.viewport(834, 1194);
		const restoreMedia = installMediaMatches({ "(pointer: coarse)": true });
		try {
			const screen = await render(
				<HostedFolderBrowser
					initialBreadcrumbs={[]}
					onClose={() => undefined}
					onSelected={() => undefined}
					service={folderService()}
				/>,
			);

			const bounds = screen
				.getByRole("dialog", { name: "Choose a folder" })
				.element()
				.getBoundingClientRect();
			expect(bounds.width).toBeCloseTo(834, 0);
			expect(bounds.height).toBeCloseTo(1194, 0);
		} finally {
			restoreMedia();
		}
	});

	it("contains keyboard focus and restores the Folders trigger on Escape", async () => {
		const screen = await render(<FocusHarness service={folderService()} />);
		const trigger = screen.getByRole("button", { name: "Folders" });
		trigger.element().focus();
		await userEvent.keyboard("{Enter}");
		const close = screen.getByRole("button", { name: "Close folder browser" });
		await expect.poll(() => document.activeElement).toBe(close.element());

		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(
			screen.getByRole("button", { name: "Open this folder" }).element(),
		);
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(close.element());

		await userEvent.keyboard("{Escape}");
		expect(
			screen.getByRole("dialog", { name: "Choose a folder" }).query(),
		).toBeNull();
		expect(document.activeElement).toBe(trigger.element());
	});

	it("keeps the last good listing visible when child loading fails and retries", async () => {
		const childRequest = gate<FolderListing>();
		const retryRequest = gate<FolderListing>();
		let childAttempts = 0;
		const service = folderService({
			listFolders: async (path) => {
				if (path === "") return structuredClone(rootListing);
				childAttempts += 1;
				return childAttempts === 1
					? childRequest.promise
					: retryRequest.promise;
			},
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);
		await screen.getByRole("button", { name: "Trips" }).click();
		const dialog = screen.getByRole("dialog", { name: "Choose a folder" });
		expect(dialog.element().getAttribute("aria-busy")).toBe("true");
		await expect
			.element(screen.getByRole("button", { name: "Open this folder" }))
			.toBeDisabled();

		childRequest.reject(new Error("Folder is temporarily unavailable"));
		await expect
			.element(screen.getByRole("alert"))
			.toHaveTextContent("Folder is temporarily unavailable");
		await expect
			.element(screen.getByRole("button", { name: "Trips" }))
			.toBeVisible();

		await screen.getByRole("button", { name: "Retry" }).click();
		expect(dialog.element().getAttribute("aria-busy")).toBe("true");
		retryRequest.resolve({
			path: "Trips",
			breadcrumbs: [{ name: "Trips", path: "Trips" }],
			children: [{ name: "Iceland", path: "Trips/Iceland" }],
			imageCount: 0,
		});
		await expect
			.element(screen.getByRole("button", { name: "Iceland" }))
			.toBeVisible();
	});

	it("recovers through returned breadcrumb paths deepest first", async () => {
		const calls: string[] = [];
		const service = folderService({
			listFolders: async (path) => {
				calls.push(path);
				if (path === "opaque/processed") throw new Error("Stale folder");
				if (path === "opaque/iceland") {
					return {
						path,
						breadcrumbs: [
							{ name: "Trips", path: "opaque/trips" },
							{ name: "Iceland", path },
						],
						children: [],
						imageCount: 0,
					};
				}
				throw new Error(`Unexpected recovery path: ${path}`);
			},
		});
		const initialBreadcrumbs: FolderBreadcrumb[] = [
			{ name: "Trips", path: "opaque/trips" },
			{ name: "Iceland", path: "opaque/iceland" },
			{ name: "Processed", path: "opaque/processed" },
		];
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={initialBreadcrumbs}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);

		await expect.element(screen.getByText("Iceland")).toBeVisible();
		expect(calls).toEqual(["opaque/processed", "opaque/iceland"]);
		expect(document.body.textContent).not.toContain("/photos");
	});

	it("falls back to the mounted root when every saved breadcrumb is stale", async () => {
		const calls: string[] = [];
		const service = folderService({
			listFolders: async (path) => {
				calls.push(path);
				if (path !== "") throw new Error("Stale folder");
				return structuredClone(rootListing);
			},
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[
					{ name: "Trips", path: "saved-trips" },
					{ name: "Iceland", path: "saved-iceland" },
				]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);

		await expect
			.element(screen.getByRole("button", { name: "Trips" }))
			.toBeVisible();
		expect(calls).toEqual(["saved-iceland", "saved-trips", ""]);
	});

	it("does not issue more recovery requests after unmount", async () => {
		const pending = gate<FolderListing>();
		const calls: string[] = [];
		const service = folderService({
			listFolders: async (path) => {
				calls.push(path);
				if (path === "saved-iceland") return pending.promise;
				throw new Error("A later recovery request should not run");
			},
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[
					{ name: "Trips", path: "saved-trips" },
					{ name: "Iceland", path: "saved-iceland" },
				]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);
		await expect.poll(() => calls).toEqual(["saved-iceland"]);

		screen.unmount();
		pending.reject(new Error("Unmounted"));
		await new Promise((resolve) => window.setTimeout(resolve, 0));

		expect(calls).toEqual(["saved-iceland"]);
	});

	it("derives Back from the server-returned breadcrumbs", async () => {
		const calls: string[] = [];
		const service = folderService({
			listFolders: async (path) => {
				calls.push(path);
				if (path === "opaque/current") {
					return {
						path,
						breadcrumbs: [
							{ name: "Parent", path: "server-returned-parent" },
							{ name: "Current", path },
						],
						children: [],
						imageCount: 0,
					};
				}
				return {
					path,
					breadcrumbs: [{ name: "Parent", path }],
					children: [],
					imageCount: 0,
				};
			},
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[{ name: "Current", path: "opaque/current" }]}
				onClose={() => undefined}
				onSelected={() => undefined}
				service={service}
			/>,
		);
		await screen.getByRole("button", { name: "Back" }).click();
		await expect
			.poll(() => calls)
			.toEqual(["opaque/current", "server-returned-parent"]);
	});

	it("stays open on cancellation and reports only a selected folder", async () => {
		const onSelected = vi.fn<(result: ChooseFolderResult) => void>();
		let attempts = 0;
		const selected: ChooseFolderResult = {
			kind: "selected",
			state: {
				settings: {
					appearance: "system",
					galleryScope: "includeSubfolders",
				},
				activeSource: {
					id: "source-trips",
					selectionId: "selection-trips",
					displayName: "Trips",
					availability: "available",
				},
			},
		};
		const service = folderService({
			selectFolder: async () => {
				attempts += 1;
				return attempts === 1 ? { kind: "cancelled" } : selected;
			},
		});
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={() => undefined}
				onSelected={onSelected}
				service={service}
			/>,
		);
		const open = screen.getByRole("button", { name: "Open this folder" });
		await open.click();
		expect(onSelected).not.toHaveBeenCalled();
		await expect
			.element(screen.getByRole("dialog", { name: "Choose a folder" }))
			.toBeVisible();

		await open.click();
		expect(onSelected).toHaveBeenCalledWith(selected);
	});

	it("blocks Close while a submitted selection commits and delivers the result", async () => {
		const selection = gate<ChooseFolderResult>();
		const onClose = vi.fn<() => void>();
		const onSelected = vi.fn<(result: ChooseFolderResult) => void>();
		const selected: ChooseFolderResult = {
			kind: "selected",
			state: {
				settings: {
					appearance: "system",
					galleryScope: "includeSubfolders",
				},
				activeSource: {
					id: "source-trips",
					selectionId: "selection-trips",
					displayName: "Trips",
					availability: "available",
				},
			},
		};
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={onClose}
				onSelected={onSelected}
				service={folderService({
					selectFolder: async () => selection.promise,
				})}
			/>,
		);
		await screen.getByRole("button", { name: "Open this folder" }).click();
		const dialog = screen.getByRole("dialog", { name: "Choose a folder" });
		const close = screen.getByRole("button", { name: "Close folder browser" });
		expect(dialog.element().getAttribute("aria-busy")).toBe("true");
		await expect.element(close).toBeDisabled();
		(close.element() as HTMLButtonElement).click();
		expect(onClose).not.toHaveBeenCalled();

		selection.resolve(selected);
		await expect.poll(() => onSelected.mock.calls).toEqual([[selected]]);
		expect(onClose).not.toHaveBeenCalled();
	});

	it("blocks Escape while selection is pending and restores interaction on cancellation", async () => {
		const selection = gate<ChooseFolderResult>();
		const onClose = vi.fn<() => void>();
		const screen = await render(
			<HostedFolderBrowser
				initialBreadcrumbs={[]}
				onClose={onClose}
				onSelected={() => undefined}
				service={folderService({
					selectFolder: async () => selection.promise,
				})}
			/>,
		);
		const open = screen.getByRole("button", { name: "Open this folder" });
		await open.click();
		screen
			.getByRole("dialog", { name: "Choose a folder" })
			.element()
			.dispatchEvent(
				new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }),
			);
		expect(onClose).not.toHaveBeenCalled();

		selection.resolve({ kind: "cancelled" });
		await expect.element(open).toBeEnabled();
		await expect
			.element(screen.getByRole("button", { name: "Close folder browser" }))
			.toBeEnabled();
		await expect.element(open).toHaveFocus();
	});

	it("suppresses its transition in reduced-motion mode and passes axe", async () => {
		const restoreMedia = installMediaMatches({
			"(prefers-reduced-motion: reduce)": true,
		});
		try {
			const screen = await render(
				<HostedFolderBrowser
					initialBreadcrumbs={[]}
					onClose={() => undefined}
					onSelected={() => undefined}
					service={folderService()}
				/>,
			);
			const dialog = screen.getByRole("dialog", { name: "Choose a folder" });
			await expect.element(dialog).toBeVisible();
			expect(getComputedStyle(dialog.element()).transitionDuration).toBe("0s");
			expect(seriousViolations(await axe.run(document))).toEqual([]);
		} finally {
			restoreMedia();
		}
	});
});
