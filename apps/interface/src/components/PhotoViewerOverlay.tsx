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
import { useViewerPreview } from "../viewer/useViewerPreview";
import type { ViewerState } from "../viewer/viewerReducer";
import { PhotoInfoDrawer } from "./PhotoInfoDrawer";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerStage } from "./ViewerStage";

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

	useEffect(() => {
		if (!previewFailureKey) return;
		setPreviewFailed(false);
	}, [previewFailureKey]);

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
	const filmstripVisible = state.filmstripVisible || filmstripRevealed;
	const handlePointerMove = (event: React.PointerEvent<HTMLElement>) => {
		if (event.pointerType !== "touch" && !controlsFocused.current)
			controls.showForInput("mouse");
		const bounds = event.currentTarget.getBoundingClientRect();
		if (event.clientY >= bounds.bottom - 96) setFilmstripRevealed(true);
	};
	const handlePointerUpCapture = (event: React.PointerEvent<HTMLElement>) => {
		if (event.pointerType !== "touch") return;
		const target = event.target as HTMLElement;
		if (target.closest("button, aside, fieldset")) return;
		controls.toggleTouch();
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
				'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
			) ?? []),
		];
		if (focusable.length === 0) return;
		const first = focusable[0];
		const last = focusable.at(-1);
		if (!first || !last) return;
		const activeIndex = focusable.indexOf(
			document.activeElement as HTMLElement,
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
			onFocusCapture={(event) => {
				const target = event.target as HTMLElement;
				if (target.closest("[data-viewer-controls], fieldset, aside")) {
					controlsFocused.current = true;
					controls.keepVisible();
				}
			}}
			onBlurCapture={(event) => {
				const target = event.target as HTMLElement;
				if (!target.closest("[data-viewer-controls], fieldset, aside")) return;
				const next = event.relatedTarget as HTMLElement | null;
				if (!next?.closest("[data-viewer-controls], fieldset, aside"))
					controlsFocused.current = false;
			}}
			onKeyDownCapture={handleKeyDown}
			onPointerMove={handlePointerMove}
			onPointerUpCapture={handlePointerUpCapture}
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
