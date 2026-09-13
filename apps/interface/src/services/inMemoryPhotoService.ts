import {
	cloneSavedFolders,
	emptySavedFolders,
	folderLabel,
	normalizeFolderLabel,
	type SavedFolderSnapshot,
	sortSavedFolders,
} from "../folders/savedFolders";
import {
	addPickReference,
	clearPickReferences,
	type PickListSnapshot,
	type PickReference,
	removePickReference,
	restoreClearedPickReferences,
} from "../picks/pickList";
import type {
	Appearance,
	BootstrapState,
	DerivativeClass,
	DerivativeReference,
	DerivativeRequest,
	GalleryScope,
	PhotoService,
	ScanProgressDto,
	SortDirection,
	WallAsset,
	WallPage,
	WallQueryRequest,
	WallUpdate,
	WallWarningState,
} from "./photoService";
import { PhotoServiceError } from "./photoService";

export interface InMemoryWallFixture extends WallAsset {
	/** URLs are supplied by the memory-mode host rather than invented by the adapter. */
	derivativeUrls?: Partial<Record<DerivativeClass, string>>;
	wallThumbnailUrl?: string;
	screenPreviewUrl?: string;
}

export interface InMemoryOptions {
	selectedFolderName?: string;
	cancelFolderPicker?: boolean;
	wallAssets?: readonly InMemoryWallFixture[];
	geometryDelayMs?: number;
	thumbnailDelayMs?: number;
	metadataDelayMs?: number;
	sourceWarnings?: readonly WallWarningState[];
}

export interface InMemoryPhotoService extends PhotoService {
	readonly derivativeRequests: DerivativeRequest[];
	readonly interactionCalls: boolean[];
	startFixtureScan(): Promise<void>;
	finishFixtureScan(): Promise<void>;
	emitForTest(update: WallUpdate): void;
}

type WallListener = (update: WallUpdate) => void;

const sourceId = "memory-source";
const maxWallPageSize = 250;

const clone = <T>(value: T): T => structuredClone(value);
const delay = (milliseconds: number): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, Math.max(0, milliseconds)));

