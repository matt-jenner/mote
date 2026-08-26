import { useEffect, useMemo, useRef } from "react";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import { viewerFilmstripWindow } from "../viewer/photoSequence";

interface ViewerFilmstripProps {
	assets: readonly WallAsset[];
	currentIndex: number;
	service: PhotoService;
	onSelectAsset: (assetId: string) => void;
	onRequestNearViewportDerivatives: (assetIds: readonly string[]) => void;
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
}: ViewerFilmstripProps) {
	const windowRange = useMemo(
		() => viewerFilmstripWindow(assets, currentIndex, 15),
		[assets, currentIndex],
	);
	const visibleAssets = assets.slice(windowRange.start, windowRange.end);
	const scrollRevision = `${currentIndex}:${windowRange.start}:${windowRange.end}`;
	const currentButtonRef = useRef<HTMLButtonElement>(null);

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

	if (visibleAssets.length === 0) return null;
	return (
		<fieldset aria-label="Photo filmstrip" className={styles.viewerFilmstrip}>
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
							onClick={() => onSelectAsset(asset.id)}
							ref={isCurrent ? currentButtonRef : undefined}
							tabIndex={-1}
							type="button"
						>
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
