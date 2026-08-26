import { ArrowDownAZ, ArrowUpAZ } from "lucide-react";
import type { WallProgress } from "../app/usePhotoWall";
import type { SortDirection } from "../services/photoService";
import styles from "../styles/photoWall.module.css";

interface WallToolbarProps {
	direction: SortDirection;
	onDirectionChange: (direction: SortDirection) => void;
	status: string;
	progress?: WallProgress;
	onRetry?: () => void;
	retryable?: boolean;
}

export function WallToolbar({
	direction,
	onDirectionChange,
	status,
	progress,
	onRetry,
	retryable = false,
}: WallToolbarProps) {
	return (
		<div className={styles.wallToolbar}>
			<div aria-live="polite" className={styles.progress} role="status">
				{status}
			</div>
			{progress ? (
				<progress
					aria-label="Photo preview progress"
					className={styles.progressTrack}
					max={progress.max ?? undefined}
					value={progress.value ?? undefined}
				/>
			) : null}
			{retryable && onRetry ? (
				<button className={styles.retryButton} onClick={onRetry} type="button">
					Retry
				</button>
			) : null}
			<fieldset aria-label="Photo order" className={styles.sortControls}>
				<button
					aria-pressed={direction === "oldestFirst"}
					className={styles.sortButton}
					onClick={() => onDirectionChange("oldestFirst")}
					type="button"
				>
					<ArrowDownAZ aria-hidden="true" size={16} strokeWidth={1.7} />
					Oldest first
				</button>
				<button
					aria-pressed={direction === "newestFirst"}
					className={styles.sortButton}
					onClick={() => onDirectionChange("newestFirst")}
					type="button"
				>
					<ArrowUpAZ aria-hidden="true" size={16} strokeWidth={1.7} />
					Newest first
				</button>
			</fieldset>
		</div>
	);
}
