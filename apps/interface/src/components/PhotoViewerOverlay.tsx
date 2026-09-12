import { Check, ChevronLeft, ChevronRight, Info, Plus } from "lucide-react";
import {
	type KeyboardEvent as ReactKeyboardEvent,
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { usePickList, usePickListOrigin } from "../picks/PickListContext";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import { findViewerIndex, shouldLoadViewerPage } from "../viewer/photoSequence";
import { useViewerControls } from "../viewer/useViewerControls";
import { useViewerGestures } from "../viewer/useViewerGestures";
import { useViewerPreview } from "../viewer/useViewerPreview";
import { useViewerTransform } from "../viewer/useViewerTransform";
import type { ViewerState } from "../viewer/viewerReducer";
import { useViewerViewport } from "../viewer/viewerViewport";
import { classifyViewerWheel } from "../viewer/viewerZoomInput";
import { PhotoInfoDrawer } from "./PhotoInfoDrawer";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerNavigator } from "./ViewerNavigator";
import { ViewerStage } from "./ViewerStage";
import { ViewerZoomControls } from "./ViewerZoomControls";

const controlAreaSelector =
	"[data-viewer-controls], [data-viewer-chrome], [data-viewer-info], [data-viewer-zoom-controls], fieldset, aside";

type ViewerZoomAnnouncement = "Fit" | `${number}%`;

function zoomAnnouncement(
	scale: number,
	maxScale: number,
): ViewerZoomAnnouncement {
	if (!Number.isFinite(scale) || !Number.isFinite(maxScale) || scale <= 1)
		return "Fit";
	return `${Math.round((Math.min(scale, maxScale) / maxScale) * 100)}%`;
}

function isTabbable(element: HTMLElement): boolean {
	if (
		element.hasAttribute("disabled") ||
		element.tabIndex < 0 ||
		element.hidden
	)
		return false;
	let current: HTMLElement | null = element;
	while (current) {
		if (
			current.hidden ||
			current.inert ||
			current.getAttribute("aria-hidden") === "true"
		)
			return false;
		const style = window.getComputedStyle(current);
		if (style.display === "none" || style.visibility === "hidden") return false;
		current = current.parentElement;
	}
	return true;
}

interface PhotoViewerOverlayProps {
	state: ViewerState;
	assets: readonly WallAsset[];
	service: PhotoService;
	onClose: () => void;
	onSetInfoOpen: (open: boolean) => void;
	onShowControls: () => void;
	onHideControls: () => void;
	onToggleTouchControls: () => void;
	onSelectAsset: (assetId: string) => void;
	loading: boolean;
	nextCursor: string | null;
	onLoadMore: () => void;
	onRequestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	onSetWallInteraction?: (active: boolean) => void;
}

