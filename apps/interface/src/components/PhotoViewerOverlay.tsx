import { ChevronLeft, ChevronRight, Info } from "lucide-react";
import {
	type KeyboardEvent as ReactKeyboardEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import { findViewerIndex, shouldLoadViewerPage } from "../viewer/photoSequence";
import { useViewerControls } from "../viewer/useViewerControls";
import { useViewerGestures } from "../viewer/useViewerGestures";
import { useViewerPreview } from "../viewer/useViewerPreview";
import type { ViewerState } from "../viewer/viewerReducer";
import { useViewerViewport } from "../viewer/viewerViewport";
import { PhotoInfoDrawer } from "./PhotoInfoDrawer";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerStage } from "./ViewerStage";

const controlAreaSelector = "[data-viewer-controls], fieldset, aside";

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
}: PhotoViewerOverlayProps) {
	const backRef = useRef<HTMLButtonElement>(null);
	const dialogRef = useRef<HTMLElement>(null);
	const [previewFailed, setPreviewFailed] = useState(false);
	const [filmstripRevealed, setFilmstripRevealed] = useState(false);
	const controlsFocused = useRef(false);
	const handlePreviewFailure = useCallback(() => setPreviewFailed(true), []);
	const asset = assets.find((item) => item.id === state.currentAssetId);
	const previewFailureKey = `${asset?.id ?? ""}:${state.previewGeneration}`;
	const currentIndex = findViewerIndex(assets, state.currentAssetId ?? "");
	const preview = useViewerPreview({
		assets,
		currentIndex,
		previewGeneration: state.previewGeneration,
	});
	const controls = useViewerControls({
		controlsVisible: state.controlsVisible,
		onHide: onHideControls,
		onShow: onShowControls,
		onToggleTouch: onToggleTouchControls,
	});
	const viewport = useViewerViewport();
	const gestures = useViewerGestures({
		onNavigate: (direction) => {
			const nextIndex = currentIndex + (direction === "next" ? 1 : -1);
			const nextAsset = assets[nextIndex];
			if (nextAsset) onSelectAsset(nextAsset.id);
		},
		onTap: () => controls.toggleTouch(),
		viewportRevision: viewport.revision,
		viewerOpen: state.open,
		viewState: "fit",
	});

	useEffect(() => {
		if (!previewFailureKey) return;
		setPreviewFailed(false);
	}, [previewFailureKey]);

	useEffect(() => {
		if (!state.controlsVisible) setFilmstripRevealed(false);
	}, [state.controlsVisible]);

	useEffect(() => {
		backRef.current?.focus();
		const onKeyDown = (event: globalThis.KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				if (state.infoOpen) onSetInfoOpen(false);
				else onClose();
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
	}, [onClose, onSetInfoOpen, state.infoOpen]);

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
		if (event.pointerType !== "touch" && !controlsFocused.current)
			controls.showForInput("mouse");
		const bounds = event.currentTarget.getBoundingClientRect();
		if (event.clientY >= bounds.bottom - 96) setFilmstripRevealed(true);
	};
	const handleKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
		const target = event.target as HTMLElement;
		const isEditable =
			target.isContentEditable ||
			Boolean(
				target.closest("input, textarea, select, [contenteditable='true']"),
			);
		if (
			!isEditable &&
			(event.key === "ArrowLeft" || event.key === "ArrowRight")
		) {
			event.preventDefault();
			const nextIndex = currentIndex + (event.key === "ArrowRight" ? 1 : -1);
			const nextAsset = assets[nextIndex];
			if (nextAsset) onSelectAsset(nextAsset.id);
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
		if (focusable.length === 0) return;
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
	return (
		<section
			aria-label="Photo viewer"
			aria-modal="true"
			className={styles.viewerOverlay}
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
			onLostPointerCapture={gestures.onLostPointerCapture}
			onPointerCancel={gestures.onPointerCancel}
			onPointerDown={gestures.onPointerDown}
			onPointerMove={(event) => {
				handlePointerMove(event);
				gestures.onPointerMove(event);
			}}
			onPointerUp={gestures.onPointerUp}
			ref={dialogRef}
			role="dialog"
		>
			<div className={styles.viewerChrome}>
				<button
					aria-label="Back to photos"
					className={styles.viewerBack}
					onClick={onClose}
					ref={backRef}
					type="button"
				>
					<ChevronLeft aria-hidden="true" size={22} strokeWidth={1.7} />
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
						if (previous) onSelectAsset(previous.id);
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
						if (next) onSelectAsset(next.id);
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
					onSelectAsset={onSelectAsset}
					service={service}
					viewportRevision={viewport.revision}
					viewportWidth={viewport.width}
				/>
			) : null}
			<div
				aria-live="polite"
				className={styles.viewerStatus}
				data-testid="viewer-status"
				role="status"
			>
				{asset.displayName}, photo {currentPosition} of {assets.length}
				{nextCursor ? " loaded" : ""}
			</div>
			<button
				aria-expanded={state.infoOpen}
				aria-label="Photo information"
				className={styles.viewerInfoButton}
				onClick={() => onSetInfoOpen(true)}
				type="button"
			>
				<Info aria-hidden="true" size={20} strokeWidth={1.7} />
			</button>
			{state.infoOpen ? (
				<PhotoInfoDrawer
					asset={asset}
					largePreviewUnavailable={
						preview.largePreviewUnavailable || previewFailed
					}
					onClose={() => onSetInfoOpen(false)}
				/>
			) : null}
		</section>
	);
}
