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
const collator = new Intl.Collator(undefined, {
	numeric: true,
	sensitivity: "base",
});
const compare = (left: string, right: string) =>
	left < right ? -1 : left > right ? 1 : 0;
export function sortSavedFolders(
	entries: readonly SavedFolder[],
): SavedFolder[] {
	return [...entries].sort(
		(left, right) =>
			collator.compare(folderLabel(left), folderLabel(right)) ||
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