export function createInMemoryPhotoService(
	options: InMemoryOptions = {},
): InMemoryPhotoService {
	const fixtures = clone(options.wallAssets ?? []).map((fixture) => ({
		...fixture,
		derivativeUrls: fixture.derivativeUrls
			? { ...fixture.derivativeUrls }
			: undefined,
	}));
	const listeners = new Set<WallListener>();
	const derivativeUrls = new Map<string, string>();
	let settled = false;
	let scanGeneration = 0;
	let scanPromise: Promise<void> | null = null;
	let derivativePhasePromise: Promise<void> | null = null;
	let settlementPromise: Promise<void> | null = null;
	let sourceWarnings: WallWarningState[] = [...(options.sourceWarnings ?? [])];
	const derivativeRequests: DerivativeRequest[] = [];
	const interactionCalls: boolean[] = [];
	let pickReferences: PickReference[] = [];
	let pickRevision = 0;
	const pickListeners = new Set<(snapshot: PickListSnapshot) => void>();
	let sortDirection: SortDirection = "oldestFirst";
	const savedListeners = new Set<(snapshot: SavedFolderSnapshot) => void>();
	let savedSequence = 0;
	let selectionSequence = 1;
	let selectionId = "memory-selection-1";
	const invalidateSelection = () => {
		selectionId = `memory-selection-${++selectionSequence}`;
		scanGeneration += 1;
		scanPromise = null;
		derivativePhasePromise = null;
		settlementPromise = null;
	};
	let state: BootstrapState = {
		settings: { appearance: "system", galleryScope: "includeSubfolders" },
		activeSource: null,
		savedFolders: emptySavedFolders(),
	};
	const publishSaved = () => {
		state.savedFolders.entries = sortSavedFolders(state.savedFolders.entries);
		for (const listener of savedListeners)
			listener(cloneSavedFolders(state.savedFolders));
	};
	let assets: WallAsset[] = fixtures.map(
		({
			derivativeUrls: _urls,
			wallThumbnailUrl: _wallUrl,
			screenPreviewUrl: _screenUrl,
			...asset
		}) => ({
			...asset,
			dateState: "provisional" as const,
			wallThumbnail: null,
			screenPreview: null,
		}),
	);

	for (const fixture of fixtures) {
		for (const kind of ["wallThumbnail", "screenPreview"] as const) {
			const reference = fixture[kind];
			const url =
				fixture.derivativeUrls?.[kind] ??
				(kind === "wallThumbnail"
					? fixture.wallThumbnailUrl
					: fixture.screenPreviewUrl);
			if (reference && url) derivativeUrls.set(`${kind}:${reference.key}`, url);
		}
	}

	const publish = (update: WallUpdate): void => {
		if (update.selectionId !== selectionId) return;
		for (const listener of listeners) listener(clone(update));
	};
	const pickSnapshot = (): PickListSnapshot => ({
		revision: pickRevision,
		items: pickReferences.map((reference) => {
			const asset = assets.find(
				(candidate) => candidate.id === reference.assetId,
			);
			return { ...clone(reference), asset: asset ? clone(asset) : null };
		}),
		persistenceError: null,
	});
	const publishPicks = () => {
		const snapshot = pickSnapshot();
		for (const listener of pickListeners) listener(clone(snapshot));
		return snapshot;
	};
	const mutatePicks = (
		change: (current: readonly PickReference[]) => PickReference[],
	) => {
		const next = change(pickReferences);
		if (JSON.stringify(next) !== JSON.stringify(pickReferences)) {
			pickReferences = next;
			pickRevision += 1;
			return publishPicks();
		}
		return pickSnapshot();
	};
	const progress = (enriched: number): ScanProgressDto => ({
		discovered: fixtures.length,
		shaped: fixtures.length,
		enriched,
		directTotal: fixtures.length,
		total: fixtures.length,
	});
	const derivativeReferences = (): DerivativeReference[] =>
		fixtures.flatMap((fixture) =>
			[fixture.wallThumbnail, fixture.screenPreview].filter(
				(reference): reference is DerivativeReference => reference !== null,
			),
		);

	const service: InMemoryPhotoService = {
		getPicks: () => clone(pickSnapshot()),
		watchPicks(listener) {
			pickListeners.add(listener);
			return () => pickListeners.delete(listener);
		},
		async loadPicks() {
			return clone(pickSnapshot());
		},
		async addPick(reference) {
			return clone(
				mutatePicks((current) => addPickReference(current, reference)),
			);
		},
		async removePick(assetId) {
			return clone(
				mutatePicks((current) => removePickReference(current, assetId)),
			);
		},
		async clearPicks() {
			return clone(mutatePicks((current) => clearPickReferences(current)));
		},
		async restorePicks(cleared) {
			return clone(
				mutatePicks((current) =>
					restoreClearedPickReferences(cleared, current),
				),
			);
		},
		async requestPickDerivatives(request) {
			await service.requestDerivatives(request);
		},
		originalDownloadUrl: () => null,
		async copyPickedOriginals() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"Copying originals is available in the desktop app.",
			);
		},
		async cancelOriginalCopy() {},
		async showLastCopyDestination() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"Opening the copy destination is available in the desktop app.",
			);
		},
		getSavedFolders: () => cloneSavedFolders(state.savedFolders),
		watchSavedFolders(listener) {
			savedListeners.add(listener);
			return () => {
				savedListeners.delete(listener);
			};
		},
		async renameSavedFolder(id, label) {
			const entry = state.savedFolders.entries.find((item) => item.id === id);
			if (
				!entry ||
				state.savedFolders.access[entry.folderId]?.state !== "available"
			)
				throw new PhotoServiceError(
					"folderUnavailable",
					"That folder is unavailable.",
				);
			entry.customLabel = normalizeFolderLabel(label);
			if (state.savedFolders.activeEntryId === id && state.activeSource)
				state.activeSource.displayName = folderLabel(entry);
			publishSaved();
			return clone(state);
		},
		async removeSavedFolder(id) {
			const entry = state.savedFolders.entries.find((item) => item.id === id);
			state.savedFolders.entries = state.savedFolders.entries.filter(
				(item) => item.id !== id,
			);
			if (entry) delete state.savedFolders.access[entry.folderId];
			if (state.savedFolders.activeEntryId === id) {
				invalidateSelection();
				state.savedFolders.activeEntryId = null;
				state.activeSource = null;
			}
			publishSaved();
			return clone(state);
		},
		async activateSavedFolder(id) {
			const entry = state.savedFolders.entries.find((item) => item.id === id);
			if (
				!entry ||
				state.savedFolders.access[entry.folderId]?.state !== "available"
			)
				return { kind: "cancelled" };
			if (state.savedFolders.activeEntryId !== id) invalidateSelection();
			state.savedFolders.activeEntryId = id;
			state.activeSource = {
				id: sourceId,
				selectionId,
				displayName: folderLabel(entry),
				availability: "available",
			};
			publishSaved();
			return { kind: "selected", state: clone(state) };
		},
		async clearActiveFolder() {
			invalidateSelection();
			state.savedFolders.activeEntryId = null;
			state.activeSource = null;
			publishSaved();
			return clone(state);
		},
		async checkSavedFolders() {
			return cloneSavedFolders(state.savedFolders);
		},
		capabilities: {
			chooseFolder: true,
			folderSelection: "native",
			locateFolder: false,
			originalAction: "none",
		},
		derivativeRequests,
		interactionCalls,
		async getBootstrapState() {
			return clone(state);
		},
		async chooseFolder() {
			if (options.cancelFolderPicker) return { kind: "cancelled" };
			const name = options.selectedFolderName ?? "Selected Folder";
			let entry = state.savedFolders.entries.find(
				(item) => item.folderId === name,
			);
			if (!entry) {
				entry = {
					id: `saved-${++savedSequence}`,
					folderId: name,
					name,
					displayPath: `/Photos/${name}`,
					customLabel: null,
				};
				state.savedFolders.entries.push(entry);
			}
			state.savedFolders.access[entry.folderId] = {
				folderId: entry.folderId,
				state: "available",
				generation: 1,
				retryAfterMs: 5000,
			};
			if (
				state.savedFolders.activeEntryId &&
				state.savedFolders.activeEntryId !== entry.id
			)
				invalidateSelection();
			state.savedFolders.activeEntryId = entry.id;
			state.savedFolders.hasOpenedFolder = true;
			state = {
				...state,
				activeSource: {
					id: sourceId,
					selectionId,
					displayName: folderLabel(entry),
					availability: "available",
				},
			};
			publishSaved();
			return { kind: "selected", state: clone(state) };
		},
		async updateAppearance(appearance: Appearance) {
			state = { ...state, settings: { ...state.settings, appearance } };
			return clone(state);
		},
		async updateGalleryScope(galleryScope: GalleryScope) {
			state = { ...state, settings: { ...state.settings, galleryScope } };
			return clone(state);
		},
		async queryWall(request: WallQueryRequest): Promise<WallPage> {
			if (
				!Number.isInteger(request.limit) ||
				request.limit < 1 ||
				request.limit > maxWallPageSize
			) {
				throw new PhotoServiceError(
					"invalidLimit",
					"The requested wall page is not valid.",
				);
			}
			const ordered = [...assets]
				.filter((asset) => !settled || asset.capturedAtUtc !== null)
				.sort((left, right) => {
					if (!settled) {
						return (
							left.provisionalOrder - right.provisionalOrder ||
							left.id.localeCompare(right.id)
						);
					}
					const leftDate = left.capturedAtUtc ?? "";
					const rightDate = right.capturedAtUtc ?? "";
					const dateOrder = leftDate.localeCompare(rightDate);
					if (dateOrder !== 0) {
						return request.direction === "newestFirst" ? -dateOrder : dateOrder;
					}
					return (
						left.displayName.localeCompare(right.displayName) ||
						left.id.localeCompare(right.id)
					);
				});
			const offset = request.cursor === null ? 0 : Number(request.cursor);
			const start = Number.isSafeInteger(offset) && offset >= 0 ? offset : 0;
			const items = ordered.slice(start, start + request.limit).map(clone);
			const nextCursor =
				start + items.length < ordered.length
					? String(start + items.length)
					: null;
			return {
				items,
				nextCursor,
				orderState: settled ? "settled" : "provisional",
				sourceWarnings: clone(sourceWarnings),
				totalCount: ordered.length,
				previewCounts: {
					wallReady: ordered.filter((item) => item.wallThumbnail !== null)
						.length,
					screenReady: ordered.filter((item) => item.screenPreview !== null)
						.length,
				},
			};
		},
		async requestDerivatives(request: DerivativeRequest) {
			derivativeRequests.push(
				clone({ ...request, assetIds: [...request.assetIds] }),
			);
			const derivatives = fixtures.flatMap((fixture) => {
				if (!request.assetIds.includes(fixture.id)) return [];
				const derivative = fixture[request.kind];
				return derivative ? [derivative] : [];
			});
			if (derivatives.length === 0) return;
			for (const derivative of derivatives) {
				const index = assets.findIndex(
					(asset) => asset.id === derivative.assetId,
				);
				const current = assets[index];
				if (!current) continue;
				assets[index] = {
					...current,
					[request.kind]: clone(derivative),
				};
			}
			publish({
				kind: "derivativesReady",
				selectionId,
				derivatives: clone(derivatives),
			});
			return undefined;
		},
		async setWallInteraction(active: boolean) {
			interactionCalls.push(active);
			return undefined;
		},
		watchWallUpdates(listener: WallListener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		derivativeUrl(reference: DerivativeReference) {
			const url = derivativeUrls.get(`${reference.kind}:${reference.key}`);
			if (!url) {
				throw new PhotoServiceError(
					"derivativeUnavailable",
					"That photo derivative is not available.",
				);
			}
			return url;
		},
		async listFolders() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"This host uses its system folder picker.",
			);
		},
		async selectFolder() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"This host uses its system folder picker.",
			);
		},
		folderBrowserState() {
			return { breadcrumbs: [], initialPath: "" };
		},
		initialSortDirection() {
			return sortDirection;
		},
		rememberSortDirection(direction: SortDirection) {
			sortDirection = direction;
		},
		startFixtureScan() {
			if (scanPromise && !settled) return scanPromise;
			const generation = ++scanGeneration;
			settled = false;
			settlementPromise = null;
			derivativePhasePromise = null;
			assets = fixtures.map(
				({
					derivativeUrls: _urls,
					wallThumbnailUrl: _wallUrl,
					screenPreviewUrl: _screenUrl,
					...asset
				}) => ({
					...asset,
					dateState: "provisional" as const,
					wallThumbnail: null,
					screenPreview: null,
				}),
			);
			scanPromise = (async () => {
				await delay(options.geometryDelayMs ?? 0);
				if (generation !== scanGeneration) return;
				publish({
					kind: "catalogBatch",
					selectionId,
					sourceId,
					assets: clone(assets),
					orderState: "provisional",
					generation,
					progress: progress(0),
				});
				derivativePhasePromise = (async () => {
					await delay(options.thumbnailDelayMs ?? 0);
					if (generation !== scanGeneration) return;
					const derivatives = derivativeReferences().filter(
						(derivative) => derivative.kind === "wallThumbnail",
					);
					if (derivatives.length === 0) return;
					for (const derivative of derivatives) {
						const index = assets.findIndex(
							(asset) => asset.id === derivative.assetId,
						);
						const current = assets[index];
						if (!current) continue;
						if (derivative.kind === "wallThumbnail") {
							assets[index] = {
								...current,
								wallThumbnail: clone(derivative),
							};
						} else {
							assets[index] = {
								...current,
								screenPreview: clone(derivative),
							};
						}
					}
					publish({
						kind: "derivativesReady",
						selectionId,
						derivatives: clone(derivatives),
					});
				})();
			})();
			return scanPromise;
		},
		finishFixtureScan() {
			if (settled) {
				scanPromise = null;
				settlementPromise = null;
				void service.startFixtureScan();
			}
			if (settlementPromise) return settlementPromise;
			settlementPromise = (async () => {
				if (!scanPromise) await service.startFixtureScan();
				const generation = scanGeneration;
				await scanPromise;
				if (derivativePhasePromise) await derivativePhasePromise;
				await delay(options.metadataDelayMs ?? 0);
				if (generation !== scanGeneration) return;
				if (settled) return;
				settled = true;
				assets = assets.map((asset) => ({ ...asset, dateState: "settled" }));
				publish({
					kind: "metadataSettled",
					selectionId,
					sourceId,
					generation: scanGeneration,
				});
			})();
			return settlementPromise;
		},
		emitForTest(update: WallUpdate) {
			if (update.selectionId !== selectionId) return;
			if (update.kind === "sourceUnavailable") {
				const entry = state.savedFolders.entries.find(
					(item) => item.id === state.savedFolders.activeEntryId,
				);
				if (entry) {
					state.savedFolders.access[entry.folderId] = {
						folderId: entry.folderId,
						state: "rootOffline",
						generation:
							(state.savedFolders.access[entry.folderId]?.generation ?? 0) + 1,
						retryAfterMs: 5000,
					};
					if (state.activeSource)
						state.activeSource.availability = "rootOffline";
					publishSaved();
				}
			}
			if (update.kind === "warning" && update.assetId === null) {
				sourceWarnings = [
					...sourceWarnings.filter(
						(warning) => warning.code !== update.warning.code,
					),
					clone(update.warning),
				];
			} else if (update.kind === "warningCleared" && update.assetId === null) {
				sourceWarnings = sourceWarnings.filter(
					(warning) => warning.code !== update.code,
				);
			}
			publish(update);
		},
	};

	return service;
}
