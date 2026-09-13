import { Check, CircleAlert, Plus } from "lucide-react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import {
	imageSourceUnavailable,
	SourceWarningBadge,
	useSourceUnavailable,
} from "../folders/SourceAvailabilityContext";
import type { PhotoService } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import type { PositionedWallAsset } from "../wall/layoutJustifiedRows";

type TilePaintPhase =
	| "placeholder"
	| "decoding"
	| "fading"
	| "interactive"
	| "failed";

interface TileRevision {
	assetId: string;
	derivativeKey: string;
	url: string;
}

interface PhotoTileProps {
	positioned: PositionedWallAsset;
	service: PhotoService;
	onOpen?: (assetId: string) => void;
	onTogglePick?: (asset: PositionedWallAsset["asset"]) => void;
	picked?: boolean;
	highlighted?: boolean;
}

function rgb(value: number | null): string | undefined {
	if (value === null || !Number.isFinite(value)) return undefined;
	const red = (value >> 16) & 255;
	const green = (value >> 8) & 255;
	const blue = value & 255;
	return `rgb(${red}, ${green}, ${blue})`;
}

function matchesImageUrl(image: HTMLImageElement, url: string): boolean {
	if (!url) return false;
	const source = image.getAttribute("src");
	if (source === url) return true;
	try {
		return image.src === new URL(url, image.baseURI).href;
	} catch {
		return false;
	}
}

