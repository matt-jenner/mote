import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import {
	imageSourceUnavailable,
	SourceWarningBadge,
	useSourceUnavailable,
} from "../folders/SourceAvailabilityContext";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import {
	viewerFilmstripCapacity,
	viewerFilmstripWindow,
} from "../viewer/photoSequence";

interface ViewerFilmstripProps {
	assets: readonly WallAsset[];
	currentIndex: number;
	service: PhotoService;
	onSelectAsset: (assetId: string) => void;
	onRequestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	viewportWidth?: number;
	viewportRevision?: number;
}

function thumbnailUrl(service: PhotoService, asset: WallAsset): string | null {
	if (!asset.wallThumbnail) return null;
	try {
		return service.derivativeUrl(asset.wallThumbnail);
	} catch {
		return null;
	}
}

function representativeColour(asset: WallAsset): string {
	if (asset.representativeRgb === null) return "var(--canvas-elevated)";
	return `rgb(${(asset.representativeRgb >> 16) & 255}, ${(asset.representativeRgb >> 8) & 255}, ${asset.representativeRgb & 255})`;
}

export function ViewerFilmstrip({
	assets,
	currentIndex,
	service,
	onSelectAsset,
	onRequestNearViewportDerivatives,
	viewportWidth = 0,
	viewportRevision = 0,
}: ViewerFilmstripProps) {
	const sourceUnavailable = useSourceUnavailable();
	const capacity = viewerFilmstripCapacity(viewportWidth);
	const windowRange = useMemo(
		() => viewerFilmstripWindow(assets, currentIndex, (capacity - 1) / 2),
		[assets, capacity, currentIndex],
	);
	const visibleAssets = useMemo(
		() => assets.slice(windowRange.start, windowRange.end),
		[assets, windowRange.end, windowRange.start],
	);
	const scrollRevision = `${currentIndex}:${windowRange.start}:${windowRange.end}:${viewportRevision}`;
	const currentButtonRef = useRef<HTMLButtonElement>(null);
	const pendingSelectionAssetId = useRef<string | null>(null);
	const currentAssetId = assets[currentIndex]?.id ?? null;

	useEffect(() => {
		const missingIds = visibleAssets
			.filter((asset) => !asset.wallThumbnail)
			.map((asset) => asset.id);
		if (missingIds.length > 0) onRequestNearViewportDerivatives(missingIds);
	}, [onRequestNearViewportDerivatives, visibleAssets]);

	useEffect(() => {
		if (!scrollRevision) return;
		const button = currentButtonRef.current;
		if (!button) return;
		const reducedMotion =
			typeof window !== "undefined" &&
			window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
		button.scrollIntoView?.({
			behavior: reducedMotion ? "auto" : "smooth",
			block: "nearest",
			inline: "center",
		});
	}, [scrollRevision]);

	useLayoutEffect(() => {
		if (
			currentIndex < 0 ||
			!pendingSelectionAssetId.current ||
			pendingSelectionAssetId.current !== currentAssetId
		)
			return;
		pendingSelectionAssetId.current = null;
		currentButtonRef.current?.focus({ preventScroll: true });
	}, [currentAssetId, currentIndex]);

	if (visibleAssets.length === 0) return null;
	return (
		<fieldset
			aria-label="Photo filmstrip"
			className={styles.viewerFilmstrip}
			data-filmstrip-capacity={capacity}
		>
			<div className={styles.viewerFilmstripTrack}>
				{visibleAssets.map((asset) => {
					const assetIndex = assets.findIndex((item) => item.id === asset.id);
					const isCurrent = assetIndex === currentIndex;
					const url = thumbnailUrl(service, asset);
					return (
						<button
							key={asset.id}
							aria-current={isCurrent ? "true" : undefined}
							aria-label={asset.displayName}
							className={styles.viewerFilmstripItem}
							disabled={
								imageSourceUnavailable(sourceUnavailable, asset) &&
								!asset.wallThumbnail &&
								!asset.screenPreview
							}
							onClick={() => {
								pendingSelectionAssetId.current = isCurrent ? null : asset.id;
								onSelectAsset(asset.id);
							}}
							ref={isCurrent ? currentButtonRef : undefined}
							type="button"
						>
							{imageSourceUnavailable(sourceUnavailable, asset) ? (
								<SourceWarningBadge />
							) : null}
							{url ? (
								<img
									alt=""
									className={styles.viewerFilmstripImage}
									draggable={false}
									src={url}
								/>
							) : (
								<span
									aria-hidden="true"
									className={styles.viewerFilmstripColour}
									style={{ backgroundColor: representativeColour(asset) }}
								/>
							)}
						</button>
					);
				})}
			</div>
		</fieldset>
	);
}
