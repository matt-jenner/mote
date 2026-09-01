export type Appearance = "system" | "light" | "dark";
export type GalleryScope = "currentFolder" | "includeSubfolders";
export type SourceAvailability =
	| "available"
	| "rootOffline"
	| "missing"
	| "unreadable";

export interface SettingsState {
	appearance: Appearance;
	galleryScope: GalleryScope;
}

export interface SourceSummary {
	id: string;
	/** Opaque host identity for the selected folder and selection epoch. */
	selectionId: string;
	displayName: string;
	availability: SourceAvailability;
}

export interface FolderBreadcrumb {
	name: string;
	path: string;
}

export interface FolderEntry {
	name: string;
	path: string;
}

export interface FolderListing {
	path: string;
	breadcrumbs: FolderBreadcrumb[];
	children: FolderEntry[];
	imageCount: number | null;
}

export interface FolderBrowserState {
	breadcrumbs: FolderBreadcrumb[];
	initialPath: string;
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
	folderSelection: "native" | "hosted";
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
	rating: number | null;
}

export interface WallPage {
	items: WallAsset[];
	nextCursor: string | null;
	orderState: OrderState;
	sourceWarnings: WallWarningState[];
}

export interface WallQueryRequest {
	cursor: string | null;
	limit: number;
	direction: SortDirection;
}

export interface DerivativeRequest {
	assetIds: string[];
	priority: DerivativePriority;
	kind: DerivativeClass;
}

export interface ScanProgressDto {
	discovered: number;
	shaped: number;
	enriched: number;
	total: number | null;
}

export type WallUpdateBase =
	| {
			kind: "catalogBatch";
			selectionId: string;
			sourceId?: string;
			assets: WallAsset[];
			orderState: OrderState;
			generation: number;
			progress: ScanProgressDto;
	  }
	| {
			kind: "derivativesReady";
			selectionId: string;
			derivatives: DerivativeReference[];
	  }
	| {
			kind: "metadataSettled";
			selectionId: string;
			sourceId: string;
			generation: number;
	  }
	| {
			kind: "progress";
			selectionId: string;
			generation: number;
			progress: ScanProgressDto;
	  }
	| { kind: "sourceUnavailable"; selectionId: string; sourceId: string }
	| {
			kind: "warning";
			selectionId: string;
			sourceId: string;
			assetId: string | null;
			warning: WallWarningState;
	  }
	| {
			kind: "warningCleared";
			selectionId: string;
			sourceId: string;
			assetId: string | null;
			code: string;
	  };

export type ResyncRequiredUpdate = {
	kind: "resyncRequired";
	selectionId: string;
};

export type WallUpdate = WallUpdateBase | ResyncRequiredUpdate;

export interface PhotoService {
	readonly capabilities: PhotoServiceCapabilities;
	getBootstrapState(): Promise<BootstrapState>;
	chooseFolder(): Promise<ChooseFolderResult>;
	updateAppearance(appearance: Appearance): Promise<BootstrapState>;
	updateGalleryScope(scope: GalleryScope): Promise<BootstrapState>;
	queryWall(request: WallQueryRequest): Promise<WallPage>;
	requestDerivatives(request: DerivativeRequest): Promise<void>;
	setWallInteraction(active: boolean): Promise<void>;
	watchWallUpdates(listener: (update: WallUpdate) => void): () => void;
	derivativeUrl(reference: DerivativeReference): string;
	listFolders(path: string): Promise<FolderListing>;
	selectFolder(path: string): Promise<ChooseFolderResult>;
	folderBrowserState(): FolderBrowserState;
	initialSortDirection(): SortDirection;
	rememberSortDirection(direction: SortDirection): void;
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
