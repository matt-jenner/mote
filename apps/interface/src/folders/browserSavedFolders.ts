import {
	emptySavedFolders,
	normalizeFolderLabel,
	type SavedFolder,
	type SavedFolderSnapshot,
	sortSavedFolders,
} from "./savedFolders";

export function createBrowserSavedFolders(
	local: Storage,
	session: Storage,
	rootId: string,
) {
	const prefix = `mote.folders.v1.${encodeURIComponent(rootId)}.`;
	const entryPrefix = `${prefix}entry.`;
	const cached = new Map<string, SavedFolder>();
	const overlay = new Map<string, SavedFolder | null>();
	let persistenceError: string | null = null;
	let history = false;
	const fail = () => {
		persistenceError =
			"Folder changes will last only while this tab is open. Browser storage is unavailable.";
	};
	const get = (storage: Storage, key: string) => {
		try {
			return storage.getItem(key);
		} catch {
			fail();
			return null;
		}
	};
	const write = (storage: Storage, key: string, value: string | null) => {
		try {
			if (value === null) storage.removeItem(key);
			else storage.setItem(key, value);
			return true;
		} catch {
			fail();
			return false;
		}
	};
	let active =
		get(session, `${prefix}active`) ?? get(local, `${prefix}last`) ?? "";
	function entries(): SavedFolder[] {
		const result = new Map<string, SavedFolder>();
		try {
			for (let i = 0; i < local.length; i++) {
				const key = local.key(i);
				if (!key?.startsWith(entryPrefix)) continue;
				const raw = local.getItem(key);
				try {
					const value = JSON.parse(raw ?? "null");
					if (
						value &&
						typeof value.id === "string" &&
						typeof value.folderId === "string" &&
						typeof value.name === "string" &&
						typeof value.displayPath === "string" &&
						(value.customLabel === null ||
							typeof value.customLabel === "string") &&
						key === entryPrefix + encodeURIComponent(value.folderId)
					) {
						result.set(value.folderId, {
							id: value.id,
							folderId: value.folderId,
							name: value.name,
							displayPath: value.displayPath,
							customLabel: normalizeFolderLabel(value.customLabel),
						});
					}
				} catch {
					/* Ignore a malformed record without losing other shortcuts. */
				}
			}
		} catch {
			fail();
			for (const [key, value] of cached) result.set(key, value);
		}
		for (const [key, value] of overlay) {
			if (value) result.set(key, value);
			else result.delete(key);
		}
		cached.clear();
		for (const [key, value] of result) cached.set(key, value);
		return sortSavedFolders([...result.values()]);
	}
	function persist(folderId: string, value: SavedFolder | null) {
		if (value) cached.set(folderId, value);
		else cached.delete(folderId);
		if (
			write(
				local,
				entryPrefix + encodeURIComponent(folderId),
				value && JSON.stringify(value),
			)
		)
			overlay.delete(folderId);
		else overlay.set(folderId, value);
	}
	function select(id: string | null) {
		const previous = active;
		active = id ?? "";
		write(session, `${prefix}active`, active);
		if (id !== null || get(local, `${prefix}last`) === previous)
			write(local, `${prefix}last`, active);
	}
	function read(): SavedFolderSnapshot {
		const list = entries();
		if (active && !list.some((e) => e.id === active)) {
			active = "";
			write(session, `${prefix}active`, "");
		}
		return {
			...emptySavedFolders(),
			entries: list,
			activeEntryId: active || null,
			hasOpenedFolder: history || get(local, `${prefix}opened`) === "1",
			persistenceError,
		};
	}
	return {
		prefix,
		read,
		select,
		migrated: () => get(local, `${prefix}migrated`) === "1",
		markMigrated: () => {
			write(local, `${prefix}migrated`, "1");
		},
		save(
			folder: Pick<SavedFolder, "folderId" | "name" | "displayPath">,
		): SavedFolder {
			const existing = entries().find((e) => e.folderId === folder.folderId);
			const entry = {
				...folder,
				id: existing?.id ?? globalThis.crypto.randomUUID(),
				customLabel: existing?.customLabel ?? null,
			};
			persist(entry.folderId, entry);
			history = true;
			write(local, `${prefix}opened`, "1");
			return entry;
		},
		rename(id: string, label: string | null) {
			const entry = entries().find((e) => e.id === id);
			if (entry)
				persist(entry.folderId, {
					...entry,
					customLabel: normalizeFolderLabel(label),
				});
		},
		remove(id: string) {
			const entry = entries().find((e) => e.id === id);
			if (!entry) return;
			persist(entry.folderId, null);
			if (active === id) select(null);
			else if (get(local, `${prefix}last`) === id)
				write(local, `${prefix}last`, "");
		},
	};
}