export function PhotoTile({
	positioned,
	service,
	onOpen = () => undefined,
	onTogglePick,
	picked = false,
	highlighted = false,
}: PhotoTileProps) {
	const { asset } = positioned;
	const sourceUnavailable = imageSourceUnavailable(
		useSourceUnavailable(),
		asset,
	);
	const [phase, setPhase] = useState<TilePaintPhase>("placeholder");
	const [previewFailed, setPreviewFailed] = useState(false);
	const imageRef = useRef<HTMLImageElement | null>(null);
	const revisionRef = useRef(0);
	const currentRevisionRef = useRef<TileRevision | null>(null);
	const phaseRef = useRef<TilePaintPhase>("placeholder");
	const reducedMotionRef = useRef(false);
	const loadHandlerRef = useRef<((image: HTMLImageElement) => void) | null>(
		null,
	);
	const errorHandlerRef = useRef<((image: HTMLImageElement) => void) | null>(
		null,
	);
	const thumbnail = asset.wallThumbnail;
	const shapeState = asset.shapeState;
	let url: string | null = null;
	if (thumbnail) {
		try {
			url = service.derivativeUrl(thumbnail);
		} catch {
			url = null;
		}
	}
	const derivativeKey = thumbnail?.key ?? "";
	const hasThumbnail = thumbnail !== null;
	const unavailableWithoutPreview = sourceUnavailable && !url;
	const updatePhase = useCallback((next: TilePaintPhase) => {
		phaseRef.current = next;
		setPhase(next);
	}, []);

	useLayoutEffect(() => {
		const revision = ++revisionRef.current;
		const tileRevision: TileRevision = {
			assetId: asset.id,
			derivativeKey,
			url: url ?? "",
		};
		currentRevisionRef.current = tileRevision;
		const reducedMotion = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;
		reducedMotionRef.current = reducedMotion;
		let cancelled = false;
		let cachedCheckFrame: number | null = null;
		let interactiveFrame: number | null = null;
		let loadSeen = false;
		let decodeSeen = false;
		let decodeStarted = false;
		let failed = false;
		const image = imageRef.current;
		const isCurrent = () => !cancelled && revisionRef.current === revision;
		const cancelInteractiveFrame = () => {
			if (interactiveFrame === null) return;
			window.cancelAnimationFrame(interactiveFrame);
			interactiveFrame = null;
		};
		const failPreview = () => {
			if (!isCurrent() || failed) return;
			failed = true;
			cancelInteractiveFrame();
			setPreviewFailed(true);
			updatePhase("failed");
		};
		const moveToFading = () => {
			if (!isCurrent() || failed || !loadSeen || !decodeSeen) return;
			updatePhase("fading");
			if (!reducedMotion) return;
			cancelInteractiveFrame();
			interactiveFrame = window.requestAnimationFrame(() => {
				interactiveFrame = null;
				if (isCurrent()) updatePhase("interactive");
			});
		};
		const markDecoded = () => {
			if (
				!isCurrent() ||
				failed ||
				!image ||
				!matchesImageUrl(image, tileRevision.url)
			)
				return;
			decodeSeen = true;
			if (!loadSeen && image.complete && image.naturalWidth > 0)
				loadSeen = true;
			moveToFading();
		};
		const startDecode = () => {
			if (
				decodeStarted ||
				!isCurrent() ||
				!image ||
				!matchesImageUrl(image, tileRevision.url)
			)
				return;
			decodeStarted = true;
			try {
				void image.decode().then(markDecoded, failPreview);
			} catch {
				failPreview();
			}
		};
		const markLoaded = () => {
			if (
				!isCurrent() ||
				failed ||
				!image ||
				!matchesImageUrl(image, tileRevision.url)
			)
				return;
			loadSeen = true;
			startDecode();
			moveToFading();
		};
		const markCachedImageLoaded = () => {
			if (
				!isCurrent() ||
				failed ||
				!image ||
				!matchesImageUrl(image, tileRevision.url)
			)
				return;
			if (image.complete && image.naturalWidth > 0) markLoaded();
		};
		const loadHandler = (currentImage: HTMLImageElement) => {
			if (
				currentImage === image &&
				matchesImageUrl(currentImage, tileRevision.url)
			)
				markLoaded();
		};
		const errorHandler = (currentImage: HTMLImageElement) => {
			if (
				currentImage === image &&
				matchesImageUrl(currentImage, tileRevision.url)
			)
				failPreview();
		};
		loadHandlerRef.current = loadHandler;
		errorHandlerRef.current = errorHandler;

		setPreviewFailed(false);
		if ((shapeState === "fallback" || unavailableWithoutPreview) && !url) {
			updatePhase("failed");
		} else if (!url || !hasThumbnail) {
			updatePhase("placeholder");
		} else {
			updatePhase("decoding");
			markCachedImageLoaded();
			cachedCheckFrame = window.requestAnimationFrame(() => {
				cachedCheckFrame = null;
				markCachedImageLoaded();
			});
			startDecode();
		}

		return () => {
			cancelled = true;
			if (loadHandlerRef.current === loadHandler) loadHandlerRef.current = null;
			if (errorHandlerRef.current === errorHandler)
				errorHandlerRef.current = null;
			if (cachedCheckFrame !== null)
				window.cancelAnimationFrame(cachedCheckFrame);
			cancelInteractiveFrame();
		};
	}, [
		unavailableWithoutPreview,
		asset.id,
		derivativeKey,
		hasThumbnail,
		shapeState,
		url,
		updatePhase,
	]);

	const style = {
		width: `${positioned.width}px`,
		height: `${positioned.height}px`,
		"--tile-colour": rgb(asset.representativeRgb),
	};
	const failed = phase === "failed" && !url;
	const layers = failed ? (
		<div className={styles.fallback} data-testid="photo-fallback">
			<span aria-hidden="true" className={styles.fallbackMark}>
				?
			</span>
			<span>File unavailable</span>
		</div>
	) : (
		<>
			<div aria-hidden="true" className={styles.neutralLayer} />
			<div aria-hidden="true" className={styles.colourLayer} />
			{url ? (
				<img
					alt={asset.displayName}
					className={`${styles.imageLayer} ${phase === "fading" || phase === "interactive" ? styles.imageLoaded : ""}`}
					decoding="async"
					draggable={false}
					onError={(event) => errorHandlerRef.current?.(event.currentTarget)}
					onLoad={(event) => loadHandlerRef.current?.(event.currentTarget)}
					ref={imageRef}
					src={url}
					onTransitionEnd={(event) => {
						if (
							event.propertyName === "opacity" &&
							!reducedMotionRef.current &&
							phaseRef.current === "fading" &&
							currentRevisionRef.current?.assetId === asset.id &&
							currentRevisionRef.current?.derivativeKey === derivativeKey &&
							currentRevisionRef.current?.url === url &&
							matchesImageUrl(event.currentTarget, url)
						)
							updatePhase("interactive");
					}}
				/>
			) : null}
			{!sourceUnavailable && (previewFailed || asset.warning) ? (
				<span
					aria-label={
						previewFailed
							? "Photo preview unavailable"
							: "Photo preview warning"
					}
					className={styles.tileWarning}
					role="img"
					title={
						previewFailed
							? "Photo preview unavailable"
							: "Photo preview warning"
					}
				>
					<CircleAlert aria-hidden="true" size={14} strokeWidth={1.7} />
				</span>
			) : null}
		</>
	);
	const className = `${styles.tile} ${picked ? styles.tilePicked : ""} ${highlighted ? styles.tileReturnHighlight : ""}`;
	const canOpen =
		asset.mediaKind !== "video" &&
		phase === "interactive" &&
		Boolean(thumbnail) &&
		!previewFailed;
	const canTogglePick = picked || phase === "interactive";

	return (
		<figure
			className={className}
			data-asset-id={asset.id}
			data-controls-stacked={positioned.width < 82 ? "true" : undefined}
			data-media-kind={asset.mediaKind}
			title={
				sourceUnavailable
					? "Source unavailable. Showing cached image."
					: undefined
			}
			style={style}
			tabIndex={-1}
		>
			{layers}
			{sourceUnavailable ? <SourceWarningBadge /> : null}
			{onTogglePick && asset.mediaKind !== "video" && canTogglePick ? (
				<button
					aria-label={
						picked
							? `Remove ${asset.displayName} from picks`
							: `Add ${asset.displayName} to picks`
					}
					aria-pressed={picked}
					className={styles.tilePickButton}
					onClick={(event) => {
						event.stopPropagation();
						onTogglePick(asset);
					}}
					type="button"
				>
					{picked ? (
						<Check aria-hidden="true" size={18} strokeWidth={2.2} />
					) : (
						<Plus aria-hidden="true" size={18} strokeWidth={2} />
					)}
				</button>
			) : null}
			{canOpen ? (
				<button
					aria-label={`Open ${asset.displayName}`}
					aria-description={
						sourceUnavailable
							? "Source unavailable. Showing cached image."
							: undefined
					}
					className={`${styles.tileOpenOverlay} ${highlighted ? styles.tileReturnHighlight : ""}`}
					onClick={() => onOpen(asset.id)}
					type="button"
				/>
			) : null}
		</figure>
	);
}
