import { describe, expect, it } from "vitest";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import {
	folderLabel,
	normalizeFolderLabel,
	type SavedFolder,
	sortSavedFolders,
} from "./savedFolders";
import { createSelectionIntent } from "./selectionIntent";

const entry = (
	id: string,
	name: string,
	customLabel: string | null = null,
): SavedFolder => ({
	id,
	folderId: id,
	name,
	displayPath: `/Photos/${id}`,
	customLabel,
});

describe("saved folder labels", () => {
	it("matches desktop ordering for duplicate and non-ASCII labels", () => {
		const rows = (
			[
				["two", "Photos", "/photos/2"],
				["ten", "Photos", "/photos/10"],
				["accent10", "éclair 10", "/photos/e10"],
				["accent2", "Éclair 2", "/photos/e2"],
				["zebra", "Zebra", "/photos/z"],
			] as const
		).map(([id, name, displayPath]) => ({ ...entry(id, name), displayPath }));
		expect(sortSavedFolders(rows).map((row) => row.id)).toEqual([
			"ten",
			"two",
			"zebra",
			"accent2",
			"accent10",
		]);
	});
	it("sorts custom or default labels naturally and leaves caller order intact", () => {
		const rows = [
			entry("a", "Z", "album 10"),
			entry("b", "Album 2"),
			entry("c", "Coast"),
		];
		expect(sortSavedFolders(rows).map((row) => row.id)).toEqual([
			"b",
			"a",
			"c",
		]);
		expect(rows.map((row) => row.id)).toEqual(["a", "b", "c"]);
	});
	it("resets whitespace labels to the folder name and deterministically breaks equal-label ties", () => {
		expect(normalizeFolderLabel("  ")).toBeNull();
		expect(normalizeFolderLabel("  Summer  ")).toBe("Summer");
		expect(folderLabel(entry("x", "Original"))).toBe("Original");
		expect(
			sortSavedFolders([entry("z", "Family"), entry("a", "family")]).map(
				(row) => row.id,
			),
		).toEqual(["a", "z"]);
	});
	it("invalidates both superseded and explicitly cleared selection intents", () => {
		const intent = createSelectionIntent();
		const old = intent.begin();
		const current = intent.begin();
		expect(intent.isCurrent(old)).toBe(false);
		expect(intent.isCurrent(current)).toBe(true);
		intent.invalidate();
		expect(intent.isCurrent(current)).toBe(false);
	});
});

describe("memory saved-folder contract", () => {
	it("does not publish a removed folder's delayed scan into a new selection", async () => {
		const service = createInMemoryPhotoService({
			selectedFolderName: "Family",
			geometryDelayMs: 20,
		});
		await service.chooseFolder();
		const oldSelection = (await service.getBootstrapState()).activeSource
			?.selectionId;
		const updates: string[] = [];
		service.watchWallUpdates((update) => updates.push(update.selectionId));
		const scan = service.startFixtureScan();
		const saved = service.getSavedFolders().entries[0];
		if (!saved) throw new Error("Expected saved folder");
		await service.removeSavedFolder(saved.id);
		await service.chooseFolder();
		expect(
			(await service.getBootstrapState()).activeSource?.selectionId,
		).not.toBe(oldSelection);
		await scan;
		expect(updates).toEqual([]);
	});
	it("reuses the chosen folder and its label, then clears an active removed entry", async () => {
		const service = createInMemoryPhotoService({
			selectedFolderName: "Family",
		});
		await service.chooseFolder();
		const first = service.getSavedFolders().entries[0];
		if (!first) throw new Error("Expected saved folder");
		await service.renameSavedFolder(first.id, "Favourites");
		await service.chooseFolder();
		expect(service.getSavedFolders().entries).toHaveLength(1);
		expect((await service.getBootstrapState()).activeSource?.displayName).toBe(
			"Favourites",
		);
		await service.removeSavedFolder(first.id);
		expect(service.getSavedFolders().entries).toHaveLength(0);
		expect((await service.getBootstrapState()).activeSource).toBeNull();
		expect(service.getSavedFolders().hasOpenedFolder).toBe(true);
	});
	it("notifies subscribers with independent snapshots", async () => {
		const service = createInMemoryPhotoService();
		const counts: number[] = [];
		const stop = service.watchSavedFolders((snapshot) => {
			counts.push(snapshot.entries.length);
			snapshot.entries.length = 0;
		});
		await service.chooseFolder();
		expect(counts).toEqual([1]);
		expect(service.getSavedFolders().entries).toHaveLength(1);
		stop();
		await service.clearActiveFolder();
		expect(counts).toEqual([1]);
	});
});
