import { AlertTriangle, Folder, X } from "lucide-react";
import type { ReactNode } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import type { PickItem } from "../picks/pickList";
import styles from "../styles/picksPanel.module.css";

interface PickRowProps {
	item: PickItem;
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

export function PickRow({ item, onRemove, action }: PickRowProps) {
	const service = usePhotoService();
	const url = thumbnailUrl(item, service.derivativeUrl);
	const unavailable =
		item.asset === null ||
		item.asset.availability !== "available" ||
		item.asset.warning !== null;

	return (
		<li className={styles.pickRow}>
			<div aria-hidden="true" className={styles.thumbnail}>
				{url ? <img alt="" src={url} /> : null}
			</div>
			<div className={styles.rowDetails}>
				<span className={styles.filename}>
					{item.asset?.displayName ?? item.assetId}
				</span>
				<span className={styles.sourceLabel}>
					<Folder aria-hidden="true" size={15} strokeWidth={1.7} />
					{item.sourceLabel}
				</span>
				{unavailable ? (
					<span className={styles.sourceWarning} role="status">
						<AlertTriangle aria-hidden="true" size={14} strokeWidth={1.8} />
						Source unavailable
					</span>
				) : null}
			</div>
			{action}
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
