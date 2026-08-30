import type {
	Appearance,
	FolderBreadcrumb,
	GalleryScope,
	SortDirection,
} from "./photoService";

export const hostedPreferencesStorageKey = "photo-viewer.hosted.v1";
export const hostedClientStorageKey = "photo-viewer.client.v1";

export interface HostedBrowserPreferences {
	selectionId: string | null;
	breadcrumbs: FolderBreadcrumb[];
	appearance: Appearance;
	galleryScope: GalleryScope;
	sortDirection: SortDirection;
}

export interface BrowserPreferences {
	read(): HostedBrowserPreferences;
	update(values: Partial<HostedBrowserPreferences>): HostedBrowserPreferences;
	clientId(): string;
}

export interface BrowserPreferencesOptions {
	localStorage: Storage;
	sessionStorage: Storage;
	randomUuid: () => string;
}

const defaultPreferences: HostedBrowserPreferences = {
	selectionId: null,
	breadcrumbs: [],
	appearance: "system",
	galleryScope: "includeSubfolders",
	sortDirection: "oldestFirst",
};

const preferenceKeys = new Set([
	"selectionId",
	"breadcrumbs",
	"appearance",
	"galleryScope",
	"sortDirection",
]);

function isAscii(value: string): boolean {
	for (let index = 0; index < value.length; index += 1) {
		if ((value.charCodeAt(index) ?? 128) > 127) return false;
	}
	return true;
}

const cloneBreadcrumbs = (
	breadcrumbs: readonly FolderBreadcrumb[],
): FolderBreadcrumb[] => breadcrumbs.map(({ name, path }) => ({ name, path }));

const clonePreferences = (
	preferences: HostedBrowserPreferences,
): HostedBrowserPreferences => ({
	...preferences,
	breadcrumbs: cloneBreadcrumbs(preferences.breadcrumbs),
});

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function decodeBreadcrumbs(value: unknown): FolderBreadcrumb[] {
	if (!Array.isArray(value)) return [];
	const breadcrumbs: FolderBreadcrumb[] = [];
	for (const entry of value) {
		if (
			!isRecord(entry) ||
			Object.keys(entry).some((key) => key !== "name" && key !== "path") ||
			typeof entry.name !== "string" ||
			typeof entry.path !== "string"
		) {
			return [];
		}
		breadcrumbs.push({ name: entry.name, path: entry.path });
	}
	return breadcrumbs;
}

function decodePreferences(value: unknown): HostedBrowserPreferences {
	if (
		!isRecord(value) ||
		Object.keys(value).some((key) => !preferenceKeys.has(key))
	) {
		return clonePreferences(defaultPreferences);
	}
	const selectionId =
		value.selectionId === null ||
		(typeof value.selectionId === "string" &&
			value.selectionId.length <= 128 &&
			isAscii(value.selectionId))
			? (value.selectionId as string | null)
			: null;
	const appearance: Appearance =
		value.appearance === "light" || value.appearance === "dark"
			? value.appearance
			: "system";
	const galleryScope: GalleryScope =
		value.galleryScope === "currentFolder"
			? "currentFolder"
			: "includeSubfolders";
	const sortDirection: SortDirection =
		value.sortDirection === "newestFirst" ? "newestFirst" : "oldestFirst";
	return {
		selectionId,
		breadcrumbs: decodeBreadcrumbs(value.breadcrumbs),
		appearance,
		galleryScope,
		sortDirection,
	};
}

function readStoredPreferences(storage: Storage): HostedBrowserPreferences {
	const stored = storage.getItem(hostedPreferencesStorageKey);
	if (stored === null) return clonePreferences(defaultPreferences);
	try {
		return decodePreferences(JSON.parse(stored));
	} catch {
		return clonePreferences(defaultPreferences);
	}
}

function validClientId(value: string | null): value is string {
	return (
		value !== null && value.length > 0 && value.length <= 128 && isAscii(value)
	);
}

export function createBrowserPreferences(
	options: BrowserPreferencesOptions,
): BrowserPreferences {
	let preferences = readStoredPreferences(options.localStorage);
	let clientId: string | null = null;
	return {
		read: () => clonePreferences(preferences),
		update(values) {
			preferences = {
				...preferences,
				...values,
				breadcrumbs:
					values.breadcrumbs === undefined
						? cloneBreadcrumbs(preferences.breadcrumbs)
						: cloneBreadcrumbs(values.breadcrumbs),
			};
			options.localStorage.setItem(
				hostedPreferencesStorageKey,
				JSON.stringify(preferences),
			);
			return clonePreferences(preferences);
		},
		clientId() {
			if (clientId !== null) return clientId;
			const stored = options.sessionStorage.getItem(hostedClientStorageKey);
			if (validClientId(stored)) {
				clientId = stored;
				return clientId;
			}
			const generated = options.randomUuid();
			if (!validClientId(generated)) {
				throw new Error("The client ID factory returned an invalid value.");
			}
			clientId = generated;
			options.sessionStorage.setItem(hostedClientStorageKey, clientId);
			return clientId;
		},
	};
}
