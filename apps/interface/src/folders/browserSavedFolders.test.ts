import { describe, expect, it } from "vitest";
import { createBrowserSavedFolders } from "./browserSavedFolders";

class MemoryStorage implements Storage {
	values = new Map<string, string>();
	get length() {
		return this.values.size;
	}
	clear() {
		this.values.clear();
	}
	getItem(key: string) {
		return this.values.get(key) ?? null;
	}
	key(index: number) {
		return [...this.values.keys()][index] ?? null;
	}
	removeItem(key: string) {
		this.values.delete(key);
	}
	setItem(key: string, value: string) {
		this.values.set(key, value);
	}
}
const folder = (id: string) => ({ folderId: id, name: id, displayPath: id });
describe("browser saved folders", () => {
	it("merges independent tab additions and keeps selection per tab", () => {
		const local = new MemoryStorage();
		const a = createBrowserSavedFolders(local, new MemoryStorage(), "root");
		const b = createBrowserSavedFolders(local, new MemoryStorage(), "root");
		const one = a.save(folder("one"));
		a.select(one.id);
		const two = b.save(folder("two"));
		b.select(two.id);
		expect(a.read().entries).toHaveLength(2);
		expect(a.read().activeEntryId).toBe(one.id);
		expect(b.read().activeEntryId).toBe(two.id);
		b.remove(one.id);
		expect(a.read().activeEntryId).toBeNull();
		expect(
			createBrowserSavedFolders(
				local,
				new MemoryStorage(),
				"another-root",
			).read().entries,
		).toEqual([]);
	});
	it("reuses labels and leaves a removed selection empty after reload", () => {
		const local = new MemoryStorage();
		const session = new MemoryStorage();
		const a = createBrowserSavedFolders(local, session, "root");
		const one = a.save(folder("one"));
		a.rename(one.id, " Holiday ");
		a.select(one.id);
		expect(a.save(folder("one")).customLabel).toBe("Holiday");
		a.remove(one.id);
		const reopened = createBrowserSavedFolders(local, session, "root").read();
		expect(reopened.activeEntryId).toBeNull();
		expect(reopened.hasOpenedFolder).toBe(true);
	});
	it("keeps edits in memory and reports denied persistence", () => {
		const local = new MemoryStorage();
		local.setItem = () => {
			throw Error("denied");
		};
		const a = createBrowserSavedFolders(local, new MemoryStorage(), "root");
		const one = a.save(folder("one"));
		a.rename(one.id, "Holiday");
		expect(a.read().entries[0]?.customLabel).toBe("Holiday");
		expect(a.read().persistenceError).toBeTruthy();
	});
});

it("retains successfully stored entries if storage later becomes unreadable", () => {
	const local = new MemoryStorage();
	const store = createBrowserSavedFolders(local, new MemoryStorage(), "root");
	const entry = store.save(folder("one"));
	store.select(entry.id);
	local.getItem = () => {
		throw Error("denied");
	};
	expect(store.read().entries).toHaveLength(1);
	expect(store.read().persistenceError).toBeTruthy();
});

it("keeps another tab's last-opened folder when removing this tab's active entry", () => {
	const local = new MemoryStorage();
	const a = createBrowserSavedFolders(local, new MemoryStorage(), "root");
	const b = createBrowserSavedFolders(local, new MemoryStorage(), "root");
	const one = a.save(folder("one"));
	a.select(one.id);
	const two = b.save(folder("two"));
	b.select(two.id);
	a.remove(one.id);
	expect(
		createBrowserSavedFolders(local, new MemoryStorage(), "root").read()
			.activeEntryId,
	).toBe(two.id);
});