export function PhotoViewerOverlay({
	state,
	assets,
	service,
	onClose,
	onSetInfoOpen,
	onShowControls,
	onHideControls,
	onToggleTouchControls,
	onSelectAsset,
	loading,
	nextCursor,
	onLoadMore,
	onRequestNearViewportDerivatives,
	onSetWallInteraction = () => undefined,
}: PhotoViewerOverlayProps) {
	const picks = usePickList();
	const pickOrigin = usePickListOrigin();
	const backRef = useRef<HTMLButtonElement>(null);
	const dialogRef = useRef<HTMLElement>(null);
	const [previewFailedKey, setPreviewFailedKey] = useState<string | null>(null);
	const [filmstripRevealed, setFilmstripRevealed] = useState(false);
	const [announcedZoomLabel, setAnnouncedZoomLabel] =
		useState<ViewerZoomAnnouncement>("Fit");
	const [zoomAnnouncementRevision, setZoomAnnouncementRevision] = useState(0);
	const announceZoomLabel = useCallback((label: ViewerZoomAnnouncement) => {
		setAnnouncedZoomLabel(label);
		setZoomAnnouncementRevision((revision) => revision + 1);
	}, []);
	const announcementResetKey = `${state.currentAssetId ?? ""}:${state.previewGeneration}`;
	const previousAnnouncementResetKey = useRef<string | null>(null);
	const controlsFocused = useRef(false);
	const entryFocusPending = useRef(true);
	const drawableSizeRef = useRef({ width: 0, height: 0 });
	const [panning, setPanning] = useState(false);
	const [navigatorManipulating, setNavigatorManipulating] = useState(false);
	const asset = assets.find((item) => item.id === state.currentAssetId);
	const currentIndex = findViewerIndex(assets, state.currentAssetId ?? "");
	const preview = useViewerPreview({
		assets,
		currentIndex,
		previewGeneration: state.previewGeneration,
	});
	const previewFailureKey = `${asset?.id ?? ""}:${state.previewGeneration}:${preview.currentUrl ?? ""}`;
	const handlePreviewFailure = useCallback(
		(failureKey: string) => {
			if (failureKey === previewFailureKey) setPreviewFailedKey(failureKey);
		},
		[previewFailureKey],
	);
	const reportInteraction = useCallback(() => {
		onSetWallInteraction(true);
		preview.reportInteraction();
	}, [onSetWallInteraction, preview.reportInteraction]);
	const transform = useViewerTransform({
		assetId: asset?.id ?? "",
		assetRevision: state.previewGeneration,
		imageWidth: asset?.width ?? 0,
		imageHeight: asset?.height ?? 0,
		onInteraction: reportInteraction,
	});
	useEffect(() => {
		if (previousAnnouncementResetKey.current === announcementResetKey) return;
		previousAnnouncementResetKey.current = announcementResetKey;
		setAnnouncedZoomLabel("Fit");
	}, [announcementResetKey]);
	const handleDrawableSizeChange = useCallback(
		(size: { width: number; height: number }) => {
			drawableSizeRef.current = size;
			transform.setDrawableSize(size);
		},
		[transform.setDrawableSize],
	);
	const handleHideControls = useCallback(() => {
		const active = document.activeElement;
		if (
			controlsFocused.current &&
			active &&
			dialogRef.current?.contains(active)
		)
			return;
		if (active?.closest(controlAreaSelector))
			dialogRef.current?.focus({ preventScroll: true });
		onHideControls();
	}, [onHideControls]);
	const handleSelectAsset = useCallback(
		(assetId: string) => {
			setAnnouncedZoomLabel("Fit");
			reportInteraction();
			onSelectAsset(assetId);
		},
		[onSelectAsset, reportInteraction],
	);
	const handleSetInfoOpen = useCallback(
		(open: boolean) => {
			reportInteraction();
			onSetInfoOpen(open);
		},
		[onSetInfoOpen, reportInteraction],
	);
	const handleClose = useCallback(() => {
		reportInteraction();
		onClose();
	}, [onClose, reportInteraction]);
	const handleTogglePick = useCallback(() => {
		if (!asset || !pickOrigin) return;
		reportInteraction();
		void picks.toggle(asset, pickOrigin).catch(() => {});
	}, [asset, pickOrigin, picks, reportInteraction]);
	const controls = useViewerControls({
		controlsVisible: state.controlsVisible,
		onHide: handleHideControls,
		onShow: onShowControls,
		onToggleTouch: onToggleTouchControls,
	});
	const viewerCenter = useCallback(
		() => ({
			x: drawableSizeRef.current.width / 2,
			y: drawableSizeRef.current.height / 2,
		}),
		[],
	);
	const handleViewerWheel = useCallback(
		(event: WheelEvent, point: { x: number; y: number }) => {
			const intent = classifyViewerWheel({
				deltaX: event.deltaX,
				deltaY: event.deltaY,
				ctrlKey: event.ctrlKey,
				metaKey: event.metaKey,
				mode: transform.mode,
			});
			if (intent.kind === "none") return false;
			reportInteraction();
			if (!controlsFocused.current) controls.showForInput("mouse");
			if (intent.kind === "zoom")
				transform.zoomAt(
					transform.geometry.scale * (intent.zoomFactor ?? 1),
					point,
				);
			else if (intent.delta) transform.panBy(intent.delta);
			return true;
		},
		[controls, reportInteraction, transform],
	);
	const handleViewerDoubleClick = useCallback(
		(point: { x: number; y: number }) => {
			reportInteraction();
			if (!controlsFocused.current) controls.showForInput("mouse");
			if (transform.mode === "zoomed") {
				announceZoomLabel("Fit");
				transform.reset();
			} else {
				announceZoomLabel(
					zoomAnnouncement(
						transform.geometry.maxScale,
						transform.geometry.maxScale,
					),
				);
				transform.zoomAt(transform.geometry.maxScale, point);
			}
		},
		[announceZoomLabel, controls, reportInteraction, transform],
	);
	const pointInDrawable = useCallback((point: { x: number; y: number }) => {
		const stage = document.querySelector<HTMLElement>(
			"[data-testid='viewer-stage']",
		);
		if (!stage) return point;
		const bounds = stage.getBoundingClientRect();
		const computed = window.getComputedStyle(stage);
		return {
			x: point.x - bounds.left - Number.parseFloat(computed.paddingLeft || "0"),
			y: point.y - bounds.top - Number.parseFloat(computed.paddingTop || "0"),
		};
	}, []);
	const handleViewerDoubleTap = useCallback(
		(point: { x: number; y: number }) => {
			reportInteraction();
			const drawablePoint = pointInDrawable(point);
			if (transform.mode === "zoomed") {
				announceZoomLabel("Fit");
				transform.reset();
			} else {
				announceZoomLabel(
					zoomAnnouncement(
						transform.geometry.maxScale,
						transform.geometry.maxScale,
					),
				);
				transform.zoomAt(transform.geometry.maxScale, drawablePoint);
			}
		},
		[announceZoomLabel, pointInDrawable, reportInteraction, transform],
	);
	const handleDiscreteStep = useCallback(
		(direction: 1 | -1, anchor?: { x: number; y: number }) => {
			const maxScale = transform.geometry.maxScale;
			const scale = transform.geometry.scale;
			const nextScale = Math.min(
				maxScale,
				Math.max(1, scale * (direction === 1 ? 1.25 : 1 / 1.25)),
			);
			announceZoomLabel(zoomAnnouncement(nextScale, maxScale));
			transform.step(direction, anchor);
		},
		[announceZoomLabel, transform],
	);
	const handleDiscreteReset = useCallback(() => {
		announceZoomLabel("Fit");
		transform.reset();
	}, [announceZoomLabel, transform]);
	const pinchBaseScaleRef = useRef<number | null>(null);
	const handleViewerPinchStart = useCallback(() => {
		pinchBaseScaleRef.current = transform.geometry.scale;
		setPanning(true);
	}, [transform.geometry.scale]);
	const handleViewerPinchEnd = useCallback(() => {
		pinchBaseScaleRef.current = null;
		setPanning(false);
		controls.resume();
	}, [controls]);
	const handleViewerPinch = useCallback(
		(scale: number, midpoint: { x: number; y: number }) => {
			reportInteraction();
			const baseScale = pinchBaseScaleRef.current ?? transform.geometry.scale;
			pinchBaseScaleRef.current ??= baseScale;
			transform.zoomAt(baseScale * scale, pointInDrawable(midpoint));
		},
		[pointInDrawable, reportInteraction, transform.geometry, transform.zoomAt],
	);
	const handlePanStart = useCallback(() => {
		setPanning(true);
		controls.keepVisible();
	}, [controls]);
	const handlePan = useCallback(
		(delta: { x: number; y: number }) => {
			reportInteraction();
			transform.panBy(delta);
		},
		[reportInteraction, transform.panBy],
	);
	const handlePanEnd = useCallback(() => {
		setPanning(false);
		controls.resume();
	}, [controls]);
	const handleNavigatorManipulation = useCallback(
		(active: boolean) => {
			setNavigatorManipulating(active);
			if (active) controls.keepVisible();
			else controls.resume();
		},
		[controls],
	);
	const viewport = useViewerViewport();
	const zoomControlsVisible =
		state.controlsVisible && (!state.infoOpen || viewport.width >= 640);
	const gestures = useViewerGestures({
		onNavigate: (direction) => {
			const nextIndex = currentIndex + (direction === "next" ? 1 : -1);
			const nextAsset = assets[nextIndex];
			if (nextAsset) handleSelectAsset(nextAsset.id);
		},
		onTap: () => controls.toggleTouch(),
		viewportRevision: viewport.revision,
		assetRevision: state.previewGeneration,
		viewerOpen: state.open,
		viewState: transform.mode,
		onPanStart: handlePanStart,
		onPan: handlePan,
		onPanEnd: handlePanEnd,
		onPinch: handleViewerPinch,
		onPinchStart: handleViewerPinchStart,
		onPinchEnd: handleViewerPinchEnd,
		onDoubleTap: handleViewerDoubleTap,
	});

	useEffect(() => {
		if (!state.controlsVisible) setFilmstripRevealed(false);
	}, [state.controlsVisible]);

	useEffect(() => {
		if (transform.mode === "fit") setNavigatorManipulating(false);
	}, [transform.mode]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: rerun when the drawer mounts or the safe-area viewport changes.
	useLayoutEffect(() => {
		const dialog = dialogRef.current;
		if (!dialog) return;
		const drawer = dialog.querySelector<HTMLElement>(
			"[data-testid='photo-info-drawer']",
		);
		if (!drawer) {
			dialog.style.removeProperty("--viewer-info-drawer-width");
			return;
		}
		const updateDrawerWidth = () => {
			dialog.style.setProperty(
				"--viewer-info-drawer-width",
				`${drawer.getBoundingClientRect().width}px`,
			);
		};
		updateDrawerWidth();
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(updateDrawerWidth);
		observer.observe(drawer);
		return () => observer.disconnect();
	}, [state.infoOpen, viewport.revision]);

	useEffect(() => {
		entryFocusPending.current = true;
		backRef.current?.focus();
		entryFocusPending.current = false;
	}, []);

	const infoOpenRef = useRef(state.infoOpen);
	const transformModeRef = useRef(transform.mode);
	const onSetInfoOpenRef = useRef(handleSetInfoOpen);
	const discreteResetRef = useRef(handleDiscreteReset);
	const onCloseRef = useRef(handleClose);
	const escapeRef = useRef<() => void>(() => undefined);
	infoOpenRef.current = state.infoOpen;
	transformModeRef.current = transform.mode;
	onSetInfoOpenRef.current = handleSetInfoOpen;
	discreteResetRef.current = handleDiscreteReset;
	onCloseRef.current = handleClose;
	escapeRef.current = () => {
		if (infoOpenRef.current) {
			onSetInfoOpenRef.current(false);
		} else if (transformModeRef.current === "zoomed") {
			discreteResetRef.current();
		} else {
			onCloseRef.current();
		}
	};

	useEffect(() => {
		const onKeyDown = (event: globalThis.KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				escapeRef.current();
			} else if (
				event.key === "Tab" &&
				!dialogRef.current?.contains(document.activeElement)
			) {
				event.preventDefault();
				backRef.current?.focus();
			}
		};
		window.addEventListener("keydown", onKeyDown, true);
		return () => window.removeEventListener("keydown", onKeyDown, true);
	}, []);

	useEffect(() => {
		if (
			shouldLoadViewerPage(
				currentIndex,
				assets.length,
				Boolean(nextCursor),
				loading,
			)
		)
			onLoadMore();
	}, [assets.length, currentIndex, loading, nextCursor, onLoadMore]);

	if (!asset) return null;
	const currentPosition = currentIndex >= 0 ? currentIndex + 1 : 0;
	const filmstripVisible =
		state.controlsVisible && (state.filmstripVisible || filmstripRevealed);
	const handlePointerMove = (event: React.PointerEvent<HTMLElement>) => {
		reportInteraction();
		if (event.pointerType !== "touch" && !controlsFocused.current)
			controls.showForInput("mouse");
		const bounds = event.currentTarget.getBoundingClientRect();
		if (event.clientY >= bounds.bottom - 96) setFilmstripRevealed(true);
	};
	const handleKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
		reportInteraction();
		const target = event.target as HTMLElement;
		const isEditable =
			target.isContentEditable ||
			Boolean(
				target.closest("input, textarea, select, [contenteditable='true']"),
			);
		if (!isEditable && (event.key === "+" || event.key === "=")) {
			event.preventDefault();
			handleDiscreteStep(1, viewerCenter());
			return;
		}
		if (!isEditable && event.key === "-") {
			event.preventDefault();
			handleDiscreteStep(-1, viewerCenter());
			return;
		}
		if (!isEditable && event.key === "0") {
			event.preventDefault();
			handleDiscreteReset();
			return;
		}
		if (
			!isEditable &&
			(event.key === "ArrowLeft" || event.key === "ArrowRight")
		) {
			event.preventDefault();
			const nextIndex = currentIndex + (event.key === "ArrowRight" ? 1 : -1);
			const nextAsset = assets[nextIndex];
			if (nextAsset) handleSelectAsset(nextAsset.id);
			return;
		}
		trapFocus(event);
	};
	const trapFocus = (event: ReactKeyboardEvent<HTMLElement>) => {
		if (event.key !== "Tab") return;
		const focusable = [
			...(dialogRef.current?.querySelectorAll<HTMLElement>(
				"button, [href], input, select, textarea, [tabindex]",
			) ?? []),
		].filter(isTabbable);
		if (focusable.length === 0) {
			event.preventDefault();
			dialogRef.current?.focus({ preventScroll: true });
			return;
		}
		const first = focusable[0];
		const last = focusable.at(-1);
		if (!first || !last) return;
		const activeElement = event.target as HTMLElement;
		const activeIndex = focusable.indexOf(
			focusable.includes(activeElement)
				? activeElement
				: (document.activeElement as HTMLElement),
		);
		if (activeIndex < 0) {
			event.preventDefault();
			(event.shiftKey ? last : first).focus();
			return;
		}
		event.preventDefault();
		const nextIndex =
			(activeIndex + (event.shiftKey ? -1 : 1) + focusable.length) %
			focusable.length;
		focusable[nextIndex]?.focus();
	};
	const infoVisible = state.controlsVisible || state.infoOpen;
	return (
		<section
			aria-label="Photo viewer"
			aria-modal="true"
			className={styles.viewerOverlay}
			data-filmstrip-visible={filmstripVisible}
			data-viewport-revision={viewport.revision}
			style={
				{
					"--viewer-viewport-width": `${viewport.width}px`,
					"--viewer-viewport-height": `${viewport.height}px`,
				} as React.CSSProperties
			}
			onFocusCapture={(event) => {
				const target = event.target as HTMLElement;
				if (target.closest(controlAreaSelector)) {
					if (entryFocusPending.current) return;
					controlsFocused.current = true;
					controls.keepVisible();
				}
			}}
			onBlurCapture={(event) => {
				const target = event.target as HTMLElement;
				if (!target.closest(controlAreaSelector)) return;
				const next = event.relatedTarget as HTMLElement | null;
				if (!next?.closest(controlAreaSelector)) {
					controlsFocused.current = false;
					controls.resume();
				}
			}}
			onKeyDownCapture={handleKeyDown}
			onLostPointerCapture={(event) => {
				reportInteraction();
				gestures.onLostPointerCapture(event);
			}}
			onPointerCancel={(event) => {
				reportInteraction();
				gestures.onPointerCancel(event);
			}}
			onPointerDown={(event) => {
				reportInteraction();
				gestures.onPointerDown(event);
			}}
			onPointerMove={(event) => {
				handlePointerMove(event);
				gestures.onPointerMove(event);
			}}
			onPointerUp={(event) => {
				reportInteraction();
				gestures.onPointerUp(event);
			}}
			ref={dialogRef}
			role="dialog"
			tabIndex={-1}
		>
			<div
				aria-hidden={!state.controlsVisible}
				className={`${styles.viewerChrome} ${!state.controlsVisible ? styles.viewerChromeHidden : ""}`}
				data-viewer-chrome="true"
			>
				<button
					aria-label="Back to photos"
					className={styles.viewerBack}
					onClick={handleClose}
					ref={backRef}
					tabIndex={state.controlsVisible ? 0 : -1}
					type="button"
				>
					<ChevronLeft aria-hidden="true" size={22} strokeWidth={1.7} />
				</button>
				<button
					aria-expanded={state.infoOpen}
					aria-label="Photo information"
					aria-hidden={!infoVisible}
					className={`${styles.viewerInfoButton} ${!infoVisible ? styles.viewerInfoHidden : ""}`}
					data-viewer-info="true"
					onClick={() => handleSetInfoOpen(true)}
					tabIndex={infoVisible ? 0 : -1}
					type="button"
				>
					<Info aria-hidden="true" size={20} strokeWidth={1.7} />
				</button>
				<button
					aria-label={
						picks.isPicked(asset.id)
							? `Remove ${asset.displayName} from picks`
							: `Add ${asset.displayName} to picks`
					}
					aria-pressed={picks.isPicked(asset.id)}
					className={styles.viewerPick}
					disabled={!pickOrigin}
					onClick={handleTogglePick}
					tabIndex={state.controlsVisible ? 0 : -1}
					type="button"
				>
					{picks.isPicked(asset.id) ? (
						<Check aria-hidden="true" size={18} strokeWidth={2.2} />
					) : (
						<Plus aria-hidden="true" size={18} strokeWidth={2} />
					)}
					<span>{picks.isPicked(asset.id) ? "Picked" : "Pick"}</span>
				</button>
			</div>
			<ViewerStage
				asset={asset}
				baseUrl={preview.baseUrl}
				currentUrl={preview.currentUrl}
				largePreviewUnavailable={preview.largePreviewUnavailable}
				onPreviewFailure={handlePreviewFailure}
				previewGeneration={state.previewGeneration}
				service={service}
				transform={transform.geometry}
				onDoubleClick={handleViewerDoubleClick}
				onDrawableSizeChange={handleDrawableSizeChange}
				onNaturalSizeChange={transform.setNaturalSize}
				onWheel={handleViewerWheel}
				panning={panning}
				viewportHeight={viewport.height}
				viewportWidth={viewport.width}
			/>
			<div
				aria-hidden={!state.controlsVisible}
				className={`${styles.viewerControls} ${!state.controlsVisible ? styles.viewerControlsHidden : ""}`}
				data-viewer-controls="true"
			>
				<button
					aria-label="Previous photo"
					className={styles.viewerNavigate}
					disabled={currentIndex <= 0}
					onClick={() => {
						const previous = assets[currentIndex - 1];
						if (previous) handleSelectAsset(previous.id);
					}}
					tabIndex={state.controlsVisible ? 0 : -1}
					type="button"
				>
					<ChevronLeft aria-hidden="true" size={24} strokeWidth={1.7} />
				</button>
				<button
					aria-label="Next photo"
					className={styles.viewerNavigate}
					disabled={currentIndex < 0 || currentIndex >= assets.length - 1}
					onClick={() => {
						const next = assets[currentIndex + 1];
						if (next) handleSelectAsset(next.id);
					}}
					tabIndex={state.controlsVisible ? 0 : -1}
					type="button"
				>
					<ChevronRight aria-hidden="true" size={24} strokeWidth={1.7} />
				</button>
			</div>
			{filmstripVisible ? (
				<ViewerFilmstrip
					assets={assets}
					currentIndex={currentIndex}
					onRequestNearViewportDerivatives={onRequestNearViewportDerivatives}
					onSelectAsset={handleSelectAsset}
					service={service}
					viewportRevision={viewport.revision}
					viewportWidth={viewport.width}
				/>
			) : null}
			<ViewerZoomControls
				canZoomIn={transform.canZoomIn}
				canZoomOut={transform.canZoomOut}
				label={transform.zoomLabel}
				onReset={handleDiscreteReset}
				onZoomIn={() => handleDiscreteStep(1, viewerCenter())}
				onZoomOut={() => handleDiscreteStep(-1, viewerCenter())}
				visible={zoomControlsVisible}
			/>
			{transform.mode === "zoomed" ? (
				<ViewerNavigator
					assetName={asset.displayName}
					assetRevision={state.previewGeneration}
					fallbackUrl={preview.baseUrl}
					imageHeight={asset.height}
					imageUrl={preview.currentUrl}
					imageWidth={asset.width}
					interactive={!controls.coarsePointer}
					onInteraction={reportInteraction}
					onManipulationChange={handleNavigatorManipulation}
					onRecenter={transform.recenter}
					viewportRevision={viewport.revision}
					visible={state.controlsVisible || panning || navigatorManipulating}
					visibleRect={transform.geometry.visibleImageRect}
				/>
			) : null}
			<div
				aria-live="polite"
				aria-atomic="true"
				className={styles.viewerStatus}
				data-testid="viewer-status"
				role="status"
			>
				{asset.displayName}, photo {currentPosition} of {assets.length}
				{`, `}
				<span key={zoomAnnouncementRevision}>{announcedZoomLabel}</span>
				{nextCursor ? " loaded" : ""}
			</div>
			{state.infoOpen ? (
				<PhotoInfoDrawer
					asset={asset}
					largePreviewUnavailable={
						preview.largePreviewUnavailable ||
						previewFailedKey === previewFailureKey
					}
					onClose={() => handleSetInfoOpen(false)}
				/>
			) : null}
		</section>
	);
}
