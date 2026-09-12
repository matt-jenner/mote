import { describe, expect, it } from "vitest";
import { createBrowserPicks } from "./browserPicks";

class MemoryStorage implements Storage {
	private readonly values = new Map<string, string>();

	get length(): number {
		return this.values.size;
	}

	clear(): void {
		this.values.clear();
	}

	getItem(key: string): string | null {
		return this.values.get(key) ?? null;
	}

	key(index: number): string | null {
		return [...this.values.keys()][index] ?? null;
	}

	removeItem(key: string): void {
		this.values.delete(key);
	}

	setItem(key: string, value: string): void {
		this.values.set(key, value);
	}
}

const reference = (
	assetId: string,
	sourceFolderId = "folder-1",
	sourceLabel = "Family",
) => ({
	assetId,
	sourceFolderId,
	sourceLabel,
});

describe("browser picks", () => {
	it("persists ordered path-free references under the active root only", () => {
		const storage = new MemoryStorage();
		const picks = createBrowserPicks(storage, "root-a");

		const pathBearingReference = {
			...reference("asset-1"),
			sourcePath: "/private/photos/family",
		};
		picks.add(pathBearingReference);
		picks.add(reference("asset-1", "folder-2", "Trips"));
		picks.add(reference("asset-2", "folder-2", "Trips"));
		picks.remove("asset-2");

		expect(
			JSON.parse(storage.getItem("mote.picks.v1.root-a") ?? "null"),
		).toEqual({
			revision: 3,
			items: [reference("asset-1")],
		});
		expect(storage.getItem("mote.picks.v1.root-b")).toBeNull();
		expect(picks.read().items).toEqual([
			{ ...reference("asset-1"), asset: null },
		]);
	});

	it("recovers valid records around malformed stored entries", () => {
		const storage = new MemoryStorage();
		storage.setItem(
			"mote.picks.v1.root-a",
			JSON.stringify({
				revision: 7,
				items: [
					reference("asset-1"),
					{ assetId: "asset-2", sourceFolderId: 42, sourceLabel: "Broken" },
					{ ...reference("asset-3"), sourcePath: "/private/photos" },
				],
			}),
		);

		expect(createBrowserPicks(storage, "root-a").read()).toMatchObject({
			revision: 7,
			items: [{ ...reference("asset-1"), asset: null }],
			persistenceError: null,
		});
	});

	it("reloads same-root state and notifies subscribers for storage events", () => {
		const storage = new MemoryStorage();
		const picks = createBrowserPicks(storage, "root-a");
		const snapshots: string[][] = [];
		picks.subscribe((snapshot) => {
			snapshots.push(snapshot.items.map((item) => item.assetId));
		});
		storage.setItem(
			"mote.picks.v1.root-a",
			JSON.stringify({ revision: 4, items: [reference("asset-2")] }),
		);

		picks.handleStorageEvent({
			key: "mote.picks.v1.root-a",
			storageArea: storage,
		} as unknown as StorageEvent);
		picks.handleStorageEvent({
			key: "mote.picks.v1.root-b",
			storageArea: storage,
		} as unknown as StorageEvent);

		expect(picks.read().items.map((item) => item.assetId)).toEqual(["asset-2"]);
		expect(snapshots).toEqual([["asset-2"]]);
	});

	it("uses the latest stored revision before a mutation and retains an unwritable update in memory", () => {
		const storage = new MemoryStorage();
		const first = createBrowserPicks(storage, "root-a");
		const second = createBrowserPicks(storage, "root-a");
		first.add(reference("asset-1"));
		second.add(reference("asset-2"));
		storage.setItem = () => {
			throw new Error("denied");
		};
		second.add(reference("asset-3"));

		expect(second.read()).toMatchObject({
			revision: 3,
			items: [
				{ ...reference("asset-1"), asset: null },
				{ ...reference("asset-2"), asset: null },
				{ ...reference("asset-3"), asset: null },
			],
		});
		expect(second.read().persistenceError).toMatch(/storage/i);
	});

	it("clears immediately and restores the cleared picks before later selections", () => {
		const storage = new MemoryStorage();
		const picks = createBrowserPicks(storage, "root-a");
		picks.add(reference("asset-1"));
		picks.add(reference("asset-2"));
		const cleared = picks.clear();
		picks.add(reference("asset-3"));
		picks.restore(cleared);

		expect(picks.read().items.map((item) => item.assetId)).toEqual([
			"asset-1",
			"asset-2",
			"asset-3",
		]);
	});
});
