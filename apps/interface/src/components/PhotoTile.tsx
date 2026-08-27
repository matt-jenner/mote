import { CircleAlert } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { PhotoService } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import type { PositionedWallAsset } from "../wall/layoutJustifiedRows";

interface PhotoTileProps {
	positioned: PositionedWallAsset;
	service: PhotoService;
	onOpen?: (assetId: string) => void;
	highlighted?: boolean;
}

function rgb(value: number | null): string | undefined {
	if (value === null || !Number.isFinite(value)) return undefined;
	const red = (value >> 16) & 255;
	const green = (value >> 8) & 255;
	const blue = value & 255;
	return `rgb(${red}, ${green}, ${blue})`;
}

export function PhotoTile({
	positioned,
	service,
	onOpen = () => undefined,
	highlighted = false,
}: PhotoTileProps) {
	const { asset } = positioned;
	const [loaded, setLoaded] = useState(false);
	const [failed, setFailed] = useState(false);
	const [previewFailed, setPreviewFailed] = useState(false);
	const imageRef = useRef<HTMLImageElement | null>(null);
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

	useLayoutEffect(() => {
		setLoaded(false);
		setPreviewFailed(false);
		setFailed(
			(shapeState === "fallback" || asset.availability !== "available") && !url,
		);
		let cancelled = false;
		const image = imageRef.current;
		const markCachedImageLoaded = () => {
			if (cancelled || imageRef.current !== image) return;
			if (
				url &&
				image &&
				(image.getAttribute("src") === url ||
					image.src === new URL(url, image.baseURI).href) &&
				image.complete &&
				image.naturalWidth > 0
			) {
				setLoaded(true);
			}
		};
		markCachedImageLoaded();
		const frame = window.requestAnimationFrame(markCachedImageLoaded);
		if (image) {
			try {
				void image.decode().then(markCachedImageLoaded, () => undefined);
			} catch {
				// The normal load/error handlers remain responsible for this image.
			}
		}
		return () => {
			cancelled = true;
			window.cancelAnimationFrame(frame);
		};
	}, [asset.availability, shapeState, url]);

	const style = {
		width: `${positioned.width}px`,
		height: `${positioned.height}px`,
		"--tile-colour": rgb(asset.representativeRgb),
	};
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
			{url && !previewFailed ? (
				<img
					alt={asset.displayName}
					className={`${styles.imageLayer} ${loaded ? styles.imageLoaded : ""}`}
					decoding="async"
					draggable={false}
					key={url}
					onError={() => setPreviewFailed(true)}
					onLoad={(event) => {
						const image = event.currentTarget;
						if (
							image === imageRef.current &&
							(image.getAttribute("src") === url ||
								image.src === new URL(url, image.baseURI).href)
						)
							setLoaded(true);
					}}
					ref={imageRef}
					src={url}
				/>
			) : null}
			{previewFailed || asset.warning ? (
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
	const canOpen = asset.mediaKind !== "video" && Boolean(asset.wallThumbnail);
	const className = `${styles.tile} ${highlighted ? styles.tileReturnHighlight : ""}`;

	if (!canOpen) {
		return (
			<figure
				aria-label={
					asset.mediaKind === "video"
						? `${asset.displayName}; video poster only; playback unavailable`
						: undefined
				}
				className={className}
				data-asset-id={asset.id}
				data-media-kind={asset.mediaKind}
				role={asset.mediaKind === "video" ? "group" : undefined}
				style={style}
			>
				{layers}
				{asset.mediaKind === "video" ? (
					<figcaption className={styles.videoCue}>
						Video · poster only
					</figcaption>
				) : null}
			</figure>
		);
	}
	return (
		<button
			aria-label={`Open ${asset.displayName}`}
			className={className}
			data-asset-id={asset.id}
			onClick={() => onOpen(asset.id)}
			style={style}
			data-media-kind={asset.mediaKind}
			type="button"
		>
			{layers}
		</button>
	);
}
