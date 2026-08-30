import { describe, expect, it } from "vitest";
import {
	createBrowserPreferences,
	hostedClientStorageKey,
	hostedPreferencesStorageKey,
} from "./browserPreferences";

class MemoryStorage implements Storage {
	private readonly values = new Map<string, string>();
	readonly writes: Array<[string, string]> = [];

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
		this.writes.push([key, value]);
	}
}

describe("browser preferences", () => {
	it("writes only the versioned browser-owned fields in one selection update", () => {
		const local = new MemoryStorage();
		const preferences = createBrowserPreferences({
			localStorage: local,
			sessionStorage: new MemoryStorage(),
			randomUuid: () => "client-a",
		});

		preferences.update({
			selectionId: "selection-a",
			breadcrumbs: [
				{ name: "Trips", path: "Trips" },
				{ name: "Iceland", path: "Trips/Iceland" },
			],
			appearance: "dark",
			galleryScope: "currentFolder",
			sortDirection: "newestFirst",
		});

		expect(local.writes).toHaveLength(1);
		expect(local.writes[0]?.[0]).toBe(hostedPreferencesStorageKey);
		expect(JSON.parse(local.writes[0]?.[1] ?? "null")).toEqual({
			selectionId: "selection-a",
			breadcrumbs: [
				{ name: "Trips", path: "Trips" },
				{ name: "Iceland", path: "Trips/Iceland" },
			],
			appearance: "dark",
			galleryScope: "currentFolder",
			sortDirection: "newestFirst",
		});
	});

	it("falls back safely for malformed JSON and unknown enum values", () => {
		const malformed = new MemoryStorage();
		malformed.setItem(hostedPreferencesStorageKey, "{not-json");
		const malformedPreferences = createBrowserPreferences({
			localStorage: malformed,
			sessionStorage: new MemoryStorage(),
			randomUuid: () => "client-a",
		});

		expect(malformedPreferences.read()).toEqual({
			selectionId: null,
			breadcrumbs: [],
			appearance: "system",
			galleryScope: "includeSubfolders",
			sortDirection: "oldestFirst",
		});

		const unknown = new MemoryStorage();
		unknown.setItem(
			hostedPreferencesStorageKey,
			JSON.stringify({
				selectionId: "selection-a",
				breadcrumbs: [{ name: "Trips", path: "Trips" }],
				appearance: "sepia",
				galleryScope: "everything",
				sortDirection: "random",
			}),
		);
		const recovered = createBrowserPreferences({
			localStorage: unknown,
			sessionStorage: new MemoryStorage(),
			randomUuid: () => "client-b",
		}).read();

		expect(recovered).toEqual({
			selectionId: "selection-a",
			breadcrumbs: [{ name: "Trips", path: "Trips" }],
			appearance: "system",
			galleryScope: "includeSubfolders",
			sortDirection: "oldestFirst",
		});
	});

	it("reuses one tab client ID and isolates different session stores", () => {
		const local = new MemoryStorage();
		const firstSession = new MemoryStorage();
		const secondSession = new MemoryStorage();
		let nextId = 0;
		const randomUuid = () => `client-${++nextId}`;
		const first = createBrowserPreferences({
			localStorage: local,
			sessionStorage: firstSession,
			randomUuid,
		});
		const sameTab = createBrowserPreferences({
			localStorage: local,
			sessionStorage: firstSession,
			randomUuid,
		});
		const otherTab = createBrowserPreferences({
			localStorage: local,
			sessionStorage: secondSession,
			randomUuid,
		});

		expect(first.clientId()).toBe("client-1");
		expect(sameTab.clientId()).toBe("client-1");
		expect(otherTab.clientId()).toBe("client-2");
		expect(firstSession.getItem(hostedClientStorageKey)).toBe("client-1");
		expect(secondSession.getItem(hostedClientStorageKey)).toBe("client-2");
	});

	it("returns defensive breadcrumb copies", () => {
		const local = new MemoryStorage();
		local.setItem(
			hostedPreferencesStorageKey,
			JSON.stringify({
				selectionId: "selection-a",
				breadcrumbs: [{ name: "Trips", path: "Trips" }],
				appearance: "system",
				galleryScope: "includeSubfolders",
				sortDirection: "oldestFirst",
			}),
		);
		const preferences = createBrowserPreferences({
			localStorage: local,
			sessionStorage: new MemoryStorage(),
			randomUuid: () => "client-a",
		});

		const first = preferences.read();
		const firstBreadcrumb = first.breadcrumbs[0];
		if (!firstBreadcrumb) throw new Error("expected a stored breadcrumb");
		firstBreadcrumb.path = "mutated";

		expect(preferences.read().breadcrumbs).toEqual([
			{ name: "Trips", path: "Trips" },
		]);
	});
});
