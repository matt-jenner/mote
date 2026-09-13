import {
	ArrowDownAZ,
	ArrowUpAZ,
	FolderTree,
	SlidersHorizontal,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { WallProgress } from "../app/usePhotoWall";
import type { GalleryScope, SortDirection } from "../services/photoService";
import styles from "../styles/photoWall.module.css";

interface WallToolbarProps {
	direction: SortDirection;
	onDirectionChange: (direction: SortDirection) => void;
	galleryScope: GalleryScope;
	onGalleryScopeChange: (scope: GalleryScope) => void;
	status: string;
	progress?: WallProgress;
	onRetry?: () => void;
	retryable?: boolean;
}

export function WallToolbar({
	direction,
	onDirectionChange,
	galleryScope,
	onGalleryScopeChange,
	status,
	progress,
	onRetry,
	retryable = false,
}: WallToolbarProps) {
	const [mobileOptionsOpen, setMobileOptionsOpen] = useState(false);
	const mobileOptionsRef = useRef<HTMLDivElement>(null);
	const mobileOptionsButtonRef = useRef<HTMLButtonElement>(null);

	useEffect(() => {
		if (!mobileOptionsOpen) return;
		const closeOutside = (event: PointerEvent) => {
			if (
				event.target instanceof Node &&
				!mobileOptionsRef.current?.contains(event.target)
			)
				setMobileOptionsOpen(false);
		};
		const closeOnEscape = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			setMobileOptionsOpen(false);
			mobileOptionsButtonRef.current?.focus();
		};
		window.addEventListener("pointerdown", closeOutside);
		window.addEventListener("keydown", closeOnEscape);
		return () => {
			window.removeEventListener("pointerdown", closeOutside);
			window.removeEventListener("keydown", closeOnEscape);
		};
	}, [mobileOptionsOpen]);

	const chooseDirection = (next: SortDirection) => {
		onDirectionChange(next);
		setMobileOptionsOpen(false);
	};
	const toggleScope = () => {
		onGalleryScopeChange(
			galleryScope === "includeSubfolders"
				? "currentFolder"
				: "includeSubfolders",
		);
		setMobileOptionsOpen(false);
	};

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
			<div className={styles.desktopViewOptions}>
				<button
					aria-label="Include subfolders"
					aria-pressed={galleryScope === "includeSubfolders"}
					className={styles.scopeButton}
					data-tooltip="Include subfolders"
					onClick={toggleScope}
					type="button"
				>
					<FolderTree aria-hidden="true" size={16} strokeWidth={1.7} />
				</button>
				<fieldset aria-label="Photo order" className={styles.sortControls}>
					<button
						aria-label="Oldest first"
						aria-pressed={direction === "oldestFirst"}
						className={styles.sortButton}
						data-tooltip="Oldest first"
						onClick={() => onDirectionChange("oldestFirst")}
						type="button"
					>
						<ArrowDownAZ aria-hidden="true" size={16} strokeWidth={1.7} />
					</button>
					<button
						aria-label="Newest first"
						aria-pressed={direction === "newestFirst"}
						className={styles.sortButton}
						data-tooltip="Newest first"
						onClick={() => onDirectionChange("newestFirst")}
						type="button"
					>
						<ArrowUpAZ aria-hidden="true" size={16} strokeWidth={1.7} />
					</button>
				</fieldset>
			</div>
			<div className={styles.mobileViewOptions} ref={mobileOptionsRef}>
				<button
					aria-expanded={mobileOptionsOpen}
					aria-haspopup="dialog"
					aria-label="View options"
					className={styles.viewOptionsButton}
					onClick={() => setMobileOptionsOpen((open) => !open)}
					ref={mobileOptionsButtonRef}
					type="button"
				>
					<SlidersHorizontal aria-hidden="true" size={18} strokeWidth={1.7} />
				</button>
				{mobileOptionsOpen ? (
					<div
						aria-label="View options"
						className={styles.viewOptionsMenu}
						role="dialog"
					>
						<fieldset
							aria-label="Photo order"
							className={styles.viewOptionsGroup}
						>
							<legend>Photo order</legend>
							<button
								aria-pressed={direction === "oldestFirst"}
								onClick={() => chooseDirection("oldestFirst")}
								type="button"
							>
								<ArrowDownAZ aria-hidden="true" size={17} strokeWidth={1.7} />
								Oldest first
							</button>
							<button
								aria-pressed={direction === "newestFirst"}
								onClick={() => chooseDirection("newestFirst")}
								type="button"
							>
								<ArrowUpAZ aria-hidden="true" size={17} strokeWidth={1.7} />
								Newest first
							</button>
						</fieldset>
						<button
							aria-pressed={galleryScope === "includeSubfolders"}
							className={styles.viewOptionsScope}
							onClick={toggleScope}
							type="button"
						>
							<FolderTree aria-hidden="true" size={17} strokeWidth={1.7} />
							Include subfolders
						</button>
					</div>
				) : null}
			</div>
		</div>
	);
}
