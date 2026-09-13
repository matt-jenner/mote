import {
	addPickReference,
	clearPickReferences,
	type PickItem,
	type PickListSnapshot,
	type PickReference,
	removePickReference,
	restoreClearedPickReferences,
} from "./pickList";

export interface BrowserPicks {
	read(): PickListSnapshot;
	subscribe(listener: (snapshot: PickListSnapshot) => void): () => void;
	add(reference: PickReference): PickListSnapshot;
	remove(assetId: string): PickListSnapshot;
	clear(): PickReference[];
	restore(cleared: readonly PickReference[]): PickListSnapshot;
	handleStorageEvent(event: StorageEvent): void;
}

interface StoredPicks {
	revision: number;
	items: PickReference[];
}

const persistenceFailure =
	"Picks will remain available in this tab, but browser storage is unavailable.";

function cloneReference(reference: PickReference): PickReference {
	return {
		assetId: reference.assetId,
		sourceFolderId: reference.sourceFolderId,
		sourceLabel: reference.sourceLabel,
	};
}

function cloneReferences(
	references: readonly PickReference[],
): PickReference[] {
	return references.map(cloneReference);
}

function toSnapshot(
	stored: StoredPicks,
	persistenceError: string | null,
): PickListSnapshot {
	const items: PickItem[] = stored.items.map((reference) => ({
		...cloneReference(reference),
		asset: null,
	}));
	return { revision: stored.revision, items, persistenceError };
}

function sameReferences(
	left: readonly PickReference[],
	right: readonly PickReference[],
): boolean {
	return (
		left.length === right.length &&
		left.every((reference, index) => {
			const other = right[index];
			return (
				other !== undefined &&
				reference.assetId === other.assetId &&
				reference.sourceFolderId === other.sourceFolderId &&
				reference.sourceLabel === other.sourceLabel
			);
		})
	);
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPickReference(value: unknown): value is PickReference {
	if (!isRecord(value)) return false;
	if (
		Object.keys(value).some(
			(key) =>
				key !== "assetId" && key !== "sourceFolderId" && key !== "sourceLabel",
		)
	)
		return false;
	return (
		typeof value.assetId === "string" &&
		typeof value.sourceFolderId === "string" &&
		typeof value.sourceLabel === "string"
	);
}

function decodeStored(value: unknown): StoredPicks {
	if (
		!isRecord(value) ||
		Object.keys(value).some((key) => key !== "revision" && key !== "items") ||
		!Number.isSafeInteger(value.revision) ||
		(value.revision as number) < 0 ||
		!Array.isArray(value.items)
	)
		return { revision: 0, items: [] };
	const seen = new Set<string>();
	const items: PickReference[] = [];
	for (const entry of value.items) {
		if (!isPickReference(entry) || seen.has(entry.assetId)) continue;
		seen.add(entry.assetId);
		items.push(cloneReference(entry));
	}
	return { revision: value.revision as number, items };
}

export function createBrowserPicks(
	localStorage: Storage,
	rootId: string,
): BrowserPicks {
	const key = `mote.picks.v1.${encodeURIComponent(rootId)}`;
	const listeners = new Set<(snapshot: PickListSnapshot) => void>();
	let persistenceError: string | null = null;
	let state: StoredPicks = { revision: 0, items: [] };

	function readStorage(): StoredPicks | null {
		try {
			const raw = localStorage.getItem(key);
			if (raw === null) return { revision: 0, items: [] };
			return decodeStored(JSON.parse(raw));
		} catch {
			persistenceError = persistenceFailure;
			return null;
		}
	}

	function reloadLatest(): void {
		const stored = readStorage();
		if (stored === null) return;
		if (stored.revision >= state.revision) state = stored;
	}

	function emit(): void {
		const snapshot = toSnapshot(state, persistenceError);
		for (const listener of listeners) listener(snapshot);
	}

	function persist(next: StoredPicks): void {
		state = next;
		try {
			localStorage.setItem(
				key,
				JSON.stringify({
					revision: next.revision,
					items: cloneReferences(next.items),
				}),
			);
			persistenceError = null;
		} catch {
			persistenceError = persistenceFailure;
		}
	}

	function mutate(
		change: (references: readonly PickReference[]) => PickReference[],
	): PickListSnapshot {
		reloadLatest();
		const items = change(state.items);
		if (!sameReferences(items, state.items)) {
			persist({ revision: state.revision + 1, items });
			emit();
		}
		return toSnapshot(state, persistenceError);
	}

	reloadLatest();

	return {
		read: () => toSnapshot(state, persistenceError),
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		add: (reference) => mutate((items) => addPickReference(items, reference)),
		remove: (assetId) => mutate((items) => removePickReference(items, assetId)),
		clear() {
			reloadLatest();
			const cleared = cloneReferences(state.items);
			mutate((items) => clearPickReferences(items));
			return cleared;
		},
		restore: (cleared) =>
			mutate((items) => restoreClearedPickReferences(cleared, items)),
		handleStorageEvent(event) {
			if (event.key !== null && event.key !== key) return;
			if (event.storageArea && event.storageArea !== localStorage) return;
			const before = state;
			const stored = readStorage();
			if (stored === null) {
				emit();
				return;
			}
			state = stored;
			if (
				before.revision !== state.revision ||
				!sameReferences(before.items, state.items)
			)
				emit();
		},
	};
}
