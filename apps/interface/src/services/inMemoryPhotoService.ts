import type {
	Appearance,
	BootstrapState,
	DerivativeClass,
	DerivativeReference,
	DerivativeRequest,
	PhotoService,
	ScanProgressDto,
	WallAsset,
	WallPage,
	WallQueryRequest,
	WallUpdate,
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
}

export interface InMemoryPhotoService extends PhotoService {
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
	let scanPromise: Promise<void> | null = null;
	let derivativePhasePromise: Promise<void> | null = null;
	let settlementPromise: Promise<void> | null = null;
	let state: BootstrapState = {
		settings: { appearance: "system" },
		activeSource: null,
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
		for (const listener of listeners) listener(clone(update));
	};
	const progress = (enriched: number): ScanProgressDto => ({
		discovered: fixtures.length,
		shaped: fixtures.length,
		enriched,
		total: fixtures.length,
	});
	const derivativeReferences = (): DerivativeReference[] =>
		fixtures.flatMap((fixture) =>
			[fixture.wallThumbnail, fixture.screenPreview].filter(
				(reference): reference is DerivativeReference => reference !== null,
			),
		);

	const service: InMemoryPhotoService = {
		capabilities: { chooseFolder: true, locateFolder: false },
		async getBootstrapState() {
			return clone(state);
		},
		async chooseFolder() {
			if (options.cancelFolderPicker) return { kind: "cancelled" };
			state = {
				...state,
				activeSource: {
					id: sourceId,
					displayName: options.selectedFolderName ?? "Selected Folder",
					availability: "available",
				},
			};
			return { kind: "selected", state: clone(state) };
		},
		async updateAppearance(appearance: Appearance) {
			state = { ...state, settings: { appearance } };
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
			};
		},
		async requestDerivatives(_request: DerivativeRequest) {
			return undefined;
		},
		async setWallInteraction(_active: boolean) {
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
		startFixtureScan() {
			if (scanPromise && !settled) return scanPromise;
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
				publish({
					kind: "catalogBatch",
					sourceId,
					assets: clone(assets),
					orderState: "provisional",
					progress: progress(0),
				});
				derivativePhasePromise = (async () => {
					await delay(options.thumbnailDelayMs ?? 0);
					const derivatives = derivativeReferences();
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
						derivatives: clone(derivatives),
					});
				})();
			})();
			return scanPromise;
		},
		finishFixtureScan() {
			if (settlementPromise) return settlementPromise;
			settlementPromise = (async () => {
				if (!scanPromise) await service.startFixtureScan();
				await scanPromise;
				if (derivativePhasePromise) await derivativePhasePromise;
				await delay(options.metadataDelayMs ?? 0);
				if (settled) return;
				settled = true;
				assets = assets.map((asset) => ({ ...asset, dateState: "settled" }));
				publish({ kind: "metadataSettled", sourceId });
			})();
			return settlementPromise;
		},
		emitForTest(update: WallUpdate) {
			publish(update);
		},
	};

	return service;
}
