export type Appearance = "system" | "light" | "dark";
export type SourceAvailability =
	| "available"
	| "rootOffline"
	| "missing"
	| "unreadable";

export interface SettingsState {
	appearance: Appearance;
}

export interface SourceSummary {
	id: string;
	displayName: string;
	availability: SourceAvailability;
}

export interface BootstrapState {
	settings: SettingsState;
	activeSource: SourceSummary | null;
}

export type ChooseFolderResult =
	| { kind: "cancelled" }
	| { kind: "selected"; state: BootstrapState };

export interface PhotoServiceCapabilities {
	chooseFolder: boolean;
	locateFolder: boolean;
}

export type SortDirection = "oldestFirst" | "newestFirst";
export type OrderState = "provisional" | "settled";
export type DerivativeClass = "wallThumbnail" | "screenPreview";
export type DerivativePriority = "visible" | "nearViewport";
export type WallMediaKind =
	| "jpeg"
	| "png"
	| "tiff"
	| "heif"
	| "webp"
	| "avif"
	| "raw"
	| "video"
	| "unknown";
export type WallShapeState = "ready" | "fallback";

export interface DerivativeReference {
	assetId: string;
	kind: DerivativeClass;
	key: string;
}

export interface WallWarningState {
	code: string;
	retryable: boolean;
}

export interface WallAsset {
	id: string;
	displayName: string;
	mediaKind: WallMediaKind;
	provisionalOrder: number;
	capturedAtUtc: string | null;
	dateState: OrderState;
	width: number;
	height: number;
	representativeRgb: number | null;
	shapeState: WallShapeState;
	availability: SourceAvailability;
	warning: WallWarningState | null;
	wallThumbnail: DerivativeReference | null;
	screenPreview: DerivativeReference | null;
}

export interface WallPage {
	items: WallAsset[];
	nextCursor: string | null;
	orderState: OrderState;
}

export interface WallQueryRequest {
	cursor: string | null;
	limit: number;
	direction: SortDirection;
}

export interface DerivativeRequest {
	assetIds: string[];
	priority: DerivativePriority;
}

export interface ScanProgressDto {
	discovered: number;
	shaped: number;
	enriched: number;
	total: number | null;
}

export type WallUpdate =
	| {
			kind: "catalogBatch";
			sourceId?: string;
			assets: WallAsset[];
			orderState: OrderState;
			progress: ScanProgressDto;
	  }
	| { kind: "derivativesReady"; derivatives: DerivativeReference[] }
	| { kind: "metadataSettled"; sourceId: string }
	| { kind: "progress"; progress: ScanProgressDto }
	| { kind: "sourceUnavailable"; sourceId: string }
	| {
			kind: "warning";
			sourceId: string;
			assetId: string | null;
			warning: WallWarningState;
	  }
	| {
			kind: "warningCleared";
			sourceId: string;
			assetId: string | null;
			code: string;
	  };

export interface PhotoService {
	readonly capabilities: PhotoServiceCapabilities;
	getBootstrapState(): Promise<BootstrapState>;
	chooseFolder(): Promise<ChooseFolderResult>;
	updateAppearance(appearance: Appearance): Promise<BootstrapState>;
	queryWall(request: WallQueryRequest): Promise<WallPage>;
	requestDerivatives(request: DerivativeRequest): Promise<void>;
	setWallInteraction(active: boolean): Promise<void>;
	watchWallUpdates(listener: (update: WallUpdate) => void): () => void;
	derivativeUrl(reference: DerivativeReference): string;
}

export class PhotoServiceError extends Error {
	constructor(
		readonly code: string,
		message: string,
	) {
		super(message);
		this.name = "PhotoServiceError";
	}
}
