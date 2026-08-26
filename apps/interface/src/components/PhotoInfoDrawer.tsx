import type { WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";

interface PhotoInfoDrawerProps {
	asset: WallAsset;
	largePreviewUnavailable: boolean;
	onClose: () => void;
}

const mediaType: Record<WallAsset["mediaKind"], string> = {
	jpeg: "JPEG image",
	png: "PNG image",
	tiff: "TIFF image",
	heif: "HEIF image",
	webp: "WebP image",
	avif: "AVIF image",
	raw: "RAW image",
	video: "Video",
	unknown: "Unknown media",
};

function captureDate(capturedAtUtc: string | null): string {
	if (!capturedAtUtc) return "Date unavailable";
	const capturedAt = new Date(capturedAtUtc);
	if (Number.isNaN(capturedAt.getTime())) return "Date unavailable";
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(capturedAt);
}

function ratingLabel(rating: number | null): string {
	if (!rating) return "Unrated";
	return `${rating} ${rating === 1 ? "star" : "stars"}`;
}

export function PhotoInfoDrawer({
	asset,
	largePreviewUnavailable,
	onClose,
}: PhotoInfoDrawerProps) {
	return (
		<aside
			aria-label="Photo information"
			className={styles.viewerInfoDrawer}
			data-testid="photo-info-drawer"
		>
			<div className={styles.viewerInfoHeader}>
				<h2>Photo information</h2>
				<button
					aria-label="Close photo information"
					className={styles.viewerInfoClose}
					onClick={onClose}
					type="button"
				>
					Close
				</button>
			</div>
			<dl className={styles.viewerInfoList}>
				<div>
					<dt>Filename</dt>
					<dd>{asset.displayName}</dd>
				</div>
				<div>
					<dt>Captured</dt>
					<dd>{captureDate(asset.capturedAtUtc)}</dd>
				</div>
				<div>
					<dt>Rating</dt>
					<dd>{ratingLabel(asset.rating)}</dd>
				</div>
				<div>
					<dt>Dimensions</dt>
					<dd>
						{asset.width} × {asset.height}
					</dd>
				</div>
				<div>
					<dt>Type</dt>
					<dd>{mediaType[asset.mediaKind]}</dd>
				</div>
			</dl>
			{largePreviewUnavailable ? (
				<p className={styles.viewerInfoWarning}>Larger preview unavailable</p>
			) : null}
		</aside>
	);
}
