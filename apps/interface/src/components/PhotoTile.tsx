import { useEffect, useRef, useState } from "react";
import type { PhotoService } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import type { PositionedWallAsset } from "../wall/layoutJustifiedRows";

interface PhotoTileProps {
	positioned: PositionedWallAsset;
	service: PhotoService;
}

function rgb(value: number | null): string | undefined {
	if (value === null || !Number.isFinite(value)) return undefined;
	const red = (value >> 16) & 255;
	const green = (value >> 8) & 255;
	const blue = value & 255;
	return `rgb(${red}, ${green}, ${blue})`;
}

export function PhotoTile({ positioned, service }: PhotoTileProps) {
	const { asset } = positioned;
	const [loaded, setLoaded] = useState(false);
	const [failed, setFailed] = useState(asset.shapeState === "fallback");
	const thumbnail = asset.wallThumbnail;
	const shapeState = asset.shapeState;
	const thumbnailKey = thumbnail?.key;
	const previousThumbnailKey = useRef(thumbnailKey);
	let url: string | null = null;
	if (thumbnail) {
		try {
			url = service.derivativeUrl(thumbnail);
		} catch {
			url = null;
		}
	}

	useEffect(() => {
		if (previousThumbnailKey.current !== thumbnailKey) setLoaded(false);
		previousThumbnailKey.current = thumbnailKey;
		setFailed(shapeState === "fallback");
	}, [shapeState, thumbnailKey]);

	const style = {
		width: `${positioned.width}px`,
		height: `${positioned.height}px`,
		"--tile-colour": rgb(asset.representativeRgb),
	};

	return (
		<figure className={styles.tile} data-asset-id={asset.id} style={style}>
			{failed ? (
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
							className={`${styles.imageLayer} ${loaded ? styles.imageLoaded : ""}`}
							decoding="async"
							draggable={false}
							onError={() => setFailed(true)}
							onLoad={() => setLoaded(true)}
							src={url}
						/>
					) : null}
				</>
			)}
			<figcaption className={styles.visuallyHidden}>
				{asset.displayName}
			</figcaption>
		</figure>
	);
}
