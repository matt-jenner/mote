import { ChevronLeft, ChevronRight } from "lucide-react";
import {
	type KeyboardEvent as ReactKeyboardEvent,
	useEffect,
	useRef,
} from "react";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import { findViewerIndex, shouldLoadViewerPage } from "../viewer/photoSequence";
import { useViewerPreview } from "../viewer/useViewerPreview";
import type { ViewerState } from "../viewer/viewerReducer";
import { ViewerFilmstrip } from "./ViewerFilmstrip";
import { ViewerStage } from "./ViewerStage";

interface PhotoViewerOverlayProps {
	state: ViewerState;
	assets: readonly WallAsset[];
	service: PhotoService;
	onClose: () => void;
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
	onSelectAsset,
	loading,
	nextCursor,
	onLoadMore,
	onRequestNearViewportDerivatives,
}: PhotoViewerOverlayProps) {
	const backRef = useRef<HTMLButtonElement>(null);
	const dialogRef = useRef<HTMLElement>(null);
	const asset = assets.find((item) => item.id === state.currentAssetId);
	const currentIndex = findViewerIndex(assets, state.currentAssetId ?? "");
	const preview = useViewerPreview({
		assets,
		currentIndex,
		previewGeneration: state.previewGeneration,
	});

	useEffect(() => {
		backRef.current?.focus();
		const onKeyDown = (event: globalThis.KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				onClose();
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
	}, [onClose]);

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
			onKeyDownCapture={handleKeyDown}
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
				previewGeneration={state.previewGeneration}
				service={service}
			/>
			<div className={styles.viewerControls}>
				<button
					aria-label="Previous photo"
					className={styles.viewerNavigate}
					disabled={currentIndex <= 0}
					onClick={() => {
						const previous = assets[currentIndex - 1];
						if (previous) onSelectAsset(previous.id);
					}}
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
					type="button"
				>
					<ChevronRight aria-hidden="true" size={24} strokeWidth={1.7} />
				</button>
			</div>
			<ViewerFilmstrip
				assets={assets}
				currentIndex={currentIndex}
				onRequestNearViewportDerivatives={onRequestNearViewportDerivatives}
				onSelectAsset={onSelectAsset}
				service={service}
			/>
		</section>
	);
}
