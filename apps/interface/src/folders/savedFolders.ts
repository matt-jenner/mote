export type FolderAccessState =
	| "unknown"
	| "checking"
	| "available"
	| "missing"
	| "unreadable"
	| "rootOffline"
	| "unverified";

export interface SavedFolder {
	id: string;
	folderId: string;
	name: string;
	displayPath: string;
	customLabel: string | null;
}

export interface FolderAccess {
	folderId: string;
	state: FolderAccessState;
	generation: number;
	retryAfterMs: number;
}

export interface SavedFolderSnapshot {
	revision?: number;
	entries: SavedFolder[];
	access: Record<string, FolderAccess>;
	activeEntryId: string | null;
	hasOpenedFolder: boolean;
	persistenceError: string | null;
}

export const emptySavedFolders = (): SavedFolderSnapshot => ({
	entries: [],
	access: {},
	activeEntryId: null,
	hasOpenedFolder: false,
	persistenceError: null,
});

export const folderLabel = (entry: SavedFolder): string =>
	entry.customLabel ?? entry.name;
export const normalizeFolderLabel = (value: string | null): string | null =>
	value?.trim() || null;
const compare = (left: string, right: string) =>
	left < right ? -1 : left > right ? 1 : 0;

// Match desktop restoration independently of the host's locale. Labels use
// lowercase Unicode scalar order with numeric ASCII runs; path and ID ties
// use raw UTF-16 string order.
function compareLabels(left: string, right: string): number {
	const a = left.toLowerCase().match(/[0-9]+|[^0-9]/gu) ?? [];
	const b = right.toLowerCase().match(/[0-9]+|[^0-9]/gu) ?? [];
	for (let i = 0; i < Math.min(a.length, b.length); i++) {
		const x = a[i] ?? "";
		const y = b[i] ?? "";
		if (/^[0-9]/u.test(x) && /^[0-9]/u.test(y)) {
			const nx = x.replace(/^0+/u, "");
			const ny = y.replace(/^0+/u, "");
			const order = nx.length - ny.length || compare(nx, ny);
			if (order) return order;
		} else {
			const order = (x.codePointAt(0) ?? 0) - (y.codePointAt(0) ?? 0);
			if (order) return order;
		}
	}
	return a.length - b.length;
}
export function sortSavedFolders(
	entries: readonly SavedFolder[],
): SavedFolder[] {
	return [...entries].sort(
		(left, right) =>
			compareLabels(folderLabel(left), folderLabel(right)) ||
			compare(left.displayPath, right.displayPath) ||
			compare(left.id, right.id),
	);
}

export const cloneSavedFolders = (
	snapshot: SavedFolderSnapshot,
): SavedFolderSnapshot => ({
	...snapshot,
	entries: snapshot.entries.map((entry) => ({ ...entry })),
	access: Object.fromEntries(
		Object.entries(snapshot.access).map(([key, value]) => [key, { ...value }]),
	),
});

export function activeFolderAccess(
	snapshot: SavedFolderSnapshot,
): FolderAccessState {
	const entry = snapshot.entries.find(
		(item) => item.id === snapshot.activeEntryId,
	);
	return entry
		? (snapshot.access[entry.folderId]?.state ?? "unknown")
		: "unknown";
}

export const sourceIsUnavailable = (state: FolderAccessState): boolean =>
	state === "missing" ||
	state === "unreadable" ||
	state === "rootOffline" ||
	state === "unverified";
