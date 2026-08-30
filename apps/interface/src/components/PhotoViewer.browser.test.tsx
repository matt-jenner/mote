import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import axe from "axe-core";
import { StrictMode, useRef, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import {
	createInMemoryPhotoService,
	type InMemoryPhotoService,
} from "../services/inMemoryPhotoService";
import type {
	DerivativeReference,
	DerivativeRequest,
	PhotoService,
	WallAsset,
	WallPage,
	WallQueryRequest,
} from "../services/photoService";
import "../styles/tokens.css";
import "../styles/global.css";
import { GestureLifecycleHarness } from "../viewer/useViewerGestures.test";
import { useViewerPreview } from "../viewer/useViewerPreview";
import { useViewerTransform } from "../viewer/useViewerTransform";
import { AppShell } from "./AppShell";
import { PhotoViewerOverlay } from "./PhotoViewerOverlay";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerNavigator } from "./ViewerNavigator";
import { ViewerStage } from "./ViewerStage";

interface Gate<T> {
	promise: Promise<T>;
	resolve: (value: T) => void;
	reject: (reason?: unknown) => void;
}

function gate<T>(): Gate<T> {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((nextResolve, nextReject) => {
		resolve = nextResolve;
		reject = nextReject;
	});
	return { promise, resolve, reject };
}

function asset(id: string, displayName: string, order: number): WallAsset {
	return {
		id,
		displayName,
		mediaKind: "jpeg",
		provisionalOrder: order,
		capturedAtUtc: `2024-01-${String(order).padStart(2, "0")}T12:00:00Z`,
		dateState: "settled",
		width: 1200,
		height: 800,
		representativeRgb: 0x225670,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: {
			assetId: id,
			kind: "wallThumbnail",
			key: `${id}-wall`,
		},
		screenPreview: null,
		rating: null,
	};
}

function serviceWithReadyPhotos(
	brokenScreenPreview = false,
	photoCount = 60,
	readyScreenPreview = false,
): InMemoryPhotoService {
	const demoWallImages = [
		"coast",
		"forest",
		"city",
		"mountain",
		"portrait",
		"interior",
	] as const;
	const assets = [
		{
			...asset("coast", "Coast", 1),
			screenPreview:
				brokenScreenPreview || readyScreenPreview
					? {
							assetId: "coast",
							kind: "screenPreview" as const,
							key: "coast-screen",
						}
					: null,
		},
		...Array.from({ length: photoCount }, (_, index) =>
			asset(`photo-${index}`, `Photo ${index}`, index + 2),
		),
	];
	const datedAssets = assets.map((item, index) => ({
		...item,
		capturedAtUtc: new Date(Date.UTC(2024, 0, index + 1)).toISOString(),
	}));
	return createInMemoryPhotoService({
		selectedFolderName: "Iceland 2025",
		wallAssets: datedAssets.map((item, index) => ({
			...item,
			wallThumbnailUrl: `/demo-photos/${demoWallImages[index % demoWallImages.length]}.jpg`,
			screenPreviewUrl:
				item.id === "coast"
					? brokenScreenPreview
						? "/demo-photos/missing.jpg"
						: readyScreenPreview
							? "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=="
							: undefined
					: undefined,
		})),
	});
}

const cachedPixel =
	"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";
const cachedZoomFixture =
	"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1536' height='1024' viewBox='0 0 1536 1024'%3E%3Crect width='100%25' height='100%25' fill='%23225670'/%3E%3C/svg%3E";

function cachedOfflineService() {
	const service = serviceWithReadyPhotos(false, 1, true);
	const originalDerivativeUrl = service.derivativeUrl.bind(service);
	const cached = new Map<string, string>();
	const sentinel = "file:///private/source/secret.jpg";
	let unavailable = false;
	const opaqueUrls = new Map<string, string>([
		["wallThumbnail:coast-wall", `${cachedZoomFixture}#cached-coast-wall`],
		["screenPreview:coast-screen", `${cachedZoomFixture}#cached-coast-screen`],
		["wallThumbnail:photo-0-wall", `${cachedZoomFixture}#cached-photo-0-wall`],
	]);
	service.derivativeUrl = (reference: DerivativeReference) => {
		const key = `${reference.kind}:${reference.key}`;
		if (unavailable) {
			const ready = cached.get(key);
			if (ready) return ready;
			throw new Error(`Source unavailable: ${sentinel}`);
		}
		const url = opaqueUrls.get(key) ?? originalDerivativeUrl(reference);
		cached.set(key, url);
		return url;
	};
	return {
		service,
		sentinel,
		prime: () => {
			for (const [kind, key] of [
				["wallThumbnail", "coast-wall"],
				["screenPreview", "coast-screen"],
				["wallThumbnail", "photo-0-wall"],
			] as const)
				service.derivativeUrl({ assetId: kind, kind, key });
		},
		goOffline: () => {
			unavailable = true;
			service.emitForTest({
				kind: "sourceUnavailable",
				selectionId: "memory-selection-1",
				sourceId: "memory-source",
			});
		},
		cachedScreenUrl: opaqueUrls.get("screenPreview:coast-screen") ?? "",
		cachedNeighbourUrl: opaqueUrls.get("wallThumbnail:photo-0-wall") ?? "",
	};
}

interface PageGate {
	request: WallQueryRequest;
	resolve: (page: WallPage) => void;
}

function installVisualViewportDouble(width: number, height: number) {
	const viewport = new EventTarget() as EventTarget & {
		width: number;
		height: number;
		setSize: (nextWidth: number, nextHeight: number) => void;
	};
	viewport.width = width;
	viewport.height = height;
	viewport.setSize = (nextWidth, nextHeight) => {
		viewport.width = nextWidth;
		viewport.height = nextHeight;
	};
	const descriptor = Object.getOwnPropertyDescriptor(window, "visualViewport");
	Object.defineProperty(window, "visualViewport", {
		configurable: true,
		value: viewport,
	});
	return () => {
		if (descriptor) Object.defineProperty(window, "visualViewport", descriptor);
		else delete (window as { visualViewport?: VisualViewport }).visualViewport;
	};
}

function installPointerModality(coarse: boolean) {
	const original = window.matchMedia;
	window.matchMedia = ((query: string) => {
		const list = original.call(window, query);
		if (query !== "(pointer: coarse)") return list;
		return new Proxy(list, {
			get(target, property, _receiver) {
				if (property === "matches") return coarse;
				const value = Reflect.get(target, property, target);
				return typeof value === "function" ? value.bind(target) : value;
			},
		});
	}) as typeof window.matchMedia;
	return () => {
		window.matchMedia = original;
	};
}

function delayedPaginationService() {
	const service = serviceWithReadyPhotos(false, 120);
	const queryRequests: WallQueryRequest[] = [];
	const queryWall = service.queryWall.bind(service);
	let heldPage: PageGate | null = null;
	service.queryWall = (request) => {
		queryRequests.push(request);
		if (request.cursor === "100" && heldPage === null) {
			return new Promise<WallPage>((resolve) => {
				heldPage = { request, resolve };
			});
		}
		return queryWall(request);
	};
	return {
		service,
		queryRequests,
		releaseNextPage: async () => {
			if (!heldPage) throw new Error("No delayed page is pending");
			const page = await queryWall(heldPage.request);
			heldPage.resolve(page);
			heldPage = null;
		},
	};
}

async function openManyAsset(name: string) {
	const service = serviceWithReadyPhotos(false, 120);
	return openAssetWithService(service, name);
}

async function openAssetWithService(
	service: InMemoryPhotoService,
	name: string,
) {
	const view = await renderViewerWall(service);
	await view.getByRole("button", { name: "Choose Folder" }).click();
	await service.finishFixtureScan();
	const tile = view.getByRole("button", { name: `Open ${name}`, exact: true });
	await expect.element(tile).toBeVisible();
	(tile.element() as HTMLButtonElement).click();
	await expect
		.element(view.getByRole("dialog", { name: "Photo viewer" }))
		.toBeVisible();
	return { service, view };
}

function renderViewerWall(service: InMemoryPhotoService) {
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	return render(
		<QueryClientProvider client={queryClient}>
			<PhotoServiceProvider service={service}>
				<AppShell />
			</PhotoServiceProvider>
		</QueryClientProvider>,
	);
}

function renderStrictViewerWall(service: InMemoryPhotoService) {
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	return render(
		<StrictMode>
			<QueryClientProvider client={queryClient}>
				<PhotoServiceProvider service={service}>
					<AppShell />
				</PhotoServiceProvider>
			</QueryClientProvider>
		</StrictMode>,
	);
}

function previewAsset(id: string, order: number): WallAsset {
	return {
		...asset(id, id.toUpperCase(), order),
		screenPreview: {
			assetId: id,
			kind: "screenPreview",
			key: `${id}-screen`,
		},
	};
}

function previewService(
	requestDerivatives: PhotoService["requestDerivatives"],
): PhotoService {
	return {
		capabilities: {
			chooseFolder: false,
			folderSelection: "native",
			locateFolder: false,
		},
		getBootstrapState: async () => ({
			settings: { appearance: "system", galleryScope: "includeSubfolders" },
			activeSource: null,
		}),
		chooseFolder: async () => ({ kind: "cancelled" }),
		updateAppearance: async (appearance) => ({
			settings: { appearance, galleryScope: "includeSubfolders" },
			activeSource: null,
		}),
		updateGalleryScope: async (galleryScope) => ({
			settings: { appearance: "system", galleryScope },
			activeSource: null,
		}),
		queryWall: async (): Promise<WallPage> => ({
			items: [],
			nextCursor: null,
			orderState: "settled",
			sourceWarnings: [],
		}),
		requestDerivatives,
		setWallInteraction: async () => undefined,
		watchWallUpdates: () => () => undefined,
		derivativeUrl: (reference) =>
			`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==#${reference.assetId}-${reference.kind}`,
		listFolders: async () => {
			throw new Error("native picker fixture");
		},
		selectFolder: async () => {
			throw new Error("native picker fixture");
		},
		folderBrowserState: () => ({ breadcrumbs: [], initialPath: "" }),
		initialSortDirection: () => "oldestFirst",
		rememberSortDirection: () => undefined,
	};
}

function seriousViolations(result: axe.AxeResults) {
	return result.violations.filter(
		(violation) =>
			violation.impact === "serious" || violation.impact === "critical",
	);
}

function PreviewHarness({
	assets,
	service,
}: {
	assets: readonly WallAsset[];
	service: PhotoService;
}) {
	const [currentIndex, setCurrentIndex] = useState(0);
	const [generation, setGeneration] = useState(1);
	const preview = useViewerPreview({
		assets,
		currentIndex,
		previewGeneration: generation,
	});
	const current = assets[currentIndex];
	if (!current) return null;
	return (
		<>
			<button
				data-testid="switch-preview"
				onClick={() => {
					setCurrentIndex(1);
					setGeneration(2);
				}}
				type="button"
			>
				Switch
			</button>
			<button
				data-testid="report-preview-interaction"
				onClick={preview.reportInteraction}
				type="button"
			>
				Report interaction
			</button>
			<ViewerStage
				asset={current}
				baseUrl={preview.baseUrl}
				currentUrl={preview.currentUrl}
				largePreviewUnavailable={preview.largePreviewUnavailable}
				previewGeneration={generation}
				service={service}
			/>
		</>
	);
}

function ActivatablePreviewHarness({
	assets,
	service,
}: {
	assets: readonly WallAsset[];
	service: PhotoService;
}) {
	const [active, setActive] = useState(true);
	const activeAssets = active ? assets : [];
	const preview = useViewerPreview({
		assets: activeAssets,
		currentIndex: 0,
		previewGeneration: 1,
	});
	const current = activeAssets[0];
	return (
		<>
			<button
				data-testid="deactivate-preview"
				onClick={() => setActive(false)}
				type="button"
			>
				Deactivate
			</button>
			<button
				data-testid="activate-preview"
				onClick={() => setActive(true)}
				type="button"
			>
				Activate
			</button>
			<output data-testid="preview-active">{String(active)}</output>
			{current ? (
				<ViewerStage
					asset={current}
					baseUrl={preview.baseUrl}
					currentUrl={preview.currentUrl}
					largePreviewUnavailable={preview.largePreviewUnavailable}
					previewGeneration={1}
					service={service}
				/>
			) : null}
		</>
	);
}

const largePreviewFixture =
	"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4096' height='2731' viewBox='0 0 4096 2731'%3E%3Crect width='4096' height='2731' fill='%23225670'/%3E%3C/svg%3E";

const thumbnailCeilingFixture =
	"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1536' height='1024' viewBox='0 0 1536 1024'%3E%3Crect width='1536' height='1024' fill='%23225670'/%3E%3C/svg%3E";
const previewCeilingFixture =
	"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4096' height='2731' viewBox='0 0 4096 2731'%3E%3Crect width='4096' height='2731' fill='%233a8f68'/%3E%3C/svg%3E#preview-ceiling";

function DecodeCeilingHarness() {
	const transform = useViewerTransform({
		assetId: "decode-ceiling",
		assetRevision: 1,
		imageWidth: 1200,
		imageHeight: 800,
		onInteraction: () => undefined,
	});
	return (
		<>
			<button
				data-testid="decode-ceiling-zoom"
				onClick={() => transform.step(1, { x: 300, y: 200 })}
				type="button"
			>
				Zoom
			</button>
			<output data-testid="decode-ceiling-max">
				{transform.geometry.maxScale}
			</output>
			<output data-testid="decode-ceiling-scale">
				{transform.geometry.scale}
			</output>
			<div style={{ width: "600px", height: "400px" }}>
				<ViewerStage
					asset={asset("decode-ceiling", "Decode ceiling", 1)}
					baseUrl={thumbnailCeilingFixture}
					currentUrl={previewCeilingFixture}
					onDrawableSizeChange={transform.setDrawableSize}
					onNaturalSizeChange={transform.setNaturalSize}
					previewGeneration={1}
					service={previewService(async () => undefined)}
					transform={transform.geometry}
					viewportHeight={400}
					viewportWidth={600}
				/>
			</div>
		</>
	);
}

function TransformHarness() {
	const [previewReady, setPreviewReady] = useState(false);
	const [revision, setRevision] = useState(1);
	const [rotated, setRotated] = useState(false);
	const transform = useViewerTransform({
		assetId: "transform-harness",
		assetRevision: revision,
		imageWidth: 6000,
		imageHeight: 4000,
		onInteraction: () => undefined,
	});
	const currentWidth = rotated ? 400 : 600;
	const currentHeight = rotated ? 600 : 400;
	return (
		<>
			<button
				data-testid="zoom-programmatically"
				onClick={() => {
					transform.setDrawableSize({ width: 600, height: 400 });
					transform.setNaturalSize({ width: 1200, height: 800 });
					transform.zoomAt(1.5, { x: 450, y: 150 });
				}}
				type="button"
			>
				Zoom
			</button>
			<button
				data-testid="release-preview-decode"
				onClick={() => setPreviewReady(true)}
				type="button"
			>
				Release preview
			</button>
			<button
				data-testid="rotate-transform"
				onClick={() => setRotated(true)}
				type="button"
			>
				Rotate
			</button>
			<button
				data-testid="navigate-transform"
				onClick={() => setRevision((value) => value + 1)}
				type="button"
			>
				Navigate
			</button>
			<div style={{ width: `${currentWidth}px`, height: `${currentHeight}px` }}>
				<ViewerStage
					asset={asset("transform-harness", "Transform", 1)}
					baseUrl={largePreviewFixture}
					currentUrl={previewReady ? largePreviewFixture : null}
					onNaturalSizeChange={transform.setNaturalSize}
					onDrawableSizeChange={transform.setDrawableSize}
					service={previewService(async () => undefined)}
					transform={transform.geometry}
					viewportHeight={currentHeight}
					viewportWidth={currentWidth}
				/>
			</div>
			<output data-testid="transform-focal">
				{JSON.stringify(transform.geometry.focal)}
			</output>
		</>
	);
}

function LetterboxTransformHarness() {
	const [zoomed, setZoomed] = useState(false);
	const transform = useViewerTransform({
		assetId: "letterbox-harness",
		assetRevision: 1,
		imageWidth: 1600,
		imageHeight: 800,
		onInteraction: () => undefined,
	});
	return (
		<>
			<button
				data-testid="zoom-letterboxed"
				onClick={() => {
					transform.setDrawableSize({ width: 712, height: 512 });
					transform.setNaturalSize({ width: 3200, height: 1600 });
					transform.zoomAt(1.5, { x: 356, y: 256 });
					setZoomed(true);
				}}
				type="button"
			>
				Zoom letterboxed
			</button>
			<ViewerStage
				asset={{
					...asset("letterbox-harness", "Letterbox", 1),
					width: 1600,
					height: 800,
				}}
				baseUrl={largePreviewFixture}
				onDrawableSizeChange={transform.setDrawableSize}
				onNaturalSizeChange={transform.setNaturalSize}
				service={previewService(async () => undefined)}
				transform={transform.geometry}
				viewportHeight={512}
				viewportWidth={712}
			/>
			<output data-testid="letterbox-zoomed">{String(zoomed)}</output>
		</>
	);
}

