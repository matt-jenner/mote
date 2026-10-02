import { AlertTriangle, Folder, X } from "lucide-react";
import type { ReactNode } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import {
	imageSourceUnavailable,
	useSourceUnavailable,
	visibleImageWarning,
} from "../folders/SourceAvailabilityContext";
import type { PickItem } from "../picks/pickList";
import styles from "../styles/picksPanel.module.css";

interface PickRowProps {
	item: PickItem;
	onOpen?: (assetId: string, launchTarget: HTMLElement) => void;
	onRemove: (assetId: string) => void;
	action?: ReactNode;
}

function thumbnailUrl(
	item: PickItem,
	derivativeUrl: ReturnType<typeof usePhotoService>["derivativeUrl"],
): string | null {
	const reference = item.asset?.wallThumbnail ?? item.asset?.screenPreview;
	if (!reference) return null;
	try {
		return derivativeUrl(reference);
	} catch {
		return null;
	}
}

export function PickRow({ item, onOpen, onRemove, action }: PickRowProps) {
	const service = usePhotoService();
	const url = thumbnailUrl(item, service.derivativeUrl);
	const sourceUnavailableContext = useSourceUnavailable();
	const sourceUnavailable =
		item.asset === null ||
		imageSourceUnavailable(sourceUnavailableContext, item.asset);
	const visibleWarning = item.asset
		? visibleImageWarning(sourceUnavailableContext, item.asset)
		: null;
	const warning = sourceUnavailable
		? "Source unavailable"
		: visibleWarning
			? "Preview unavailable"
			: null;
	const downloadUrl =
		!sourceUnavailable && service.capabilities.originalAction === "download"
			? service.originalDownloadUrl(item.assetId)
			: null;

	return (
		<li className={styles.pickRow}>
			{item.asset && onOpen ? (
				<button
					aria-label={`Review ${item.asset.displayName}`}
					className={`${styles.thumbnail} ${styles.thumbnailButton}`}
					data-pick-review-asset-id={item.assetId}
					onClick={(event) => onOpen(item.assetId, event.currentTarget)}
					type="button"
				>
					{url ? <img alt="" src={url} /> : null}
				</button>
			) : (
				<div aria-hidden="true" className={styles.thumbnail}>
					{url ? <img alt="" src={url} /> : null}
				</div>
			)}
			<div className={styles.rowDetails}>
				<span className={styles.filename}>
					{item.asset?.displayName ?? item.assetId}
				</span>
				<span className={styles.sourceLabel}>
					<Folder aria-hidden="true" size={15} strokeWidth={1.7} />
					{item.sourceLabel}
				</span>
				{warning ? (
					<span className={styles.sourceWarning} role="status">
						<AlertTriangle aria-hidden="true" size={14} strokeWidth={1.8} />
						{warning}
					</span>
				) : null}
				{downloadUrl ? (
					<a className={styles.downloadLink} href={downloadUrl}>
						Download original
					</a>
				) : null}
				{action}
			</div>
			<button
				aria-label={`Remove ${item.asset?.displayName ?? item.assetId}`}
				className={styles.removeButton}
				onClick={() => onRemove(item.assetId)}
				type="button"
			>
				<X aria-hidden="true" size={20} strokeWidth={1.8} />
			</button>
		</li>
	);
}