function RevisionPaintHarness() {
	const [revision, setRevision] = useState(1);
	const renderModes = useRef<string[]>([]);
	const transform = useViewerTransform({
		assetId: `revision-${revision}`,
		assetRevision: revision,
		imageWidth: 1200,
		imageHeight: 800,
		onInteraction: () => undefined,
	});
	renderModes.current.push(transform.mode);
	return (
		<>
			<button
				data-testid="zoom-revision"
				onClick={() => {
					transform.setDrawableSize({ width: 600, height: 400 });
					transform.setNaturalSize({ width: 1200, height: 800 });
					transform.zoomAt(1.5, { x: 300, y: 200 });
				}}
				type="button"
			>
				Zoom revision
			</button>
			<button
				data-testid="navigate-revision"
				onClick={() => {
					renderModes.current = [];
					setRevision((value) => value + 1);
				}}
				type="button"
			>
				Navigate revision
			</button>
			<ViewerStage
				asset={asset(`revision-${revision}`, "Revision", revision)}
				baseUrl={largePreviewFixture}
				onDrawableSizeChange={transform.setDrawableSize}
				onNaturalSizeChange={transform.setNaturalSize}
				service={previewService(async () => undefined)}
				transform={transform.geometry}
				viewportHeight={400}
				viewportWidth={600}
			/>
			<output data-testid="revision-render-modes">
				{renderModes.current.join(",")}
			</output>
		</>
	);
}

function NavigatorLifecycleHarness() {
	const [assetRevision, setAssetRevision] = useState(1);
	const [viewportRevision, setViewportRevision] = useState(1);
	const [interactive, setInteractive] = useState(true);
	const [mounted, setMounted] = useState(true);
	const [endCount, setEndCount] = useState(0);
	return (
		<>
			{mounted ? (
				<ViewerNavigator
					assetName="Lifecycle photo"
					assetRevision={assetRevision}
					fallbackUrl={cachedPixel}
					imageHeight={800}
					imageUrl={null}
					imageWidth={1200}
					interactive={interactive}
					onInteraction={() => undefined}
					onManipulationChange={(active) => {
						if (!active) setEndCount((count) => count + 1);
					}}
					onRecenter={() => undefined}
					viewportRevision={viewportRevision}
					visible
					visibleRect={{ x: 0.25, y: 0.25, width: 0.5, height: 0.5 }}
				/>
			) : null}
			<button
				data-testid="navigator-lifecycle-asset"
				onClick={() => setAssetRevision((revision) => revision + 1)}
				type="button"
			>
				Asset
			</button>
			<button
				data-testid="navigator-lifecycle-viewport"
				onClick={() => setViewportRevision((revision) => revision + 1)}
				type="button"
			>
				Viewport
			</button>
			<button
				data-testid="navigator-lifecycle-modality"
				onClick={() => setInteractive((value) => !value)}
				type="button"
			>
				Modality
			</button>
			<button
				data-testid="navigator-lifecycle-unmount"
				onClick={() => setMounted(false)}
				type="button"
			>
				Unmount
			</button>
			<output data-testid="navigator-lifecycle-end-count">{endCount}</output>
		</>
	);
}

function RefreshingPreviewHarness({
	initialAssets,
	service,
}: {
	initialAssets: readonly WallAsset[];
	service: PhotoService;
}) {
	const [assets, setAssets] = useState(initialAssets);
	const preview = useViewerPreview({
		assets,
		currentIndex: 0,
		previewGeneration: 1,
	});
	const current = assets[0];
	if (!current) return null;
	return (
		<>
			<button
				data-testid="refresh-preview-assets"
				onClick={() =>
					setAssets((previous) =>
						previous.map((asset) => ({
							...asset,
							wallThumbnail: asset.wallThumbnail
								? { ...asset.wallThumbnail }
								: null,
							screenPreview: asset.screenPreview
								? { ...asset.screenPreview }
								: null,
						})),
					)
				}
				type="button"
			>
				Refresh assets
			</button>
			<ViewerStage
				asset={current}
				baseUrl={preview.baseUrl}
				currentUrl={preview.currentUrl}
				largePreviewUnavailable={preview.largePreviewUnavailable}
				previewGeneration={1}
				service={service}
			/>
		</>
	);
}

function StageFailureHarness() {
	const [currentIndex, setCurrentIndex] = useState(0);
	const assets = [asset("a", "A", 1), asset("b", "B", 2), asset("c", "C", 3)];
	const current = assets[currentIndex];
	if (!current) return null;
	return (
		<>
			<button
				data-testid="switch-stage-failure"
				onClick={() => setCurrentIndex((index) => Math.min(2, index + 1))}
				type="button"
			>
				Next stage asset
			</button>
			<ViewerStage
				asset={current}
				baseUrl={`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==#${current.id}-base`}
				currentUrl={`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==#${current.id}-screen`}
				largePreviewUnavailable={false}
				previewGeneration={currentIndex + 1}
				service={previewService(async () => undefined)}
			/>
		</>
	);
}

const zeroNaturalDerivative =
	"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='0' height='0'%3E%3C/svg%3E#zero-natural";
const invalidGeometryService = previewService(async () => undefined);
invalidGeometryService.derivativeUrl = (reference) =>
	reference.key === "zero-natural-screen" ? zeroNaturalDerivative : cachedPixel;

function InvalidGeometryViewerHarness() {
	const [currentIndex, setCurrentIndex] = useState(0);
	const [revision, setRevision] = useState(1);
	const assets = [
		{
			...asset("malformed", "Malformed", 1),
			width: 0,
			height: 0,
			wallThumbnail: null,
			screenPreview: null,
		},
		{
			...asset("zero-natural", "Zero natural", 2),
			wallThumbnail: null,
			screenPreview: {
				assetId: "zero-natural",
				kind: "screenPreview" as const,
				key: "zero-natural-screen",
			},
		},
	] as const;
	const current = assets[currentIndex] ?? assets[0];
	if (!current) return null;
	return (
		<PhotoViewerOverlay
			assets={assets}
			onClose={() => undefined}
			onLoadMore={() => undefined}
			onRequestNearViewportDerivatives={() => undefined}
			onSelectAsset={(assetId) => {
				const nextIndex = assets.findIndex(
					(candidate) => candidate.id === assetId,
				);
				if (nextIndex >= 0) {
					setCurrentIndex(nextIndex);
					setRevision((value) => value + 1);
				}
			}}
			onSetInfoOpen={() => undefined}
			onSetWallInteraction={() => undefined}
			onShowControls={() => undefined}
			onHideControls={() => undefined}
			onToggleTouchControls={() => undefined}
			loading={false}
			nextCursor={null}
			service={invalidGeometryService}
			state={{
				open: true,
				currentAssetId: current.id,
				returnAnchor: null,
				infoOpen: false,
				controlsVisible: true,
				filmstripVisible: true,
				previewGeneration: revision,
			}}
		/>
	);
}

async function openAsset(
	name: string,
	brokenScreenPreview = false,
	readyScreenPreview = false,
	photoCount = 60,
) {
	const service = serviceWithReadyPhotos(
		brokenScreenPreview,
		photoCount,
		readyScreenPreview,
	);
	const view = await renderViewerWall(service);
	await view.getByRole("button", { name: "Choose Folder" }).click();
	await service.finishFixtureScan();
	if (brokenScreenPreview) {
		service.emitForTest({
			kind: "derivativesReady",
			selectionId: "memory-selection-1",
			derivatives: [
				{
					assetId: "coast",
					kind: "screenPreview",
					key: "coast-screen",
				},
			],
		});
	}
	const tile = view.getByRole("button", { name: `Open ${name}`, exact: true });
	await expect.element(tile).toBeVisible();
	return { service, view, tile };
}

describe("immersive photo viewer checkpoint", () => {
	beforeEach(async () => {
		await page.viewport(1440, 1024);
	});

	afterEach(() => {
		document.documentElement.dataset.theme = "system";
		document.documentElement.style.colorScheme = "light dark";
	});

	it("keeps the same transform when the screen preview replaces the thumbnail", async () => {
		const view = await render(<TransformHarness />);
		await view.getByTestId("zoom-programmatically").click();
		const layer = view.getByTestId("viewer-transform-layer").element();
		const before = layer.style.transform;
		await view.getByTestId("release-preview-decode").click();
		await expect
			.element(view.getByTestId("viewer-transform-layer"))
			.toHaveAttribute("data-viewer-mode", "zoomed");
		expect(layer.style.transform).toBe(before);
	});

	it("keeps the thumbnail zoom ceiling until the screen preview decodes", async () => {
		const originalDecode = HTMLImageElement.prototype.decode;
		let releaseDecode: (() => void) | null = null;
		HTMLImageElement.prototype.decode = function () {
			if (this.src.includes("preview-ceiling"))
				return new Promise<void>((resolve) => {
					releaseDecode = resolve;
				});
			return originalDecode.call(this);
		};
		try {
			const view = await render(<DecodeCeilingHarness />);
			const stage = view.getByTestId("viewer-stage").element();
			const thumbnail = stage.querySelector<HTMLImageElement>(
				"[data-viewer-layer='wallThumbnail']",
			);
			const frame = view.getByTestId("viewer-frame").element();
			await expect.poll(() => thumbnail?.naturalWidth ?? 0).toBeGreaterThan(0);
			const thumbnailCeiling = Math.min(
				(thumbnail?.naturalWidth ?? 0) / frame.getBoundingClientRect().width,
				(thumbnail?.naturalHeight ?? 0) / frame.getBoundingClientRect().height,
			);
			await expect
				.poll(() =>
					Number(view.getByTestId("decode-ceiling-max").element().textContent),
				)
				.toBeCloseTo(thumbnailCeiling, 2);
			const release = releaseDecode as (() => void) | null;
			if (!release) throw new Error("screen preview decode did not start");
			await view.getByTestId("decode-ceiling-zoom").click();
			const scaleBefore = Number(
				view.getByTestId("decode-ceiling-scale").element().textContent,
			);
			expect(scaleBefore).toBe(1.25);
			release();
			await expect
				.poll(() =>
					Number(view.getByTestId("decode-ceiling-max").element().textContent),
				)
				.toBeGreaterThan(thumbnailCeiling);
			expect(
				Number(view.getByTestId("decode-ceiling-scale").element().textContent),
			).toBe(scaleBefore);
			await view.unmount();
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("restores the thumbnail zoom ceiling when screen preview decoding fails", async () => {
		const originalDecode = HTMLImageElement.prototype.decode;
		let rejectDecode: ((reason?: unknown) => void) | null = null;
		HTMLImageElement.prototype.decode = function () {
			if (this.src.includes("preview-ceiling"))
				return new Promise<void>((_, reject) => {
					rejectDecode = reject;
				});
			return originalDecode.call(this);
		};
		try {
			const view = await render(<DecodeCeilingHarness />);
			const stage = view.getByTestId("viewer-stage").element();
			const thumbnail = stage.querySelector<HTMLImageElement>(
				"[data-viewer-layer='wallThumbnail']",
			);
			const frame = view.getByTestId("viewer-frame").element();
			await expect.poll(() => thumbnail?.naturalWidth ?? 0).toBeGreaterThan(0);
			const thumbnailCeiling = Math.min(
				(thumbnail?.naturalWidth ?? 0) / frame.getBoundingClientRect().width,
				(thumbnail?.naturalHeight ?? 0) / frame.getBoundingClientRect().height,
			);
			await expect
				.poll(() =>
					Number(view.getByTestId("decode-ceiling-max").element().textContent),
				)
				.toBeCloseTo(thumbnailCeiling, 2);
			const reject = rejectDecode as ((reason?: unknown) => void) | null;
			if (!reject) throw new Error("screen preview decode did not start");
			await view.getByTestId("decode-ceiling-zoom").click();
			const scaleBefore = Number(
				view.getByTestId("decode-ceiling-scale").element().textContent,
			);
			expect(scaleBefore).toBe(1.25);
			reject(new Error("screen preview unavailable"));
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "true");
			await expect
				.poll(() =>
					Number(view.getByTestId("decode-ceiling-max").element().textContent),
				)
				.toBeCloseTo(thumbnailCeiling, 2);
			expect(
				Number(view.getByTestId("decode-ceiling-scale").element().textContent),
			).toBe(scaleBefore);
			await view.unmount();
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("resets on navigation while preserving zoom through rotation", async () => {
		const view = await render(<TransformHarness />);
		await view.getByTestId("zoom-programmatically").click();
		const beforeRotation = view
			.getByTestId("transform-focal")
			.element().textContent;
		await view.getByTestId("rotate-transform").click();
		await expect
			.poll(() => view.getByTestId("transform-focal").element().textContent)
			.toBe(beforeRotation);
		await view.getByTestId("navigate-transform").click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-viewer-mode", "fit");
	});

	it("lets a letterboxed zoomed layer expand into the drawable stage", async () => {
		const view = await render(<LetterboxTransformHarness />);
		await view.getByTestId("zoom-letterboxed").click();
		const stage = view.getByTestId("viewer-stage").element();
		const frame = view.getByTestId("viewer-frame").element();
		const layer = view.getByTestId("viewer-transform-layer").element();
		expect(getComputedStyle(stage).overflow).toBe("hidden");
		expect(getComputedStyle(frame).overflow).toBe("visible");
		expect(layer.getBoundingClientRect().width).toBeGreaterThan(
			frame.getBoundingClientRect().width,
		);
	});

	it("paints a newly selected asset at fit on its first revision render", async () => {
		const view = await render(<RevisionPaintHarness />);
		await view.getByTestId("zoom-revision").click();
		await view.getByTestId("navigate-revision").click();
		const modes = view
			.getByTestId("revision-render-modes")
			.element().textContent;
		expect(modes?.split(",").every((mode) => mode === "fit")).toBe(true);
	});

	it("supports desktop zoom controls, wheel, double-click, keyboard, and mouse pan", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		const dialog = view.getByRole("dialog", { name: "Photo viewer" });
		await expect.element(dialog).toBeVisible();
		const stage = view.getByTestId("viewer-stage").element();
		const zoomIn = view.getByRole("button", { name: "Zoom in" });
		const zoomOut = view.getByRole("button", { name: "Zoom out" });
		const resetZoom = view.getByRole("button", { name: /Reset zoom/ });
		await expect.element(zoomIn).not.toBeDisabled();
		await zoomIn.click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await expect.element(resetZoom).toHaveTextContent(/%/);
		expect(getComputedStyle(stage).cursor).toBe("grab");
		while (!(zoomIn.element() as HTMLButtonElement).disabled)
			await zoomIn.click();
		await expect.element(zoomIn).toBeDisabled();
		expect(zoomIn.element().getAttribute("disabled")).not.toBeNull();
		await expect.element(zoomOut).not.toBeDisabled();
		while (!(zoomOut.element() as HTMLButtonElement).disabled)
			await zoomOut.click();
		await expect.element(zoomOut).toBeDisabled();
		expect(zoomOut.element().getAttribute("disabled")).not.toBeNull();
		await zoomIn.click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		for (const button of [
			zoomIn.element(),
			zoomOut.element(),
			resetZoom.element(),
		]) {
			const rect = button.getBoundingClientRect();
			expect(rect.width).toBeGreaterThanOrEqual(44);
			expect(rect.height).toBeGreaterThanOrEqual(44);
		}

		const overlay = dialog.element();
		const capturedPointers = new Set<number>();
		const captured = new Map<number, boolean>();
		Object.defineProperty(overlay, "setPointerCapture", {
			configurable: true,
			value: (pointerId: number) => {
				capturedPointers.add(pointerId);
				captured.set(pointerId, true);
			},
		});
		Object.defineProperty(overlay, "hasPointerCapture", {
			configurable: true,
			value: (pointerId: number) => captured.has(pointerId),
		});
		Object.defineProperty(overlay, "releasePointerCapture", {
			configurable: true,
			value: (pointerId: number) => {
				captured.delete(pointerId);
			},
		});
		const layer = view.getByTestId("viewer-transform-layer").element();
		const drag = (type: string, x: number, y: number) =>
			overlay.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					button: 0,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId: 88,
					pointerType: "mouse",
				}),
			);
		const beforeDrag = layer.style.transform;
		drag("pointerdown", 600, 400);
		drag("pointermove", 630, 430);
		await expect.poll(() => getComputedStyle(stage).cursor).toBe("grabbing");
		await expect.poll(() => layer.style.transform).not.toBe(beforeDrag);
		expect(capturedPointers.has(88)).toBe(true);
		drag("pointerup", 630, 430);
		await expect.poll(() => getComputedStyle(stage).cursor).toBe("grab");
		expect(captured.has(88)).toBe(false);

		const ordinaryWheel = new WheelEvent("wheel", {
			bubbles: true,
			cancelable: true,
			deltaY: 30,
			clientX: 720,
			clientY: 320,
		});
		stage.dispatchEvent(ordinaryWheel);
		expect(ordinaryWheel.defaultPrevented).toBe(true);
		await resetZoom.click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		const fitWheel = new WheelEvent("wheel", {
			bubbles: true,
			cancelable: true,
			deltaY: 30,
			clientX: 720,
			clientY: 320,
		});
		stage.dispatchEvent(fitWheel);
		expect(fitWheel.defaultPrevented).toBe(false);
		const modifiedWheel = new WheelEvent("wheel", {
			bubbles: true,
			cancelable: true,
			ctrlKey: true,
			deltaY: -120,
			clientX: 720,
			clientY: 320,
		});
		stage.dispatchEvent(modifiedWheel);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		expect(modifiedWheel.defaultPrevented).toBe(true);

		await userEvent.keyboard("0");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		overlay.focus();
		await userEvent.keyboard("+");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await userEvent.keyboard("-");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		await userEvent.keyboard("=");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await userEvent.keyboard("0");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		stage.dispatchEvent(
			new MouseEvent("dblclick", {
				bubbles: true,
				clientX: 800,
				clientY: 400,
			}),
		);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		stage.dispatchEvent(
			new MouseEvent("dblclick", {
				bubbles: true,
				clientX: 800,
				clientY: 400,
			}),
		);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");

		const infoButton = view.getByRole("button", { name: "Photo information" });
		await infoButton.click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		const zoomCluster = overlay.querySelector<HTMLElement>(
			"[data-viewer-zoom-controls]",
		);
		const back = overlay.querySelector<HTMLElement>(
			"[data-viewer-chrome] button",
		);
		const info = overlay.querySelector<HTMLElement>("[data-viewer-info]");
		const boundsOverlap = (first: DOMRect, second: DOMRect) =>
			first.left < second.right &&
			first.right > second.left &&
			first.top < second.bottom &&
			first.bottom > second.top;
		const clusterBounds = zoomCluster?.getBoundingClientRect();
		if (!clusterBounds || !back || !info)
			throw new Error("viewer chrome bounds were not rendered");
		expect(boundsOverlap(clusterBounds, drawer.getBoundingClientRect())).toBe(
			false,
		);
		expect(boundsOverlap(clusterBounds, back.getBoundingClientRect())).toBe(
			false,
		);
		expect(boundsOverlap(clusterBounds, info.getBoundingClientRect())).toBe(
			false,
		);
		const editable = document.createElement("input");
		drawer.append(editable);
		editable.focus();
		editable.dispatchEvent(
			new KeyboardEvent("keydown", { bubbles: true, key: "=" }),
		);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");

		await view.getByRole("button", { name: "Close photo information" }).click();
		await zoomIn.click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await view.getByRole("button", { name: "Next photo" }).click();
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "photo-0");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		expect(overlay.querySelector("[data-viewer-navigator]")).toBeNull();
	});

	it("shows an interactive desktop navigator with the normalized viewport", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		const zoomIn = view.getByRole("button", { name: "Zoom in" });
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-viewer-mode", "fit");
		const dialog = view.getByRole("dialog", { name: "Photo viewer" }).element();
		expect(dialog.querySelector("[data-viewer-navigator]")).toBeNull();
		await zoomIn.click();
		const navigator = view
			.getByTestId("viewer-stage")
			.element()
			.closest("[role='dialog']")
			?.querySelector<HTMLElement>("[data-viewer-navigator]");
		if (!navigator) throw new Error("navigator was not rendered after zooming");
		expect(navigator.getAttribute("aria-hidden")).toBe("true");
		expect(navigator.getAttribute("role")).toBeNull();
		expect(getComputedStyle(navigator).pointerEvents).toBe("auto");
		expect(
			view
				.getByRole("img", {
					name: "Navigator for Coast",
					includeHidden: true,
				})
				.query(),
		).toBeNull();
		const viewport = navigator.querySelector<HTMLElement>(
			"[data-viewer-navigator-viewport]",
		);
		if (!viewport) throw new Error("navigator viewport was not rendered");
		expect(viewport.style.width).toMatch(/%$/);
		expect(viewport.style.height).toMatch(/%$/);
		const layer = view.getByTestId("viewer-transform-layer").element();
		const before = layer.style.transform;
		const bounds = navigator.getBoundingClientRect();
		navigator.dispatchEvent(
			new MouseEvent("click", {
				bubbles: true,
				clientX: bounds.right - 8,
				clientY: bounds.bottom - 8,
				detail: 1,
			}),
		);
		await expect.poll(() => layer.style.transform).not.toBe(before);
		const beforeDrag = layer.style.transform;
		const dragEvent = (type: string, x: number, y: number) =>
			navigator.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					button: 0,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId: 41,
					pointerType: "mouse",
				}),
			);
		dragEvent("pointerdown", bounds.right - 24, bounds.bottom - 24);
		dragEvent("pointermove", bounds.left + 24, bounds.top + 24);
		await expect.poll(() => layer.style.transform).not.toBe(beforeDrag);
		dragEvent("pointerup", bounds.right - 24, bounds.bottom - 24);
		await view.getByRole("button", { name: "Photo information" }).click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		const drawerNavigator = dialog.querySelector<HTMLElement>(
			"[data-viewer-navigator]",
		);
		if (!drawerNavigator)
			throw new Error("navigator disappeared beside drawer");
		expect(drawerNavigator.getBoundingClientRect().right).toBeLessThanOrEqual(
			drawer.getBoundingClientRect().left,
		);
		await view.unmount();
	});

	it("keeps the touch navigator display-only", async () => {
		await page.viewport(390, 844);
		const restorePointer = installPointerModality(true);
		const { view, tile } = await openAsset("Coast");
		try {
			(tile.element() as HTMLButtonElement).click();
			await view.getByRole("button", { name: "Zoom in" }).click();
			const navigator = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element()
				.querySelector<HTMLElement>("[data-viewer-navigator]");
			if (!navigator)
				throw new Error("navigator was not rendered after zooming");
			expect(navigator.getAttribute("aria-hidden")).toBe("true");
			expect(getComputedStyle(navigator).pointerEvents).toBe("none");
			const layer = view.getByTestId("viewer-transform-layer").element();
			const before = layer.style.transform;
			navigator.dispatchEvent(
				new MouseEvent("click", {
					bubbles: true,
					clientX: navigator.getBoundingClientRect().right - 8,
					clientY: navigator.getBoundingClientRect().bottom - 8,
					detail: 1,
				}),
			);
			expect(layer.style.transform).toBe(before);
			await view.unmount();
		} finally {
			restorePointer();
		}
	});

	it("keeps the navigator above controls in rotated landscape and beside the drawer", async () => {
		const root = document.documentElement;
		const previousSafeAreas = [
			"--safe-area-top",
			"--safe-area-right",
			"--safe-area-bottom",
			"--safe-area-left",
		].map(
			(property) => [property, root.style.getPropertyValue(property)] as const,
		);
		const safeArea = { top: 60, right: 68, bottom: 52, left: 72 };
		let view: Awaited<ReturnType<typeof renderViewerWall>> | null = null;
		try {
			await page.viewport(844, 390);
			for (const [property, value] of [
				["--safe-area-top", `${safeArea.top}px`],
				["--safe-area-right", `${safeArea.right}px`],
				["--safe-area-bottom", `${safeArea.bottom}px`],
				["--safe-area-left", `${safeArea.left}px`],
			] as const)
				root.style.setProperty(property, value);
			const opened = await openAsset("Coast");
			view = opened.view;
			(opened.tile.element() as HTMLButtonElement).click();
			const activeView = opened.view;
			await activeView.getByRole("button", { name: "Zoom in" }).click();
			const dialog = activeView
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			const navigator = dialog.querySelector<HTMLElement>(
				"[data-viewer-navigator]",
			);
			const previous = dialog.querySelector<HTMLElement>(
				"[data-viewer-controls] [aria-label='Previous photo']",
			);
			const next = dialog.querySelector<HTMLElement>(
				"[data-viewer-controls] [aria-label='Next photo']",
			);
			const back = dialog.querySelector<HTMLElement>(
				"[data-viewer-chrome] [aria-label='Back to photos']",
			);
			const info = dialog.querySelector<HTMLElement>("[data-viewer-info]");
			const zoom = dialog.querySelector<HTMLElement>(
				"[data-viewer-zoom-controls]",
			);
			const filmstrip = dialog.querySelector<HTMLElement>(
				"[aria-label='Photo filmstrip']",
			);
			if (
				!navigator ||
				!previous ||
				!next ||
				!back ||
				!info ||
				!zoom ||
				!filmstrip
			)
				throw new Error("landscape navigator controls were not rendered");
			const boundsOverlap = (first: DOMRect, second: DOMRect) =>
				first.left < second.right &&
				first.right > second.left &&
				first.top < second.bottom &&
				first.bottom > second.top;
			const assertNavigatorBounds = (drawer?: Element) => {
				const overlayBounds = dialog.getBoundingClientRect();
				const navigatorBounds = navigator.getBoundingClientRect();
				expect(navigatorBounds.left).toBeGreaterThanOrEqual(
					overlayBounds.left + safeArea.left,
				);
				expect(navigatorBounds.right).toBeLessThanOrEqual(
					overlayBounds.right - safeArea.right,
				);
				expect(navigatorBounds.top).toBeGreaterThanOrEqual(
					overlayBounds.top + safeArea.top,
				);
				expect(navigatorBounds.bottom).toBeLessThanOrEqual(
					overlayBounds.bottom - safeArea.bottom,
				);
				for (const [name, element] of [
					["Previous", previous],
					["Next", next],
					["Back", back],
					["Info", info],
					["zoom controls", zoom],
					["filmstrip", filmstrip],
					["open drawer", drawer],
				] as const) {
					if (!element) continue;
					expect(
						boundsOverlap(navigatorBounds, element.getBoundingClientRect()),
						`${name} overlaps the navigator`,
					).toBe(false);
				}
			};
			assertNavigatorBounds();
			await activeView
				.getByRole("button", { name: "Photo information" })
				.click();
			const drawer = activeView
				.getByRole("complementary", { name: "Photo information" })
				.element();
			assertNavigatorBounds(drawer);
		} finally {
			if (view) await view.unmount();
			for (const [property, value] of previousSafeAreas) {
				if (value) root.style.setProperty(property, value);
				else root.style.removeProperty(property);
			}
			await page.viewport(1440, 1024);
		}
	});

	it("uses coarse pointer modality for a wide touch viewport", async () => {
		await page.viewport(844, 390);
		const restorePointer = installPointerModality(true);
		try {
			const { view, tile } = await openAsset("Coast");
			(tile.element() as HTMLButtonElement).click();
			await view.getByRole("button", { name: "Zoom in" }).click();
			const navigator = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element()
				.querySelector<HTMLElement>("[data-viewer-navigator]");
			if (!navigator)
				throw new Error("navigator was not rendered after zooming");
			expect(navigator.getAttribute("aria-hidden")).toBe("true");
			const layer = view.getByTestId("viewer-transform-layer").element();
			const before = layer.style.transform;
			navigator.dispatchEvent(
				new MouseEvent("click", {
					bubbles: true,
					clientX: navigator.getBoundingClientRect().right - 8,
					clientY: navigator.getBoundingClientRect().bottom - 8,
					detail: 1,
				}),
			);
			expect(layer.style.transform).toBe(before);
			await view.unmount();
		} finally {
			restorePointer();
		}
	});

	it("keeps a narrow open drawer layout on-screen or hides an unreadable navigator", async () => {
		await page.viewport(640, 844);
		const wide = await openAsset("Coast");
		(wide.tile.element() as HTMLButtonElement).click();
		await wide.view.getByRole("button", { name: "Zoom in" }).click();
		await wide.view.getByRole("button", { name: "Photo information" }).click();
		const wideDialog = wide.view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const wideNavigator = wideDialog.querySelector<HTMLElement>(
			"[data-viewer-navigator]",
		);
		const wideDrawer = wide.view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		if (!wideNavigator) throw new Error("640px navigator was not rendered");
		const wideBounds = wideNavigator.getBoundingClientRect();
		const dialogBounds = wideDialog.getBoundingClientRect();
		const drawerBounds = wideDrawer.getBoundingClientRect();
		expect(wideBounds.left).toBeGreaterThanOrEqual(dialogBounds.left);
		expect(wideBounds.right).toBeLessThanOrEqual(drawerBounds.left);
		expect(wideBounds.top).toBeGreaterThanOrEqual(dialogBounds.top);
		expect(wideBounds.bottom).toBeLessThanOrEqual(dialogBounds.bottom);
		await wide.view.unmount();

		await page.viewport(639, 844);
		const narrow = await openAsset("Coast");
		(narrow.tile.element() as HTMLButtonElement).click();
		await narrow.view.getByRole("button", { name: "Zoom in" }).click();
		await narrow.view
			.getByRole("button", { name: "Photo information" })
			.click();
		const narrowNavigator = narrow.view
			.getByRole("dialog", { name: "Photo viewer" })
			.element()
			.querySelector<HTMLElement>("[data-viewer-navigator]");
		if (narrowNavigator)
			expect(getComputedStyle(narrowNavigator).display).toBe("none");
		await narrow.view.unmount();
	});

	it("hides the narrow drawer-covered zoom cluster and restores its tab order", async () => {
		await page.viewport(390, 844);
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Zoom in" }).click();
		const dialog = view.getByRole("dialog", { name: "Photo viewer" }).element();
		const zoomCluster = dialog.querySelector<HTMLElement>(
			"[data-viewer-zoom-controls]",
		);
		if (!zoomCluster) throw new Error("zoom cluster was not rendered");
		await view.getByRole("button", { name: "Photo information" }).click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		expect(getComputedStyle(zoomCluster).display).toBe("none");
		expect(
			[...zoomCluster.querySelectorAll<HTMLButtonElement>("button")].every(
				(button) => button.tabIndex < 0,
			),
		).toBe(true);
		expect(zoomCluster.getBoundingClientRect().right).toBeLessThanOrEqual(
			drawer.getBoundingClientRect().left,
		);
		await view.getByRole("button", { name: "Close photo information" }).click();
		await expect
			.poll(() => getComputedStyle(zoomCluster).display)
			.not.toBe("none");
		expect(
			[...zoomCluster.querySelectorAll<HTMLButtonElement>("button")].every(
				(button) => button.tabIndex === 0,
			),
		).toBe(true);
		expect(zoomCluster.getBoundingClientRect().right).toBeLessThanOrEqual(390);
		await view.unmount();
	});

	it("measures phone content bounds before fitting the navigator image", async () => {
		await page.viewport(390, 844);
		const restorePointer = installPointerModality(true);
		try {
			const { view, tile } = await openAsset("Coast");
			(tile.element() as HTMLButtonElement).click();
			await view.getByRole("button", { name: "Zoom in" }).click();
			const dialog = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			const navigator = dialog.querySelector<HTMLElement>(
				"[data-viewer-navigator]",
			);
			const imageFrame = navigator?.querySelector<HTMLElement>(
				"[class*='viewerNavigatorImage']",
			);
			const viewport = navigator?.querySelector<HTMLElement>(
				"[data-viewer-navigator-viewport]",
			);
			if (!navigator || !imageFrame || !viewport)
				throw new Error("phone navigator geometry was not rendered");
			const navigatorBounds = navigator.getBoundingClientRect();
			const imageBounds = imageFrame.getBoundingClientRect();
			expect(navigatorBounds.width).toBe(144);
			expect(navigatorBounds.height).toBe(88);
			expect(imageBounds.left - navigatorBounds.left).toBeCloseTo(7.5, 1);
			expect(imageBounds.top - navigatorBounds.top).toBeCloseTo(1, 1);
			expect(imageBounds.width).toBeCloseTo(129, 1);
			expect(imageBounds.height).toBeCloseTo(86, 1);
			expect(viewport.style.left).toBe("10%");
			expect(viewport.style.top).toBe("0%");
			expect(viewport.style.width).toBe("80%");
			expect(viewport.style.height).toBe("100%");
			await view.unmount();
		} finally {
			restorePointer();
		}
	});

	it("fades the mounted navigator after inactivity and restores it for stage manipulation", async () => {
		await page.viewport(1440, 1024);
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Zoom in" }).click();
		const dialog = view.getByRole("dialog", { name: "Photo viewer" }).element();
		const navigator = dialog.querySelector<HTMLElement>(
			"[data-viewer-navigator]",
		);
		const stage = view.getByTestId("viewer-stage").element();
		if (!navigator) throw new Error("navigator was not rendered after zooming");
		navigator.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				button: 0,
				clientX: 760,
				clientY: 120,
				isPrimary: true,
				pointerId: 78,
				pointerType: "mouse",
			}),
		);
		navigator.dispatchEvent(
			new PointerEvent("lostpointercapture", {
				bubbles: true,
				pointerId: 78,
				pointerType: "mouse",
			}),
		);
		await new Promise((resolve) => window.setTimeout(resolve, 2600));
		expect(getComputedStyle(navigator).opacity).toBe("0");
		expect(getComputedStyle(navigator).visibility).toBe(
			window.matchMedia("(prefers-reduced-motion: reduce)").matches
				? "hidden"
				: "visible",
		);
		expect(getComputedStyle(navigator).pointerEvents).toBe("none");
		stage.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				button: 0,
				clientX: 640,
				clientY: 480,
				isPrimary: true,
				pointerId: 77,
				pointerType: "mouse",
			}),
		);
		await expect.poll(() => getComputedStyle(navigator).opacity).toBe("1");
		stage.dispatchEvent(
			new PointerEvent("pointerup", {
				bubbles: true,
				button: 0,
				clientX: 640,
				clientY: 480,
				isPrimary: true,
				pointerId: 77,
				pointerType: "mouse",
			}),
		);
		await view.unmount();
	});

	it("suppresses navigator fade in reduced-motion mode", async () => {
		const view = await render(
			<>
				<ViewerNavigator
					assetName="Motion photo"
					imageHeight={800}
					imageUrl={null}
					imageWidth={1200}
					interactive
					onInteraction={() => undefined}
					onRecenter={() => undefined}
					visible={false}
					visibleRect={{ x: 0.25, y: 0.25, width: 0.5, height: 0.5 }}
				/>
				<ViewerStage
					asset={asset("reduced-motion", "Reduced motion", 1)}
					service={previewService(async () => undefined)}
					viewportHeight={400}
					viewportWidth={600}
				/>
			</>,
		);
		const navigator = view.getByTestId("viewer-navigator").element();
		expect(getComputedStyle(navigator).transitionDuration).toBe("0s");
		expect(getComputedStyle(navigator).opacity).toBe("0");
		expect(getComputedStyle(navigator).visibility).toBe("hidden");
		expect(getComputedStyle(navigator).pointerEvents).toBe("none");
		expect(
			getComputedStyle(view.getByTestId("viewer-transform-layer").element())
				.transitionDuration,
		).toBe("0s");
		await view.unmount();
	});

	it("keeps the thumbnail until the preview decodes and preserves the viewport", async () => {
		const originalDecode = HTMLImageElement.prototype.decode;
		let releaseDecode: (() => void) | null = null;
		HTMLImageElement.prototype.decode = function () {
			if (this.src.includes("data:image/svg+xml"))
				return new Promise<void>((resolve) => {
					releaseDecode = resolve;
				});
			return originalDecode.call(this);
		};
		const service = serviceWithReadyPhotos(false, 1, true);
		const originalDerivativeUrl = service.derivativeUrl.bind(service);
		service.derivativeUrl = (reference) =>
			reference.kind === "screenPreview"
				? "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='2400' height='1600'%3E%3Crect width='100%25' height='100%25' fill='teal'/%3E%3C/svg%3E"
				: originalDerivativeUrl(reference);
		try {
			const { view } = await openAssetWithService(service, "Coast");
			await view.getByRole("button", { name: "Zoom in" }).click();
			const dialog = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			const navigator = dialog.querySelector<HTMLElement>(
				"[data-viewer-navigator]",
			);
			if (!navigator)
				throw new Error("navigator was not rendered after zooming");
			const viewport = navigator.querySelector<HTMLElement>(
				"[data-viewer-navigator-viewport]",
			);
			const image = navigator.querySelector<HTMLImageElement>("img");
			if (!viewport || !image)
				throw new Error("navigator image was not rendered");
			expect(image.dataset.viewerNavigatorLayer).toBe("wallThumbnail");
			const before = {
				left: viewport.style.left,
				top: viewport.style.top,
				width: viewport.style.width,
				height: viewport.style.height,
				bounds: viewport.getBoundingClientRect().toJSON(),
			};
			const release = releaseDecode as (() => void) | null;
			if (!release) throw new Error("preview decode did not start");
			release();
			await expect
				.poll(
					() =>
						navigator.querySelector<HTMLImageElement>("img")?.dataset
							.viewerNavigatorLayer ?? "",
				)
				.toBe("screenPreview");
			expect({
				left: viewport.style.left,
				top: viewport.style.top,
				width: viewport.style.width,
				height: viewport.style.height,
				bounds: viewport.getBoundingClientRect().toJSON(),
			}).toEqual(before);
			await view.unmount();
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("recovers the thumbnail when the navigator preview decode fails", async () => {
		const originalDecode = HTMLImageElement.prototype.decode;
		let rejectDecode: ((reason?: unknown) => void) | null = null;
		HTMLImageElement.prototype.decode = function () {
			if (this.src.includes("missing.jpg"))
				return new Promise<void>((_, reject) => {
					rejectDecode = reject;
				});
			return originalDecode.call(this);
		};
		try {
			const { view, tile } = await openAsset("Coast", true);
			(tile.element() as HTMLButtonElement).click();
			await view.getByRole("button", { name: "Zoom in" }).click();
			const navigator = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element()
				.querySelector<HTMLElement>("[data-viewer-navigator]");
			if (!navigator)
				throw new Error("navigator was not rendered after zooming");
			const image = navigator.querySelector<HTMLImageElement>("img");
			const viewport = navigator.querySelector<HTMLElement>(
				"[data-viewer-navigator-viewport]",
			);
			if (!image || !viewport)
				throw new Error("navigator image was not rendered");
			expect(image.dataset.viewerNavigatorLayer).toBe("wallThumbnail");
			const before = {
				left: viewport.style.left,
				top: viewport.style.top,
				width: viewport.style.width,
				height: viewport.style.height,
				bounds: viewport.getBoundingClientRect().toJSON(),
			};
			const reject = rejectDecode as ((reason?: unknown) => void) | null;
			if (!reject) throw new Error("preview decode did not start");
			reject(new Error("preview unavailable"));
			await expect
				.poll(() => navigator.dataset.viewerNavigatorDecodeStatus ?? "")
				.toBe("failed");
			expect(image.dataset.viewerNavigatorLayer).toBe("wallThumbnail");
			expect({
				left: viewport.style.left,
				top: viewport.style.top,
				width: viewport.style.width,
				height: viewport.style.height,
				bounds: viewport.getBoundingClientRect().toJSON(),
			}).toEqual(before);
			await view.unmount();
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("ends a zoomed mouse drag exactly once for every lifecycle cancellation", async () => {
		for (const action of [
			"gesture-asset-revision",
			"gesture-viewport-revision",
			"gesture-close",
		] as const) {
			const stats = { panEnds: 0 };
			const view = await render(<GestureLifecycleHarness stats={stats} />);
			const target = view.getByTestId("gesture-lifecycle-target").element();
			target.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					button: 0,
					clientX: 100,
					clientY: 200,
					isPrimary: true,
					pointerId: 91,
					pointerType: "mouse",
				}),
			);
			await view.getByTestId(action).click();
			await expect.poll(() => stats.panEnds).toBe(1);
			target.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					button: 0,
					clientX: 120,
					clientY: 220,
					isPrimary: true,
					pointerId: 91,
					pointerType: "mouse",
				}),
			);
			expect(stats.panEnds).toBe(1);
			await view.unmount();
		}

		const stats = { panEnds: 0 };
		const view = await render(<GestureLifecycleHarness stats={stats} />);
		const target = view.getByTestId("gesture-lifecycle-target").element();
		target.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				button: 0,
				clientX: 100,
				clientY: 200,
				isPrimary: true,
				pointerId: 92,
				pointerType: "mouse",
			}),
		);
		await view.unmount();
		expect(stats.panEnds).toBe(1);
	});

	it("ends an active navigator drag on every mounted lifecycle boundary", async () => {
		const view = await render(<NavigatorLifecycleHarness />);
		const beginDrag = () => {
			const navigator = view
				.getByTestId("navigator-lifecycle-end-count")
				.element()
				.parentElement?.querySelector<HTMLElement>("[data-viewer-navigator]");
			if (!navigator) throw new Error("navigator was not mounted");
			navigator.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					button: 0,
					clientX: 100,
					clientY: 100,
					isPrimary: true,
					pointerId: 701,
					pointerType: "mouse",
				}),
			);
		};
		for (const action of [
			"navigator-lifecycle-asset",
			"navigator-lifecycle-viewport",
			"navigator-lifecycle-modality",
		] as const) {
			beginDrag();
			const before = Number(
				view.getByTestId("navigator-lifecycle-end-count").element().textContent,
			);
			await view.getByTestId(action).click();
			await expect
				.poll(() =>
					Number(
						view.getByTestId("navigator-lifecycle-end-count").element()
							.textContent,
					),
				)
				.toBe(before + 1);
		}
		await view.getByTestId("navigator-lifecycle-modality").click();
		beginDrag();
		await view.getByTestId("navigator-lifecycle-unmount").click();
		await expect
			.poll(() =>
				Number(
					view.getByTestId("navigator-lifecycle-end-count").element()
						.textContent,
				),
			)
			.toBe(4);
		await view.unmount();
	});

	it("opens the selected tile above the mounted wall and returns to its exact position", async () => {
		const { view, tile } = await openAsset("Coast");
		const wall = view.getByRole("region", { name: "Photos" });
		wall.element().scrollTop = 420;

		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		expect(document.querySelector("[data-testid='photo-wall']")).not.toBeNull();

		await view.getByRole("button", { name: "Back to photos" }).click();
		expect(wall.element().scrollTop).toBe(420);
		await expect.element(tile).toHaveFocus();
	});

	it("keeps indexed videos poster-only and non-openable", async () => {
		const video = {
			...asset("clip", "Clip", 1),
			mediaKind: "video" as const,
		};
		const service = createInMemoryPhotoService({
			selectedFolderName: "Video fixture",
			wallAssets: [
				{
					...video,
					wallThumbnailUrl: "/demo-photos/coast.jpg",
				},
			],
		});
		const view = await renderViewerWall(service);
		await view.getByRole("button", { name: "Choose Folder" }).click();
		await service.finishFixtureScan();
		expect(view.getByText("Video · poster only").query()).toBeNull();
		expect(document.querySelector("figcaption")).toBeNull();
		expect(
			view.getByRole("button", { name: "Open Clip", exact: true }).query(),
		).toBeNull();
	});

	it("returns focus and highlight to the asset viewed when closing", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await view.getByRole("button", { name: "Back to photos" }).click();
		const viewedTile = view.getByRole("button", {
			name: "Open Photo 0",
			exact: true,
		});
		await expect.element(viewedTile).toHaveFocus();
		await expect
			.poll(() =>
				viewedTile.element().className.includes("tileReturnHighlight"),
			)
			.toBe(true);
		expect(
			view.getByRole("region", { name: "Photos" }).element().scrollTop,
		).toBe(0);
		void tile;
	});

	it("uses the wall thumbnail for the first frame", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const image = document.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		expect(image?.src).toContain("/demo-photos/coast.jpg");
		await expect
			.poll(() => (image?.complete ? image.naturalWidth : 0))
			.toBeGreaterThan(0);
	});

	it("requests the visible photo as a screen preview", async () => {
		const { service, view, tile } = await openAsset("Coast");
		const visibleCurrentRequests = () =>
			service.derivativeRequests.filter(
				(request) =>
					request.kind === "screenPreview" &&
					request.priority === "visible" &&
					request.assetIds.length === 1 &&
					request.assetIds[0] === "coast",
			).length;
		expect(visibleCurrentRequests()).toBe(0);
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect.poll(visibleCurrentRequests).toBe(1);
		expect(visibleCurrentRequests()).toBe(1);
	});

	it("deduplicates the current preview across StrictMode effect replay", async () => {
		const service = serviceWithReadyPhotos(false, 1, false);
		const currentRequests: DerivativeRequest[] = [];
		const releaseFirst = gate<void>();
		const firstComplete = gate<void>();
		let holdFirst = true;
		const originalRequestDerivatives = service.requestDerivatives.bind(service);
		service.requestDerivatives = async (request) => {
			const isCurrent =
				request.kind === "screenPreview" &&
				request.priority === "visible" &&
				request.assetIds.length === 1 &&
				request.assetIds[0] === "coast";
			if (isCurrent) {
				currentRequests.push({ ...request, assetIds: [...request.assetIds] });
				if (holdFirst) {
					holdFirst = false;
					await releaseFirst.promise;
				}
			}
			await originalRequestDerivatives(request);
			if (isCurrent && currentRequests.length === 1) firstComplete.resolve();
		};

		const visibleCurrentRequests = () =>
			currentRequests.filter(
				(request) =>
					request.kind === "screenPreview" &&
					request.priority === "visible" &&
					request.assetIds.length === 1 &&
					request.assetIds[0] === "coast",
			).length;
		const view = await renderStrictViewerWall(service);
		await view.getByRole("button", { name: "Choose Folder" }).click();
		await service.finishFixtureScan();
		const tile = view.getByRole("button", { name: "Open Coast", exact: true });
		await expect.element(tile).toBeVisible();
		expect(visibleCurrentRequests()).toBe(0);

		await tile.click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect.poll(visibleCurrentRequests).toBe(1);
		expect(visibleCurrentRequests()).toBe(1);
		releaseFirst.resolve();
		await firstComplete.promise;
		expect(visibleCurrentRequests()).toBe(1);

		await view.getByRole("button", { name: "Back to photos" }).click();
		await expect.element(tile).toBeVisible();
		await tile.click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect.poll(visibleCurrentRequests).toBe(2);
		expect(visibleCurrentRequests()).toBe(2);
		await view.unmount();
	});

	it("does not request screen previews for a defensive video asset", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
		});
		const video = {
			...asset("clip", "Clip", 1),
			mediaKind: "video" as const,
			screenPreview: {
				assetId: "clip",
				kind: "screenPreview" as const,
				key: "clip-screen",
			},
		};
		const view = await render(
			<PhotoServiceProvider service={service}>
				<PreviewHarness assets={[video]} service={service} />
			</PhotoServiceProvider>,
		);
		await new Promise((resolve) => window.setTimeout(resolve, 25));
		expect(requests).toHaveLength(0);
		expect(
			view.getByTestId("viewer-stage").element().querySelector("img"),
		).toBeNull();
		await view.unmount();
	});

	it("prioritizes current preview work before immediate neighbours", async () => {
		const service = serviceWithReadyPhotos(false, 2);
		const view = await renderViewerWall(service);
		await view.getByRole("button", { name: "Choose Folder" }).click();
		await service.finishFixtureScan();
		const tile = view.getByRole("button", { name: "Open Coast", exact: true });
		await expect.element(tile).toBeVisible();
		service.derivativeRequests.length = 0;
		(tile.element() as HTMLButtonElement).click();
		await expect
			.poll(() =>
				service.derivativeRequests.some(
					(request) =>
						request.kind === "screenPreview" &&
						request.priority === "visible" &&
						request.assetIds.includes("coast"),
				),
			)
			.toBe(true);
		const screenRequests = service.derivativeRequests.filter(
			(request) => request.kind === "screenPreview",
		);
		const currentRequest = screenRequests.findIndex(
			(request) =>
				request.priority === "visible" && request.assetIds.includes("coast"),
		);
		const neighbourRequest = screenRequests.findIndex(
			(request) =>
				request.priority === "nearViewport" &&
				request.assetIds.includes("photo-0"),
		);
		expect(currentRequest).toBeGreaterThanOrEqual(0);
		expect(neighbourRequest).toBeGreaterThan(currentRequest);
	});

	it("keeps the open viewer and information drawer free of serious violations", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		expect(seriousViolations(await axe.run(document))).toEqual([]);

		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		expect(seriousViolations(await axe.run(document))).toEqual([]);
	});

	it("announces discrete zoom labels through the polite viewer status", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		const status = view.getByTestId("viewer-status");
		await expect.element(status).toHaveAttribute("aria-live", "polite");
		await expect.element(status).toHaveTextContent("Fit");
		const mutations: MutationRecord[] = [];
		const observer = new MutationObserver((records) =>
			mutations.push(...records),
		);
		observer.observe(status.element(), {
			characterData: true,
			childList: true,
			subtree: true,
		});
		try {
			await view.getByRole("button", { name: "Zoom in" }).click();
			await expect.element(status).toHaveTextContent(/\d+%/);
			await new Promise((resolve) => window.setTimeout(resolve, 0));
			expect(mutations.length).toBeGreaterThan(0);
			mutations.splice(0, mutations.length);
			await view.getByRole("button", { name: "Reset zoom" }).click();
			await expect.element(status).toHaveTextContent("Fit");
			await new Promise((resolve) => window.setTimeout(resolve, 0));
			expect(mutations.length).toBeGreaterThan(0);
		} finally {
			observer.disconnect();
			await view.unmount();
		}
	});

	it("keeps continuous zoom and pan frames out of the polite live region", async () => {
		await page.viewport(800, 600);
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		await view.getByRole("button", { name: "Zoom in" }).click();
		await view.getByRole("button", { name: "Reset zoom" }).click();
		const status = view.getByTestId("viewer-status").element();
		const mutations: MutationRecord[] = [];
		const observer = new MutationObserver((records) =>
			mutations.push(...records),
		);
		observer.observe(status, {
			characterData: true,
			childList: true,
			subtree: true,
		});
		const flush = () =>
			new Promise<void>((resolve) => window.setTimeout(resolve, 0));
		const clearMutations = () => {
			mutations.splice(0, mutations.length);
		};
		try {
			const modifiedWheel = new WheelEvent("wheel", {
				bubbles: true,
				cancelable: true,
				ctrlKey: true,
				deltaY: -24,
				clientX: 720,
				clientY: 400,
			});
			stage.dispatchEvent(modifiedWheel);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			await flush();
			expect(mutations).toHaveLength(0);

			await view.getByRole("button", { name: "Reset zoom" }).click();
			await flush();
			clearMutations();
			stage.dispatchEvent(
				new MouseEvent("dblclick", {
					bubbles: true,
					clientX: 400,
					clientY: 300,
				}),
			);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			await flush();
			expect(mutations.length).toBeGreaterThan(0);
			expect(status.textContent).toMatch(/\d+%/);
			clearMutations();
			stage.dispatchEvent(
				new MouseEvent("dblclick", {
					bubbles: true,
					clientX: 400,
					clientY: 300,
				}),
			);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
			await flush();
			expect(mutations.length).toBeGreaterThan(0);
			expect(status.textContent).toContain("Fit");

			await view.getByRole("button", { name: "Reset zoom" }).click();
			clearMutations();
			const touch = (type: string, pointerId: number, x: number, y: number) =>
				overlay.dispatchEvent(
					new PointerEvent(type, {
						bubbles: true,
						cancelable: true,
						clientX: x,
						clientY: y,
						isPrimary: pointerId === 601,
						pointerId,
						pointerType: "touch",
					}),
				);
			touch("pointerdown", 601, 560, 400);
			touch("pointerdown", 602, 680, 400);
			touch("pointermove", 601, 520, 400);
			touch("pointermove", 602, 720, 400);
			touch("pointerup", 601, 520, 400);
			touch("pointerup", 602, 720, 400);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			await flush();
			expect(mutations).toHaveLength(0);

			clearMutations();
			touch("pointerdown", 603, 720, 400);
			touch("pointermove", 603, 760, 430);
			touch("pointerup", 603, 760, 430);
			await flush();
			expect(mutations).toHaveLength(0);

			const navigator = overlay.querySelector<HTMLElement>(
				"[data-viewer-navigator]",
			);
			if (!navigator) throw new Error("navigator was not rendered");
			clearMutations();
			navigator.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					button: 0,
					clientX: 760,
					clientY: 120,
					isPrimary: true,
					pointerId: 604,
					pointerType: "mouse",
				}),
			);
			navigator.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					button: 0,
					clientX: 700,
					clientY: 150,
					isPrimary: true,
					pointerId: 604,
					pointerType: "mouse",
				}),
			);
			navigator.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					button: 0,
					clientX: 700,
					clientY: 150,
					isPrimary: true,
					pointerId: 604,
					pointerType: "mouse",
				}),
			);
			await flush();
			expect(mutations).toHaveLength(0);

			await view.getByRole("button", { name: "Reset zoom" }).click();
			await flush();
			clearMutations();
			overlay.focus();
			const discrete = async (key: string, expected: RegExp | string) => {
				clearMutations();
				overlay.dispatchEvent(
					new KeyboardEvent("keydown", {
						bubbles: true,
						cancelable: true,
						key,
					}),
				);
				await flush();
				expect(
					mutations.length,
					`${key} did not mutate status: ${status.textContent}`,
				).toBeGreaterThan(0);
				expect(status.textContent).toMatch(
					typeof expected === "string" ? new RegExp(expected) : expected,
				);
			};
			await discrete("+", /\d+%/);
			await discrete("=", /\d+%/);
			await discrete("-", /\d+%/);
			await discrete("0", /Fit/);
		} finally {
			observer.disconnect();
			await view.unmount();
		}
	});

	it("announces a discrete reset after continuous zoom even when the label stays Fit", async () => {
		await page.viewport(800, 600);
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await view.getByRole("button", { name: "Zoom in" }).click();
		await view.getByRole("button", { name: "Reset zoom" }).click();
		const stage = view.getByTestId("viewer-stage").element();
		const status = view.getByTestId("viewer-status").element();
		const mutations: MutationRecord[] = [];
		const observer = new MutationObserver((records) =>
			mutations.push(...records),
		);
		observer.observe(status, {
			characterData: true,
			childList: true,
			subtree: true,
		});
		try {
			stage.dispatchEvent(
				new WheelEvent("wheel", {
					bubbles: true,
					cancelable: true,
					ctrlKey: true,
					deltaY: -24,
					clientX: 720,
					clientY: 400,
				}),
			);
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-viewer-mode", "zoomed");
			await new Promise((resolve) => window.setTimeout(resolve, 0));
			expect(status.textContent).toContain("Fit");
			expect(mutations).toHaveLength(0);

			await view.getByRole("button", { name: "Reset zoom" }).click();
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-viewer-mode", "fit");
			await new Promise((resolve) => window.setTimeout(resolve, 0));
			expect(mutations.length).toBeGreaterThan(0);
		} finally {
			observer.disconnect();
			await view.unmount();
		}
	});

	it("audits every open viewer accessibility state without navigator focus traps", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const dialog = view.getByRole("dialog", { name: "Photo viewer" }).element();
		const runAxe = async () => {
			expect(seriousViolations(await axe.run(document))).toEqual([]);
		};
		await runAxe();
		await view.getByRole("button", { name: "Zoom in" }).click();
		const navigator = dialog.querySelector<HTMLElement>(
			"[data-viewer-navigator]",
		);
		if (!navigator) throw new Error("zoomed viewer navigator was not rendered");
		expect(navigator.tabIndex).toBe(-1);
		expect(navigator.getAttribute("aria-hidden")).toBe("true");
		await runAxe();

		vi.useFakeTimers();
		try {
			dialog.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			await expect
				.poll(() =>
					dialog
						.querySelector("[data-viewer-controls]")
						?.getAttribute("aria-hidden"),
				)
				.toBe("true");
			vi.useRealTimers();
			await runAxe();
			dialog.focus();
			const tab = new KeyboardEvent("keydown", {
				bubbles: true,
				cancelable: true,
				key: "Tab",
			});
			dialog.dispatchEvent(tab);
			expect(tab.defaultPrevented).toBe(true);
			expect(document.activeElement).toBe(dialog);
			dialog.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await expect
				.poll(() =>
					dialog
						.querySelector("[data-viewer-controls]")
						?.getAttribute("aria-hidden"),
				)
				.toBe("false");
			await view.getByRole("button", { name: "Photo information" }).click();
			await expect
				.element(view.getByRole("complementary", { name: "Photo information" }))
				.toBeVisible();
			await runAxe();
		} finally {
			vi.useRealTimers();
		}
		await view.unmount();

		await page.viewport(844, 390);
		const restorePointer = installPointerModality(true);
		try {
			const phone = await openAsset("Coast");
			(phone.tile.element() as HTMLButtonElement).click();
			await expect
				.element(phone.view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			await phone.view.getByRole("button", { name: "Zoom in" }).click();
			const phoneNavigator = phone.view
				.getByRole("dialog", { name: "Photo viewer" })
				.element()
				.querySelector<HTMLElement>("[data-viewer-navigator]");
			if (!phoneNavigator)
				throw new Error("rotated phone navigator was not rendered");
			expect(phoneNavigator.getAttribute("aria-hidden")).toBe("true");
			expect(phoneNavigator.tabIndex).toBe(-1);
			await runAxe();
			await phone.view.unmount();
		} finally {
			restorePointer();
		}
	});

	it("keeps malformed geometry and zero-natural previews at Fit while navigation works", async () => {
		const view = await render(
			<PhotoServiceProvider service={invalidGeometryService}>
				<InvalidGeometryViewerHarness />
			</PhotoServiceProvider>,
		);
		const stage = view.getByTestId("viewer-stage");
		const zoomIn = view.getByRole("button", { name: "Zoom in" });
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		await expect.element(zoomIn).toBeDisabled();
		await view.getByRole("button", { name: "Next photo" }).click();
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "zero-natural");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		await expect.element(zoomIn).toBeDisabled();
		expect(
			stage
				.element()
				.querySelector<HTMLImageElement>("[data-viewer-layer='screenPreview']")
				?.naturalWidth ?? 0,
		).toBe(0);
		await expect
			.element(view.getByTestId("viewer-status"))
			.toHaveTextContent("Zero natural, photo 2 of 2, Fit");
		await view.getByRole("button", { name: "Previous photo" }).click();
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "malformed");
	});

	it("exposes current selection and disabled directions at both boundaries", async () => {
		const { view, tile } = await openAsset("Coast", false, false, 1);
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const previous = view.getByRole("button", { name: "Previous photo" });
		const next = view.getByRole("button", { name: "Next photo" });
		const filmstrip = view.getByRole("group", { name: "Photo filmstrip" });
		const current = filmstrip
			.element()
			.querySelector<HTMLButtonElement>("button[aria-current='true']");
		expect(current?.getAttribute("aria-label")).toBe("Coast");
		expect(previous.element()).toBeDisabled();
		expect(next.element()).not.toBeDisabled();

		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-status"))
			.toHaveTextContent("Photo 0, photo 2 of 2");
		const nextButton = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element()
			.querySelector<HTMLButtonElement>("button[aria-label='Next photo']");
		const previousButton = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element()
			.querySelector<HTMLButtonElement>("button[aria-label='Previous photo']");
		expect(nextButton?.disabled).toBe(true);
		expect(previousButton?.disabled).toBe(false);
		expect(seriousViolations(await axe.run(document))).toEqual([]);
	});

	it("keeps every phone viewer target at least 44 pixels", async () => {
		await page.viewport(390, 844);
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const targets = overlay.querySelectorAll<HTMLButtonElement>("button");
		for (const target of targets) {
			const bounds = target.getBoundingClientRect();
			expect(bounds.width).toBeGreaterThanOrEqual(44);
			expect(bounds.height).toBeGreaterThanOrEqual(44);
		}
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		const close = view
			.getByRole("button", { name: "Close photo information" })
			.element();
		expect(close.getBoundingClientRect().width).toBeGreaterThanOrEqual(44);
		expect(close.getBoundingClientRect().height).toBeGreaterThanOrEqual(44);
	});

	it("keeps the information drawer close target inside rotated safe areas", async () => {
		const root = document.documentElement;
		const previousSafeAreas = [
			"--safe-area-top",
			"--safe-area-right",
			"--safe-area-bottom",
			"--safe-area-left",
		].map(
			(property) => [property, root.style.getPropertyValue(property)] as const,
		);
		const safeArea = { top: 60, right: 68, bottom: 52, left: 72 };
		let restoreViewport: (() => void) | null = null;
		let view: Awaited<ReturnType<typeof renderViewerWall>> | null = null;
		try {
			await page.viewport(390, 844);
			restoreViewport = installVisualViewportDouble(390, 844);
			for (const [property, value] of [
				["--safe-area-top", `${safeArea.top}px`],
				["--safe-area-right", `${safeArea.right}px`],
				["--safe-area-bottom", `${safeArea.bottom}px`],
				["--safe-area-left", `${safeArea.left}px`],
			] as const)
				root.style.setProperty(property, value);
			const opened = await openAsset("Coast");
			view = opened.view;
			(opened.tile.element() as HTMLButtonElement).click();
			const activeView = opened.view;
			await activeView
				.getByRole("button", { name: "Photo information" })
				.click();
			const overlay = activeView
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			const drawer = activeView
				.getByRole("complementary", { name: "Photo information" })
				.element();
			const assertDrawerBounds = () => {
				const overlayBounds = overlay.getBoundingClientRect();
				const drawerBounds = drawer.getBoundingClientRect();
				const closeBounds = activeView
					.getByRole("button", { name: "Close photo information" })
					.element()
					.getBoundingClientRect();
				expect(drawerBounds.left).toBeGreaterThanOrEqual(
					overlayBounds.left + safeArea.left - 1,
				);
				expect(drawerBounds.right).toBeLessThanOrEqual(
					overlayBounds.right - safeArea.right + 1,
				);
				expect(closeBounds.right).toBeLessThanOrEqual(
					drawerBounds.right - 8 + 1,
				);
				expect(closeBounds.width).toBeGreaterThanOrEqual(44);
			};
			assertDrawerBounds();
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			await page.viewport(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			window.dispatchEvent(new Event("orientationchange"));
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			await expect
				.poll(() => Number(overlay.dataset.viewportRevision))
				.toBeGreaterThan(0);
			assertDrawerBounds();
		} finally {
			if (view) await view.unmount();
			for (const [property, value] of previousSafeAreas) {
				if (value) root.style.setProperty(property, value);
				else root.style.removeProperty(property);
			}
			restoreViewport?.();
			await page.viewport(1440, 1024);
		}
	});

	it("keeps ready derivative URLs usable after the source becomes unavailable", async () => {
		const offline = cachedOfflineService();
		offline.prime();
		const { view } = await openAssetWithService(offline.service, "Coast");
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage");
		await expect
			.poll(() =>
				overlay.querySelector<HTMLImageElement>(
					"[data-viewer-layer='screenPreview'][data-ready='true']",
				),
			)
			.not.toBeNull();
		expect(
			overlay.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview'][data-ready='true']",
			)?.src,
		).toBe(offline.cachedScreenUrl);
		offline.goOffline();
		expect(() =>
			offline.service.derivativeUrl({
				assetId: "uncached",
				kind: "screenPreview",
				key: "uncached-screen",
			}),
		).toThrow(offline.sentinel);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "coast");
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-viewer-mode", "zoomed");
		expect(
			overlay.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview'][data-ready='true']",
			)?.src,
		).toBe(offline.cachedScreenUrl);
		expect(document.body.textContent).not.toContain(offline.sentinel);
		expect(overlay.innerHTML).not.toContain(offline.sentinel);
		await view.getByRole("button", { name: "Reset zoom" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-viewer-mode", "fit");
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		const neighbour = overlay.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		await expect
			.poll(() => (neighbour?.complete ? neighbour.naturalWidth : 0))
			.toBeGreaterThan(0);
		expect(neighbour?.src).toBe(offline.cachedNeighbourUrl);
		expect(neighbour?.src).not.toContain(offline.sentinel);
		expect(document.body.textContent).not.toContain(offline.sentinel);
		expect(overlay.innerHTML).not.toContain(offline.sentinel);
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		expect(neighbour?.src).toBe(offline.cachedNeighbourUrl);
		expect(document.body.textContent).not.toContain(offline.sentinel);
		expect(overlay.innerHTML).not.toContain(offline.sentinel);
	});

	it("marks unavailable larger previews and suppresses reduced-motion transitions", async () => {
		const { view, tile } = await openAsset("Coast", true);
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const stage = view.getByTestId("viewer-stage").element();
		await expect
			.poll(() => stage.getAttribute("data-large-preview-unavailable"))
			.toBe("true");
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByText("Larger preview unavailable"))
			.toBeVisible();
		expect(
			getComputedStyle(
				stage.querySelector("[data-viewer-layer='screenPreview']") ?? stage,
			).transitionDuration,
		).toBe("0s");
	});

	it("surfaces backend preview rejection immediately and caps retries", async () => {
		const service = serviceWithReadyPhotos(false, 1);
		service.requestDerivatives = async (request) => {
			service.derivativeRequests.push({
				...request,
				assetIds: [...request.assetIds],
			});
			throw new Error("preview generation failed");
		};
		const { view } = await openAssetWithService(service, "Coast");
		await expect
			.poll(
				() =>
					service.derivativeRequests.filter(
						(request) =>
							request.kind === "screenPreview" &&
							request.priority === "visible" &&
							request.assetIds.includes("coast"),
					).length,
			)
			.toBeGreaterThan(0);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-large-preview-unavailable", "true");
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByText("Larger preview unavailable"))
			.toBeVisible();
		await new Promise((resolve) => setTimeout(resolve, 500));
		expect(
			service.derivativeRequests.filter(
				(request) =>
					request.kind === "screenPreview" &&
					request.priority === "visible" &&
					request.assetIds.includes("coast"),
			).length,
		).toBeLessThanOrEqual(3);
	});

	it("navigates with a horizontal touch swipe and toggles chrome on a tap", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const dispatchTouch = (type: string, clientX: number, clientY: number) =>
			overlay.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX,
					clientY,
					isPrimary: true,
					pointerId: 7,
					pointerType: "touch",
				}),
			);
		dispatchTouch("pointerdown", 620, 400);
		dispatchTouch("pointerup", 540, 410);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const before = controls?.getAttribute("aria-hidden");
		dispatchTouch("pointerdown", 620, 400);
		dispatchTouch("pointerup", 620, 400);
		await expect
			.poll(() => controls?.getAttribute("aria-hidden"))
			.not.toBe(before);
	});

	it("pinches around a midpoint and pans a zoomed image without navigating", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		// Prime natural-size measurement so the pinch starts from a real fit state.
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await view.getByRole("button", { name: "Reset zoom" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		expect(getComputedStyle(stage).touchAction).toBe("pan-y");
		const dispatch = (
			type: string,
			pointerId: number,
			clientX: number,
			clientY: number,
		) => {
			const event = new PointerEvent(type, {
				bubbles: true,
				cancelable: true,
				clientX,
				clientY,
				isPrimary: pointerId === 101,
				pointerId,
				pointerType: "touch",
			});
			overlay.dispatchEvent(event);
			return event;
		};
		dispatch("pointerdown", 101, 520, 400);
		dispatch("pointerdown", 102, 640, 400);
		dispatch("pointermove", 101, 517, 400);
		const pinchMove = dispatch("pointermove", 102, 643, 400);
		dispatch("pointermove", 101, 514, 400);
		dispatch("pointermove", 102, 646, 400);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		expect(getComputedStyle(stage).touchAction).toBe("none");
		expect(pinchMove.defaultPrevented).toBe(true);
		expect(
			stage.querySelector<HTMLElement>("[data-testid='viewer-transform-layer']")
				?.style.transform,
		).toContain("scale(1.1)");
		expect(
			stage.querySelector<HTMLElement>("[data-testid='viewer-transform-layer']")
				?.style.transform,
		).not.toMatch(/translate3d\(0px, 0px/);
		expect(
			stage.querySelector<HTMLElement>("[data-testid='viewer-transform-layer']")
				?.style.transform,
		).toContain("translate3d(14.076789px, 8.195122px");
		const zoomedTransform = stage.querySelector<HTMLElement>(
			"[data-testid='viewer-transform-layer']",
		)?.style.transform;
		dispatch("pointerup", 101, 508, 400);
		dispatch("pointerup", 102, 652, 400);

		dispatch("pointerdown", 103, 720, 400);
		dispatch("pointermove", 103, 760, 430);
		dispatch("pointerup", 103, 760, 430);
		await expect.element(stage).toHaveAttribute("data-current-asset", "coast");
		expect(
			stage.querySelector<HTMLElement>("[data-testid='viewer-transform-layer']")
				?.style.transform,
		).not.toBe(zoomedTransform);

		dispatch("pointerdown", 201, 654, 400);
		dispatch("pointerdown", 202, 786, 400);
		dispatch("pointermove", 201, 657, 400);
		dispatch("pointermove", 202, 783, 400);
		dispatch("pointermove", 201, 660, 400);
		dispatch("pointermove", 202, 780, 400);
		dispatch("pointerup", 201, 660, 400);
		dispatch("pointerup", 202, 780, 400);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
	});

	it("delays a tap and turns two nearby taps into one zoom without chrome flicker", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const before = controls?.getAttribute("aria-hidden");
		vi.useFakeTimers();
		overlay.dispatchEvent(
			new PointerEvent("pointermove", {
				bubbles: true,
				clientX: 720,
				clientY: 400,
				pointerType: "mouse",
			}),
		);
		await vi.advanceTimersByTimeAsync(2500);
		await expect.poll(() => controls?.getAttribute("aria-hidden")).toBe("true");
		const tap = (pointerId: number, x: number, y: number) => {
			overlay.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				}),
			);
			overlay.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				}),
			);
		};
		tap(111, 720, 400);
		await vi.advanceTimersByTimeAsync(60);
		expect(controls?.getAttribute("aria-hidden")).toBe("true");
		tap(112, 730, 408);
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		expect(controls?.getAttribute("aria-hidden")).toBe("true");
		expect(before).toBe("false");
		vi.useRealTimers();
	});

	it("prevents browser double-tap zoom only while the viewer is open", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		await expect
			.poll(
				() =>
					(
						view
							.getByRole("button", { name: "Zoom in" })
							.element() as HTMLButtonElement
					).disabled,
			)
			.toBe(false);
		vi.useFakeTimers();
		try {
			const dispatch = (
				type: "pointerdown" | "pointerup",
				pointerId: number,
			) => {
				const event = new PointerEvent(type, {
					bubbles: true,
					cancelable: true,
					clientX: 720,
					clientY: 400,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				});
				overlay.dispatchEvent(event);
				return event;
			};
			dispatch("pointerdown", 501);
			dispatch("pointerup", 501);
			await vi.advanceTimersByTimeAsync(60);
			dispatch("pointerdown", 502);
			const secondUp = dispatch("pointerup", 502);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			expect(secondUp.defaultPrevented).toBe(true);
			await view.getByRole("button", { name: "Back to photos" }).click();
			const dispatchClosedTap = (pointerId: number, x: number, y: number) => {
				const down = new PointerEvent("pointerdown", {
					bubbles: true,
					cancelable: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				});
				const up = new PointerEvent("pointerup", {
					bubbles: true,
					cancelable: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				});
				document.dispatchEvent(down);
				document.dispatchEvent(up);
				return [down, up];
			};
			const closedFirstTap = dispatchClosedTap(503, 720, 400);
			await vi.advanceTimersByTimeAsync(60);
			const closedSecondTap = dispatchClosedTap(504, 728, 408);
			const closedTapEvents = [...closedFirstTap, ...closedSecondTap];
			expect(closedTapEvents.every((event) => !event.defaultPrevented)).toBe(
				true,
			);
			expect(document.querySelector("[aria-label='Photo viewer']")).toBeNull();
		} finally {
			vi.useRealTimers();
		}
	});

	it("releases document wheel handling after the viewer closes", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const stage = view.getByTestId("viewer-stage").element();
		await view.getByRole("button", { name: "Zoom in" }).click();
		const openWheel = new WheelEvent("wheel", {
			bubbles: true,
			cancelable: true,
			deltaY: 32,
			clientX: 720,
			clientY: 400,
		});
		stage.dispatchEvent(openWheel);
		expect(openWheel.defaultPrevented).toBe(true);
		await view.getByRole("button", { name: "Back to photos" }).click();
		const closedWheel = new WheelEvent("wheel", {
			bubbles: true,
			cancelable: true,
			deltaY: 32,
			clientX: 720,
			clientY: 400,
		});
		document.dispatchEvent(closedWheel);
		expect(closedWheel.defaultPrevented).toBe(false);
	});

	it("keeps a middle asset while a zoomed drag pans at the edge", async () => {
		const service = serviceWithReadyPhotos(false, 3);
		const originalDerivativeUrl = service.derivativeUrl.bind(service);
		const originalQueryWall = service.queryWall.bind(service);
		service.queryWall = async (request) => {
			const page = await originalQueryWall(request);
			return {
				...page,
				items: page.items.map((item) => ({
					...item,
					screenPreview: item.screenPreview ?? {
						assetId: item.id,
						kind: "screenPreview" as const,
						key: `${item.id}-screen`,
					},
				})),
			};
		};
		service.derivativeUrl = (reference) =>
			reference.kind === "wallThumbnail" || reference.kind === "screenPreview"
				? "/demo-photos/coast.jpg"
				: originalDerivativeUrl(reference);
		const { view } = await openAssetWithService(service, "Photo 1");
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "photo-1");
		const dispatch = (type: string, pointerId: number, x: number) => {
			const event = new PointerEvent(type, {
				bubbles: true,
				cancelable: true,
				clientX: x,
				clientY: 400,
				isPrimary: true,
				pointerId,
				pointerType: "touch",
			});
			overlay.dispatchEvent(event);
			return event;
		};
		// This direction navigates while fit, proving the same direction is
		// deliberately consumed as pan once zoomed.
		dispatch("pointerdown", 301, 620);
		dispatch("pointerup", 301, 520);
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "photo-2");
		await view.getByRole("button", { name: "Previous photo" }).click();
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "photo-1");
		await expect
			.element(view.getByRole("button", { name: "Zoom in" }))
			.toBeEnabled();
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		const before = stage.querySelector<HTMLElement>(
			"[data-testid='viewer-transform-layer']",
		)?.style.transform;
		dispatch("pointerdown", 302, 620);
		const move = dispatch("pointermove", 302, 120);
		dispatch("pointerup", 302, 120);
		await expect
			.element(stage)
			.toHaveAttribute("data-current-asset", "photo-1");
		expect(move.defaultPrevented).toBe(true);
		await expect
			.poll(
				() =>
					stage.querySelector<HTMLElement>(
						"[data-testid='viewer-transform-layer']",
					)?.style.transform,
			)
			.not.toBe(before);
	});

	it("keeps filmstrip and zoom controls out of stage touch routing", async () => {
		const service = serviceWithReadyPhotos(false, 3);
		const originalDerivativeUrl = service.derivativeUrl.bind(service);
		service.derivativeUrl = (reference) =>
			reference.kind === "wallThumbnail"
				? "/demo-photos/coast.jpg"
				: originalDerivativeUrl(reference);
		const { view } = await openAssetWithService(service, "Photo 0");
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const stage = view.getByTestId("viewer-stage").element();
		const filmstripButton = view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 0", exact: true })
			.element();
		const transform = stage.querySelector<HTMLElement>(
			"[data-testid='viewer-transform-layer']",
		);
		const before = transform?.style.transform;
		const dispatch = (target: Element, type: string, pointerId: number) =>
			target.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX: 620,
					clientY: 400,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				}),
			);
		dispatch(filmstripButton, "pointerdown", 311);
		dispatch(filmstripButton, "pointerup", 311);
		const zoomIn = view.getByRole("button", { name: "Zoom in" }).element();
		dispatch(zoomIn, "pointerdown", 312);
		dispatch(zoomIn, "pointerup", 312);
		const nextButton = view
			.getByRole("button", { name: "Next photo" })
			.element();
		dispatch(nextButton, "pointerdown", 313);
		dispatch(nextButton, "pointerup", 313);
		expect(stage.dataset.currentAsset).toBe("photo-0");
		expect(transform?.style.transform).toBe(before);
	});

	it("tombstones excluded drawer, filmstrip, and zoom touches through outside release", async () => {
		const service = serviceWithReadyPhotos(false, 3);
		const originalDerivativeUrl = service.derivativeUrl.bind(service);
		service.derivativeUrl = (reference) =>
			reference.kind === "wallThumbnail"
				? "/demo-photos/coast.jpg"
				: originalDerivativeUrl(reference);
		const { view } = await openAssetWithService(service, "Photo 0");
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const stage = view.getByTestId("viewer-stage").element();
		await view.getByRole("button", { name: "Zoom in" }).click();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const zoomIn = view.getByRole("button", { name: "Zoom in" }).element();
		const filmstrip = view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 0", exact: true })
			.element();
		const dispatch = (
			target: Element,
			type: string,
			pointerId: number,
			x = 620,
			y = 400,
		) =>
			target.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					cancelable: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId,
					pointerType: "touch",
				}),
			);
		const assertOutsideReleaseIsIgnored = (
			target: Element,
			pointerId: number,
		) => {
			dispatch(target, "pointerdown", pointerId);
			dispatch(overlay, "pointerup", pointerId);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			expect(stage.dataset.currentAsset).toBe("photo-0");
		};
		assertOutsideReleaseIsIgnored(zoomIn, 801);
		assertOutsideReleaseIsIgnored(filmstrip, 802);

		await view.getByRole("button", { name: "Photo information" }).click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		assertOutsideReleaseIsIgnored(drawer, 803);

		const before = stage.querySelector<HTMLElement>(
			"[data-testid='viewer-transform-layer']",
		)?.style.transform;
		dispatch(stage, "pointerdown", 804, 520, 400);
		dispatch(drawer, "pointerdown", 805, 560, 420);
		dispatch(overlay, "pointercancel", 805, 560, 420);
		dispatch(stage, "pointermove", 804, 600, 440);
		dispatch(stage, "pointerup", 804, 600, 440);
		await expect
			.poll(
				() =>
					stage.querySelector<HTMLElement>(
						"[data-testid='viewer-transform-layer']",
					)?.style.transform,
			)
			.not.toBe(before);
		expect(stage.dataset.currentAsset).toBe("photo-0");
		await view.unmount();
	});

	it("clears a pending single tap on pointer cancellation and asset revision", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const stage = view.getByTestId("viewer-stage").element();
		vi.useFakeTimers();
		try {
			const tap = (pointerId: number) => {
				overlay.dispatchEvent(
					new PointerEvent("pointerdown", {
						bubbles: true,
						clientX: 720,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
				overlay.dispatchEvent(
					new PointerEvent("pointerup", {
						bubbles: true,
						clientX: 720,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
			};
			tap(321);
			overlay.dispatchEvent(
				new PointerEvent("pointercancel", {
					bubbles: true,
					pointerId: 321,
					pointerType: "touch",
				}),
			);
			await vi.advanceTimersByTimeAsync(280);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");

			tap(322);
			await userEvent.keyboard("{ArrowRight}");
			await expect
				.element(stage)
				.toHaveAttribute("data-current-asset", "photo-0");
			await vi.advanceTimersByTimeAsync(280);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
		} finally {
			vi.useRealTimers();
		}
	});

	it("clears a pending single tap on viewport, rotation, and close", async () => {
		const restoreViewport = installVisualViewportDouble(390, 844);
		try {
			const { view, tile } = await openAsset("Coast");
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const overlay = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			const controls = overlay.querySelector<HTMLElement>(
				"[data-viewer-controls]",
			);
			const tap = (pointerId: number) => {
				overlay.dispatchEvent(
					new PointerEvent("pointerdown", {
						bubbles: true,
						clientX: 720,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
				overlay.dispatchEvent(
					new PointerEvent("pointerup", {
						bubbles: true,
						clientX: 720,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
			};
			tap(331);
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			await new Promise((resolve) => setTimeout(resolve, 320));
			expect(controls?.getAttribute("aria-hidden")).toBe("false");

			tap(332);
			viewport.setSize(390, 844);
			window.dispatchEvent(new Event("orientationchange"));
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			await new Promise((resolve) => setTimeout(resolve, 320));
			expect(controls?.getAttribute("aria-hidden")).toBe("false");

			tap(333);
			await view.getByRole("button", { name: "Back to photos" }).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.not.toBeInTheDocument();
			await new Promise((resolve) => setTimeout(resolve, 320));
		} finally {
			restoreViewport();
		}
	});

	it("does not fire a pending tap after the viewer closes", async () => {
		const stats = { panEnds: 0, taps: 0 };
		const view = await render(<GestureLifecycleHarness stats={stats} />);
		const target = view.getByTestId("gesture-lifecycle-target").element();
		const tap = (type: "pointerdown" | "pointerup") =>
			target.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX: 320,
					clientY: 240,
					isPrimary: true,
					pointerId: 341,
					pointerType: "touch",
				}),
			);
		tap("pointerdown");
		tap("pointerup");
		await view.getByTestId("gesture-close").click();
		await expect
			.element(view.getByTestId("gesture-lifecycle-target"))
			.not.toBeInTheDocument();
		await new Promise((resolve) => setTimeout(resolve, 320));
		expect(stats.taps).toBe(0);
		expect(view.getByTestId("gesture-taps").element().textContent).toBe("0");
	});

	it("does not lose the first tap when the second tap is too far away", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		vi.useFakeTimers();
		try {
			const tap = (pointerId: number, x: number) => {
				overlay.dispatchEvent(
					new PointerEvent("pointerdown", {
						bubbles: true,
						clientX: x,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
				overlay.dispatchEvent(
					new PointerEvent("pointerup", {
						bubbles: true,
						clientX: x,
						clientY: 400,
						isPrimary: true,
						pointerId,
						pointerType: "touch",
					}),
				);
			};
			tap(121, 500);
			await vi.advanceTimersByTimeAsync(100);
			tap(122, 700);
			await vi.advanceTimersByTimeAsync(180);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
		} finally {
			vi.useRealTimers();
		}
	});

	it("reports keyboard, button, filmstrip, and touch activity to the wall", async () => {
		const { service, view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		service.interactionCalls.length = 0;
		await userEvent.keyboard("{ArrowRight}");
		await view.getByRole("button", { name: "Previous photo" }).click();
		await view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 0", exact: true })
			.click();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		overlay.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				clientX: 620,
				clientY: 400,
				isPrimary: true,
				pointerId: 31,
				pointerType: "touch",
			}),
		);
		overlay.dispatchEvent(
			new PointerEvent("pointerup", {
				bubbles: true,
				clientX: 540,
				clientY: 410,
				isPrimary: true,
				pointerId: 31,
				pointerType: "touch",
			}),
		);
		await expect.poll(() => service.interactionCalls.includes(true)).toBe(true);
		await new Promise((resolve) => setTimeout(resolve, 250));
		await expect.poll(() => service.interactionCalls.at(-1)).toBe(false);
	});

	it("keeps mouse drags and clicks out of touch gesture routing", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const dispatch = (type: string, pointerType: string, x: number) =>
			overlay.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					button: 0,
					clientX: x,
					clientY: 400,
					isPrimary: true,
					pointerId: 22,
					pointerType,
				}),
			);
		dispatch("pointerdown", "mouse", 620);
		dispatch("pointerup", "mouse", 520);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "coast");
		dispatch("pointerdown", "touch", 620);
		dispatch("pointerup", "touch", 520);
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
	});

	it("coalesces visual viewport rotation without resetting the viewer", async () => {
		const restoreViewport = installVisualViewportDouble(390, 844);
		const originalScrollIntoView = Element.prototype.scrollIntoView;
		let scrollCalls = 0;
		Element.prototype.scrollIntoView = () => {
			scrollCalls += 1;
		};
		try {
			const { service, view, tile } = await openAsset("Coast", false, true);
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const overlay = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			await expect
				.poll(() =>
					overlay.querySelector(
						"[data-viewer-layer='screenPreview'][data-ready='true']",
					),
				)
				.not.toBeNull();
			const screenPreview = overlay.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview'][data-ready='true']",
			);
			const portraitCapacity = Number(
				overlay
					.querySelector("[data-filmstrip-capacity]")
					?.getAttribute("data-filmstrip-capacity"),
			);
			const beforeScrollCalls = scrollCalls;
			await view.getByRole("button", { name: "Photo information" }).click();
			await expect
				.element(view.getByRole("complementary", { name: "Photo information" }))
				.toBeVisible();
			await expect
				.poll(() =>
					service.derivativeRequests.some(
						(request) =>
							request.kind === "screenPreview" &&
							request.assetIds.includes("coast"),
					),
				)
				.toBe(true);
			const beforeRevision = Number(overlay.dataset.viewportRevision);
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			window.dispatchEvent(new Event("orientationchange"));
			expect(Number(overlay.dataset.viewportRevision)).toBe(beforeRevision);
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			await expect
				.poll(() => Number(overlay.dataset.viewportRevision))
				.toBe(beforeRevision + 1);
			expect(
				overlay
					.querySelector("[data-filmstrip-capacity]")
					?.getAttribute("data-filmstrip-capacity"),
			).toBe("11");
			expect(Number(portraitCapacity)).toBe(5);
			expect(scrollCalls).toBeGreaterThan(beforeScrollCalls);
			expect(
				overlay.querySelector("[data-current-asset='coast']"),
			).not.toBeNull();
			expect(
				overlay.querySelector("[data-testid='photo-info-drawer']"),
			).not.toBeNull();
			expect(
				overlay
					.querySelector("[aria-current='true']")
					?.getAttribute("aria-label"),
			).toBe("Coast");
			expect(
				overlay.querySelector(
					"[data-viewer-layer='screenPreview'][data-ready='true']",
				),
			).toBe(screenPreview);
			expect(seriousViolations(await axe.run(document))).toEqual([]);
		} finally {
			Element.prototype.scrollIntoView = originalScrollIntoView;
			restoreViewport();
		}
	});

	it("keeps drawer movement scrolling without entering stage gestures", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		const drawer = view
			.getByRole("complementary", { name: "Photo information" })
			.element();
		drawer.style.maxHeight = "120px";
		await expect
			.poll(() => drawer.scrollHeight > drawer.clientHeight)
			.toBe(true);
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const controlsBefore = controls?.getAttribute("aria-hidden");
		const dispatch = (type: string, x: number, y: number) =>
			drawer.dispatchEvent(
				new PointerEvent(type, {
					bubbles: true,
					clientX: x,
					clientY: y,
					isPrimary: true,
					pointerId: 11,
					pointerType: "touch",
				}),
			);
		drawer.scrollTop = 80;
		expect(drawer.scrollTop).toBeGreaterThan(0);
		dispatch("pointerdown", 500, 300);
		dispatch("pointerup", 518, 390);
		expect(controls?.getAttribute("aria-hidden")).toBe(controlsBefore);
		expect(
			view.getByTestId("viewer-stage").element().dataset.currentAsset,
		).toBe("coast");
		dispatch("pointerdown", 500, 300);
		dispatch("pointerup", 420, 310);
		expect(
			view.getByTestId("viewer-stage").element().dataset.currentAsset,
		).toBe("coast");
	});

	it("cancels an active swipe when the viewport revision changes", async () => {
		const restoreViewport = installVisualViewportDouble(390, 844);
		try {
			const { view, tile } = await openAsset("Coast");
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const overlay = view
				.getByRole("dialog", { name: "Photo viewer" })
				.element();
			overlay.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					clientX: 620,
					clientY: 400,
					isPrimary: true,
					pointerId: 13,
					pointerType: "touch",
				}),
			);
			const viewport = window.visualViewport as VisualViewport & {
				setSize: (width: number, height: number) => void;
			};
			viewport.setSize(844, 390);
			viewport.dispatchEvent(new Event("resize"));
			window.dispatchEvent(new Event("orientationchange"));
			await new Promise<void>((resolve) =>
				requestAnimationFrame(() => resolve()),
			);
			overlay.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					clientX: 520,
					clientY: 410,
					isPrimary: true,
					pointerId: 13,
					pointerType: "touch",
				}),
			);
			expect(
				view.getByTestId("viewer-stage").element().dataset.currentAsset,
			).toBe("coast");
		} finally {
			restoreViewport();
		}
	});

	it("opens information only from Info and keeps it open during navigation", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await view.getByTestId("viewer-stage").click();
		expect(
			view.getByRole("complementary", { name: "Photo information" }).query(),
		).toBeNull();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect.element(view.getByText("Unrated")).toBeVisible();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
	});

	it("closes information on the first Escape and returns to the wall on the second", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		await userEvent.keyboard("{Escape}");
		expect(
			view.getByRole("complementary", { name: "Photo information" }).query(),
		).toBeNull();
		await userEvent.keyboard("{Escape}");
		expect(
			view.getByRole("dialog", { name: "Photo viewer" }).query(),
		).toBeNull();
	});

	it("resets zoom on the first Escape and closes the viewer on the second", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const stage = view.getByTestId("viewer-stage");
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");

		await userEvent.keyboard("{Escape}");
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();

		await userEvent.keyboard("{Escape}");
		expect(
			view.getByRole("dialog", { name: "Photo viewer" }).query(),
		).toBeNull();
	});

	it("unwinds the information drawer, zoom, and viewer in Escape order", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const stage = view.getByTestId("viewer-stage");
		await view.getByRole("button", { name: "Zoom in" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
		await view.getByRole("button", { name: "Reset zoom" }).click();
		await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
		const status = view.getByTestId("viewer-status").element();
		const mutations: MutationRecord[] = [];
		const observer = new MutationObserver((records) =>
			mutations.push(...records),
		);
		observer.observe(status, {
			characterData: true,
			childList: true,
			subtree: true,
		});
		const flush = () =>
			new Promise<void>((resolve) => window.setTimeout(resolve, 0));
		try {
			stage.element().dispatchEvent(
				new WheelEvent("wheel", {
					bubbles: true,
					cancelable: true,
					ctrlKey: true,
					deltaY: -24,
					clientX: 720,
					clientY: 400,
				}),
			);
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			await flush();
			expect(status.textContent).toContain("Fit");
			expect(mutations).toHaveLength(0);

			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
			await view.getByRole("button", { name: "Photo information" }).click();
			await expect
				.element(view.getByRole("complementary", { name: "Photo information" }))
				.toBeVisible();

			mutations.splice(0, mutations.length);
			await userEvent.keyboard("{Escape}");
			expect(
				view.getByRole("complementary", { name: "Photo information" }).query(),
			).toBeNull();
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");

			mutations.splice(0, mutations.length);
			await userEvent.keyboard("{Escape}");
			await expect.element(stage).toHaveAttribute("data-viewer-mode", "fit");
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			await expect.poll(() => mutations.length).toBeGreaterThan(0);
			expect(status.textContent).toContain("Fit");

			await userEvent.keyboard("{Escape}");
			expect(
				view.getByRole("dialog", { name: "Photo viewer" }).query(),
			).toBeNull();
		} finally {
			observer.disconnect();
			await view.unmount();
		}
	});

	it("announces the current position and loads more when pagination remains", async () => {
		const { view } = await openManyAsset("Coast");
		const tile = view.getByRole("button", { name: "Open Coast", exact: true });
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByTestId("viewer-status"))
			.toHaveTextContent("Coast, photo 1 of 100, Fit loaded");
	});

	it("uses quiet mouse timers and toggles touch controls with the stage", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			'[aria-hidden="false"]',
		);
		const stage = view.getByTestId("viewer-stage").element();
		vi.useFakeTimers();
		try {
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect.element(view.getByTestId("viewer-stage")).toBeVisible();
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 1024,
					pointerType: "mouse",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();

			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();
			await vi.advanceTimersByTimeAsync(3499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.element(view.getByRole("group", { name: "Photo filmstrip" }))
				.toBeVisible();
			stage.dispatchEvent(
				new PointerEvent("pointerup", {
					bubbles: true,
					pointerType: "touch",
				}),
			);
			await expect
				.poll(() =>
					view.getByRole("group", { name: "Photo filmstrip" }).query(),
				)
				.toBeNull();
		} finally {
			vi.useRealTimers();
		}
	});

	it("starts idle hiding on open and resumes after control focus leaves", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const controls = overlay.querySelector<HTMLElement>(
			"[data-viewer-controls]",
		);
		const zoomControls = overlay.querySelector<HTMLElement>(
			"[data-viewer-zoom-controls]",
		);
		const next = view.getByRole("button", { name: "Next photo" }).element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("false");
			next.focus();
			await vi.advanceTimersByTimeAsync(3000);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			const zoomIn = view.getByRole("button", { name: "Zoom in" }).element();
			zoomIn.focus();
			await vi.advanceTimersByTimeAsync(3000);
			expect(zoomControls?.getAttribute("aria-hidden")).toBe("false");
			zoomIn.blur();
			await vi.advanceTimersByTimeAsync(2499);
			expect(controls?.getAttribute("aria-hidden")).toBe("false");
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
		} finally {
			vi.useRealTimers();
		}
	});

	it("starts the desktop inactivity timer when the viewer opens", async () => {
		const { view, tile } = await openAsset("Coast");
		vi.useFakeTimers();
		try {
			(tile.element() as HTMLButtonElement).click();
			await expect
				.element(view.getByRole("dialog", { name: "Photo viewer" }))
				.toBeVisible();
			const controls = document.querySelector<HTMLElement>(
				"[data-viewer-controls]",
			);
			await vi.advanceTimersByTimeAsync(2500);
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			expect(
				view.getByRole("group", { name: "Photo filmstrip" }).query(),
			).toBeNull();
			const hiddenNavigation = controls?.querySelectorAll("button") ?? [];
			expect([...hiddenNavigation].every((button) => button.tabIndex < 0)).toBe(
				true,
			);
		} finally {
			vi.useRealTimers();
		}
	});

	it("removes the full chrome from tab order when controls hide", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			const chrome = overlay.querySelector<HTMLElement>("[data-viewer-chrome]");
			const controls = overlay.querySelector<HTMLElement>(
				"[data-viewer-controls]",
			);
			const info =
				overlay.querySelector<HTMLButtonElement>("[data-viewer-info]");
			await expect.poll(() => document.activeElement).toBe(overlay);
			await expect.poll(() => chrome?.getAttribute("aria-hidden")).toBe("true");
			await expect
				.poll(() => controls?.getAttribute("aria-hidden"))
				.toBe("true");
			await expect.poll(() => info?.getAttribute("aria-hidden")).toBe("true");
			expect(info?.tabIndex).toBe(-1);
			expect(
				overlay.querySelector('[data-viewer-controls] [tabindex="-1"]'),
			).not.toBeNull();
		} finally {
			vi.useRealTimers();
		}
	});

	it("keeps Tab inside the dialog when all viewer chrome is hidden", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const dialog = view.getByRole("dialog", { name: "Photo viewer" }).element();
		vi.useFakeTimers();
		try {
			dialog.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			await expect.poll(() => dialog.getAttribute("aria-hidden")).toBeNull();
			await expect
				.poll(() =>
					dialog
						.querySelector("[data-viewer-chrome]")
						?.getAttribute("aria-hidden"),
				)
				.toBe("true");
			dialog.focus();
			const tab = new KeyboardEvent("keydown", {
				bubbles: true,
				cancelable: true,
				key: "Tab",
			});
			dialog.dispatchEvent(tab);
			expect(tab.defaultPrevented).toBe(true);
			expect(document.activeElement).toBe(dialog);
			const reverseTab = new KeyboardEvent("keydown", {
				bubbles: true,
				cancelable: true,
				key: "Tab",
				shiftKey: true,
			});
			dialog.dispatchEvent(reverseTab);
			expect(reverseTab.defaultPrevented).toBe(true);
			expect(document.activeElement).toBe(dialog);
		} finally {
			vi.useRealTimers();
		}
	});

	it("cycles visible drawer controls while primary controls are hidden", async () => {
		const { view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await view.getByRole("button", { name: "Photo information" }).click();
		await expect
			.element(view.getByRole("complementary", { name: "Photo information" }))
			.toBeVisible();
		const overlay = view
			.getByRole("dialog", { name: "Photo viewer" })
			.element();
		const back = view.getByRole("button", { name: "Back to photos" }).element();
		const previous = overlay.querySelector<HTMLButtonElement>(
			'[data-viewer-controls] button[aria-label="Previous photo"]',
		);
		const next = overlay.querySelector<HTMLButtonElement>(
			'[data-viewer-controls] button[aria-label="Next photo"]',
		);
		const close = view
			.getByRole("button", { name: "Close photo information" })
			.element();
		vi.useFakeTimers();
		try {
			overlay.dispatchEvent(
				new PointerEvent("pointermove", {
					bubbles: true,
					clientY: 100,
					pointerType: "mouse",
				}),
			);
			await vi.advanceTimersByTimeAsync(2500);
			back.focus();
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(previous);
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(next);
			await userEvent.keyboard("{Tab}");
			expect(document.activeElement).not.toBe(previous);
			close.focus();
			await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
			expect(document.activeElement).not.toBe(
				view.getByRole("button", { name: "Next photo" }).element(),
			);
		} finally {
			vi.useRealTimers();
		}
	});

	it("retries a rejected screen-preview request", async () => {
		const requests: string[][] = [];
		let attempts = 0;
		const service = previewService(async (request) => {
			if (request.assetIds.includes("a")) {
				requests.push([...request.assetIds]);
				if (attempts++ === 0) throw new Error("temporary failure");
			}
		});
		await render(
			<PhotoServiceProvider service={service}>
				<PreviewHarness
					assets={[asset("a", "A", 1), asset("b", "B", 2)]}
					service={service}
				/>
			</PhotoServiceProvider>,
		);

		await expect
			.poll(() => requests.filter((ids) => ids.includes("a")).length)
			.toBe(2);
	});

	it("clears per-key retry state after deactivation and reactivation", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
			if (requests.length <= 3) throw new Error("temporary failure");
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<ActivatablePreviewHarness
						assets={[asset("a", "A", 1)]}
						service={service}
					/>
				</PhotoServiceProvider>,
			);
			await vi.advanceTimersByTimeAsync(0);
			await expect.poll(() => requests.length).toBe(1);
			await vi.advanceTimersByTimeAsync(100);
			await expect.poll(() => requests.length).toBe(2);
			await vi.advanceTimersByTimeAsync(250);
			await expect.poll(() => requests.length).toBe(3);
			await vi.advanceTimersByTimeAsync(0);
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "true");

			await view.getByTestId("deactivate-preview").click();
			await expect
				.element(view.getByTestId("preview-active"))
				.toHaveTextContent("false");
			await view.getByTestId("activate-preview").click();
			await expect.poll(() => requests.length).toBe(4);
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "false");
			await view.unmount();
		} finally {
			vi.useRealTimers();
		}
	});

	it("fences a late preview rejection after a real unmount", async () => {
		const requests: DerivativeRequest[] = [];
		let rejectPending = (_reason?: unknown): void => undefined;
		const pending = new Promise<void>((_resolve, reject) => {
			rejectPending = reject;
		});
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
			await pending;
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<PreviewHarness assets={[asset("a", "A", 1)]} service={service} />
				</PhotoServiceProvider>,
			);
			await vi.advanceTimersByTimeAsync(0);
			await expect.poll(() => requests.length).toBe(1);
			await view.unmount();
			rejectPending(new Error("late failure"));
			await Promise.resolve();
			await Promise.resolve();
			await vi.advanceTimersByTimeAsync(1000);
			expect(requests).toHaveLength(1);
		} finally {
			vi.useRealTimers();
		}
	});

	it("keeps an in-flight preview rejection alive across an equivalent asset refresh", async () => {
		const requests: DerivativeRequest[] = [];
		let rejectCurrent = (_reason?: unknown): void => {
			throw new Error("Current preview request is not pending");
		};
		let rejectionReady = false;
		let firstRequest = true;
		const service = previewService((request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
			if (request.priority === "visible" && request.assetIds.includes("a")) {
				if (!firstRequest)
					return Promise.reject(new Error("temporary failure"));
				firstRequest = false;
				return new Promise<void>((_resolve, reject) => {
					rejectCurrent = reject;
					rejectionReady = true;
				});
			}
			return Promise.resolve();
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<RefreshingPreviewHarness
						initialAssets={[asset("a", "A", 1), asset("b", "B", 2)]}
						service={service}
					/>
				</PhotoServiceProvider>,
			);
			await vi.advanceTimersByTimeAsync(0);
			await expect.poll(() => rejectionReady).toBe(true);
			await view.getByTestId("refresh-preview-assets").click();
			rejectCurrent(new Error("temporary failure"));
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "true");
			await vi.advanceTimersByTimeAsync(100);
			await expect
				.poll(
					() =>
						requests.filter(
							(request) =>
								request.priority === "visible" &&
								request.assetIds.includes("a"),
						).length,
				)
				.toBe(2);
			await vi.advanceTimersByTimeAsync(250);
			await expect
				.poll(
					() =>
						requests.filter(
							(request) =>
								request.priority === "visible" &&
								request.assetIds.includes("a"),
						).length,
				)
				.toBe(3);
			expect(
				requests.filter(
					(request) =>
						request.priority === "visible" && request.assetIds.includes("a"),
				),
			).toHaveLength(3);
		} finally {
			vi.useRealTimers();
		}
	});

	it("retains a scheduled preview retry across an equivalent asset refresh", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
			if (request.priority === "visible" && request.assetIds.includes("a"))
				throw new Error("permanent failure");
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<RefreshingPreviewHarness
						initialAssets={[asset("a", "A", 1), asset("b", "B", 2)]}
						service={service}
					/>
				</PhotoServiceProvider>,
			);
			await vi.advanceTimersByTimeAsync(0);
			await expect
				.poll(
					() =>
						requests.filter(
							(request) =>
								request.priority === "visible" &&
								request.assetIds.includes("a"),
						).length,
				)
				.toBe(1);
			await view.getByTestId("refresh-preview-assets").click();
			await vi.advanceTimersByTimeAsync(99);
			expect(
				requests.filter(
					(request) =>
						request.priority === "visible" && request.assetIds.includes("a"),
				),
			).toHaveLength(1);
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(
					() =>
						requests.filter(
							(request) =>
								request.priority === "visible" &&
								request.assetIds.includes("a"),
						).length,
				)
				.toBe(2);
			await vi.advanceTimersByTimeAsync(250);
			await expect
				.poll(
					() =>
						requests.filter(
							(request) =>
								request.priority === "visible" &&
								request.assetIds.includes("a"),
						).length,
				)
				.toBe(3);
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "true");
		} finally {
			vi.useRealTimers();
		}
	});

	it("marks a permanently rejected preview unavailable and stops retrying", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
			throw new Error("permanent failure");
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<PreviewHarness assets={[asset("a", "A", 1)]} service={service} />
				</PhotoServiceProvider>,
			);
			await vi.runOnlyPendingTimersAsync();
			await vi.advanceTimersByTimeAsync(1000);
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-large-preview-unavailable", "true");
			expect(requests.length).toBeLessThanOrEqual(3);
		} finally {
			vi.useRealTimers();
		}
	});

	it("promotes a near-viewport request to visible after switching assets", async () => {
		const requests: Array<{ assetIds: string[]; priority: string }> = [];
		const service = previewService(async (request) => {
			requests.push({
				assetIds: [...request.assetIds],
				priority: request.priority,
			});
		});
		const assets = [asset("a", "A", 1), asset("b", "B", 2), asset("c", "C", 3)];
		const view = await render(
			<PhotoServiceProvider service={service}>
				<PreviewHarness assets={assets} service={service} />
			</PhotoServiceProvider>,
		);
		await expect
			.poll(() =>
				requests.some(
					(request) =>
						request.assetIds.includes("b") &&
						request.priority === "nearViewport",
				),
			)
			.toBe(true);
		await view.getByTestId("switch-preview").click();
		await expect
			.poll(() =>
				requests.some(
					(request) =>
						request.assetIds.length === 1 &&
						request.assetIds[0] === "b" &&
						request.priority === "visible",
				),
			)
			.toBe(true);
	});

	it("defers the idle neighbour group while interaction remains active", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
		});
		const idleCallback = Object.getOwnPropertyDescriptor(
			window,
			"requestIdleCallback",
		);
		const cancelIdleCallback = Object.getOwnPropertyDescriptor(
			window,
			"cancelIdleCallback",
		);
		Object.defineProperty(window, "requestIdleCallback", {
			configurable: true,
			value: undefined,
		});
		Object.defineProperty(window, "cancelIdleCallback", {
			configurable: true,
			value: undefined,
		});
		vi.useFakeTimers();
		try {
			const view = await render(
				<PhotoServiceProvider service={service}>
					<PreviewHarness
						assets={[
							asset("a", "A", 1),
							asset("b", "B", 2),
							asset("c", "C", 3),
						]}
						service={service}
					/>
				</PhotoServiceProvider>,
			);
			await vi.advanceTimersByTimeAsync(0);
			await expect
				.poll(() =>
					requests.some(
						(request) =>
							request.assetIds.length === 1 &&
							request.assetIds[0] === "a" &&
							request.priority === "visible",
					),
				)
				.toBe(true);
			await view.getByTestId("report-preview-interaction").click();
			await vi.advanceTimersByTimeAsync(150);
			await view.getByTestId("report-preview-interaction").click();
			await vi.advanceTimersByTimeAsync(100);
			expect(requests.some((request) => request.assetIds.includes("c"))).toBe(
				false,
			);
			await vi.advanceTimersByTimeAsync(249);
			expect(requests.some((request) => request.assetIds.includes("c"))).toBe(
				false,
			);
			await vi.advanceTimersByTimeAsync(1);
			await expect
				.poll(() => requests.some((request) => request.assetIds.includes("c")))
				.toBe(true);
		} finally {
			vi.useRealTimers();
			if (idleCallback)
				Object.defineProperty(window, "requestIdleCallback", idleCallback);
			else Reflect.deleteProperty(window, "requestIdleCallback");
			if (cancelIdleCallback)
				Object.defineProperty(window, "cancelIdleCallback", cancelIdleCallback);
			else Reflect.deleteProperty(window, "cancelIdleCallback");
		}
	});

	it("fences an older decode completion when the current asset changes", async () => {
		const pending = new Map<string, () => void>();
		const originalDecode = HTMLImageElement.prototype.decode;
		HTMLImageElement.prototype.decode = function () {
			const id = this.src.includes("a-screenPreview") ? "a" : "b";
			return new Promise<void>((resolve) => pending.set(id, resolve));
		};
		try {
			const service = previewService(async () => undefined);
			const assets = [previewAsset("a", 1), previewAsset("b", 2)];
			const view = await render(
				<PhotoServiceProvider service={service}>
					<PreviewHarness assets={assets} service={service} />
				</PhotoServiceProvider>,
			);
			await expect.poll(() => pending.has("a")).toBe(true);
			// Keep A pending while changing the current asset and generation.
			await view.getByTestId("switch-preview").click();
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-current-asset", "b");
			await expect
				.poll(
					() =>
						document.querySelector<HTMLImageElement>(
							"[data-viewer-layer='screenPreview']",
						)?.dataset.ready,
				)
				.toBe("false");
			await expect.poll(() => pending.has("b")).toBe(true);
			pending.get("b")?.();
			await expect
				.poll(
					() =>
						document.querySelector<HTMLImageElement>(
							"[data-viewer-layer='screenPreview']",
						)?.dataset.ready,
				)
				.toBe("true");
			// A completes after B is ready; this stale completion must be ignored.
			pending.get("a")?.();
			await new Promise((resolve) => setTimeout(resolve, 0));
			await expect
				.element(view.getByTestId("viewer-stage"))
				.toHaveAttribute("data-current-asset", "b");
			const screen = document.querySelector<HTMLImageElement>(
				"[data-viewer-layer='screenPreview']",
			);
			expect(screen?.src).toContain("b-screenPreview");
			expect(screen?.dataset.ready).toBe("true");
		} finally {
			HTMLImageElement.prototype.decode = originalDecode;
		}
	});

	it("advances from a broken screen preview to a decoding wall thumbnail", async () => {
		const { view, tile } = await openAsset("Coast", true);
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		await expect
			.poll(
				() =>
					document.querySelector<HTMLImageElement>(
						"[data-viewer-layer='wallThumbnail']",
					)?.src ?? "",
			)
			.toContain("/demo-photos/coast.jpg");
		const image = document.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		await expect
			.poll(() => (image?.complete ? image.naturalWidth : 0))
			.toBeGreaterThan(0);
	});

	it("isolates base and screen failures by derivative URL and asset generation", async () => {
		const view = await render(<StageFailureHarness />);
		const stage = view.getByTestId("viewer-stage").element();
		const base = stage.querySelector<HTMLImageElement>(
			"[data-viewer-layer='wallThumbnail']",
		);
		if (base) base.src = "data:image/gif;base64,invalid";
		await expect
			.poll(() =>
				stage.querySelector("[data-viewer-layer='representativeColour']"),
			)
			.not.toBeNull();
		await view.getByTestId("switch-stage-failure").click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "b");
		await expect
			.poll(() => stage.querySelector("[data-viewer-layer='wallThumbnail']"))
			.not.toBeNull();
		expect(
			stage.querySelector("[data-viewer-layer='representativeColour']"),
		).toBeNull();
		const screen = stage.querySelector<HTMLImageElement>(
			"[data-viewer-layer='screenPreview']",
		);
		if (screen) screen.src = "data:image/gif;base64,invalid";
		await expect
			.poll(() => stage.getAttribute("data-large-preview-unavailable"))
			.toBe("true");
		await view.getByTestId("switch-stage-failure").click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "c");
		await expect
			.poll(() => stage.getAttribute("data-large-preview-unavailable"))
			.toBe("false");
		expect(
			stage.querySelector("[data-viewer-layer='representativeColour']"),
		).toBeNull();
	});

	it("keeps keyboard focus inside the viewer instead of covered source controls", async () => {
		const { view, tile } = await openAsset("Photo 1");
		(tile.element() as HTMLButtonElement).click();
		const back = view.getByRole("button", { name: "Back to photos" });
		await expect.element(back).toBeVisible();
		const previous = view.getByRole("button", { name: "Previous photo" });
		const next = view.getByRole("button", { name: "Next photo" });
		const folders = view.getByRole("button", {
			name: "Folders",
			exact: true,
		});
		const firstThumb = view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Coast", exact: true });
		await back.element().focus();
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(previous.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(next.element());
		await userEvent.keyboard("{Tab}");
		expect(document.activeElement).toBe(firstThumb.element());
		expect(document.activeElement).not.toBe(folders.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(next.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(previous.element());
		await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
		expect(document.activeElement).toBe(back.element());
	});

	it("does not steal focus from viewer controls when selection or drawer state rerenders", async () => {
		const { view, tile } = await openAsset("Photo 1");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const previous = view.getByRole("button", { name: "Previous photo" });
		const next = view.getByRole("button", { name: "Next photo" });
		const info = view.getByRole("button", { name: "Photo information" });
		await previous.element().focus();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
		await expect.poll(() => document.activeElement).toBe(previous.element());
		await next.element().focus();
		await userEvent.keyboard("{ArrowLeft}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-1");
		await expect.poll(() => document.activeElement).toBe(next.element());
		await info.element().focus();
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
		await expect.poll(() => document.activeElement).toBe(info.element());
		await info.click();
		const close = view
			.getByRole("button", { name: "Close photo information" })
			.element();
		await close.focus();
		await userEvent.keyboard("{ArrowLeft}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-1");
		await expect.poll(() => document.activeElement).toBe(close);
		const selected = view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 2", exact: true });
		await selected.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
		await expect.poll(() => document.activeElement).toBe(selected.element());
	});

	it("does not retain a stale filmstrip focus request after reselecting the current asset", async () => {
		const { view, tile } = await openAsset("Photo 1");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		const filmstrip = view.getByRole("group", { name: "Photo filmstrip" });
		await filmstrip
			.getByRole("button", { name: "Photo 1", exact: true })
			.click();
		const next = view.getByRole("button", { name: "Next photo" });
		await next.element().focus();
		await next.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
		await expect.poll(() => document.activeElement).toBe(next.element());
	});

	it("closes when the current asset disappears during an open viewer", async () => {
		const { service, view, tile } = await openAsset("Coast");
		(tile.element() as HTMLButtonElement).click();
		await expect
			.element(view.getByRole("dialog", { name: "Photo viewer" }))
			.toBeVisible();
		service.emitForTest({
			kind: "resyncRequired",
			selectionId: "memory-selection-1",
		});
		await expect
			.poll(() => view.getByRole("dialog", { name: "Photo viewer" }).query())
			.toBeNull();
		const workspace = view.getByRole("region", { name: "Photo workspace" });
		await expect
			.poll(() => (workspace.element() as HTMLElement).inert)
			.toBe(false);
		await expect
			.poll(() => document.activeElement)
			.toBe(view.getByRole("region", { name: "Photos" }).element());
	});

	it("navigates in wall order without wrapping and bounds the filmstrip", async () => {
		const { view } = await openManyAsset("Coast");
		await userEvent.keyboard("{ArrowRight}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await userEvent.keyboard("{ArrowLeft}");
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "coast");
		await expect
			.element(view.getByRole("button", { name: "Previous photo" }))
			.toBeDisabled();
		const filmstrip = view.getByRole("group", { name: "Photo filmstrip" });
		expect(
			filmstrip.element().querySelectorAll("button").length,
		).toBeLessThanOrEqual(31);
		expect(
			filmstrip
				.element()
				.querySelector("button[aria-current='true']")
				?.getAttribute("aria-label"),
		).toBe("Coast");
	});

	it("holds the current photo while loading and then continues into the next page", async () => {
		const delayed = delayedPaginationService();
		const { view } = await openAssetWithService(delayed.service, "Photo 96");
		await expect
			.poll(
				() =>
					delayed.queryRequests.filter((request) => request.cursor === "100")
						.length,
			)
			.toBe(1);
		const next = view.getByRole("button", { name: "Next photo" });
		await expect.element(next).toBeEnabled();
		await next.click();
		await next.click();
		await expect.element(next).toBeDisabled();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-98");
		await delayed.releaseNextPage();
		await expect.element(next).toBeEnabled();
		await next.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-99");
		expect(
			delayed.queryRequests.filter((request) => request.cursor === "100"),
		).toHaveLength(1);
	});

	it("routes visible controls and filmstrip selection through ordered navigation", async () => {
		const { view } = await openManyAsset("Photo 1");
		await view.getByRole("button", { name: "Previous photo" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-0");
		await view.getByRole("button", { name: "Next photo" }).click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-1");
		await view
			.getByRole("group", { name: "Photo filmstrip" })
			.getByRole("button", { name: "Photo 2", exact: true })
			.click();
		await expect
			.element(view.getByTestId("viewer-stage"))
			.toHaveAttribute("data-current-asset", "photo-2");
	});

	it("requests missing filmstrip thumbnails near the viewport and scrolls immediately for reduced motion", async () => {
		const requests: DerivativeRequest[] = [];
		const service = previewService(async (request) => {
			requests.push({ ...request, assetIds: [...request.assetIds] });
		});
		const assets = [1, 2, 3, 4].map((order) => ({
			...asset(`film-${order}`, `Film ${order}`, order),
			wallThumbnail: null,
		}));
		const scrollCalls: ScrollIntoViewOptions[] = [];
		const originalScrollIntoView = Element.prototype.scrollIntoView;
		const originalMatchMedia = window.matchMedia;
		Element.prototype.scrollIntoView = (options) => {
			scrollCalls.push(options as ScrollIntoViewOptions);
		};
		window.matchMedia = (() =>
			({ matches: true }) as MediaQueryList) as typeof window.matchMedia;
		try {
			await render(
				<ViewerFilmstrip
					assets={assets}
					currentIndex={1}
					onRequestNearViewportDerivatives={(ids) =>
						service.requestDerivatives({
							assetIds: [...ids],
							kind: "wallThumbnail",
							priority: "nearViewport",
						})
					}
					onSelectAsset={() => undefined}
					service={service}
				/>,
			);
			await expect.poll(() => requests.length).toBeGreaterThan(0);
			expect(requests[0]).toMatchObject({
				kind: "wallThumbnail",
				priority: "nearViewport",
			});
			expect(scrollCalls[0]).toMatchObject({
				behavior: "auto",
				block: "nearest",
				inline: "center",
			});
		} finally {
			Element.prototype.scrollIntoView = originalScrollIntoView;
			window.matchMedia = originalMatchMedia;
		}
	});
});
