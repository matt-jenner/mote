import { FolderOpen } from "lucide-react";
import { type RefObject, useCallback } from "react";
import type { PhotoWallController } from "../app/usePhotoWall";
import { usePickList, usePickListOrigin } from "../picks/PickListContext";
import type { SourceSummary, WallAsset } from "../services/photoService";
import styles from "../styles/appShell.module.css";
import { PhotoWallCanvas } from "./PhotoWallCanvas";

interface SourceCanvasProps {
	hasOpenedFolder?: boolean;
	source: SourceSummary | null;
	chooseFolderAvailable: boolean;
	onChooseFolder: () => void;
	wall: PhotoWallController;
	regionRef: RefObject<HTMLElement | null>;
	onOpen: (assetId: string) => void;
	highlightedAssetId?: string | null;
}

export function SourceCanvas({
	hasOpenedFolder = false,
	source,
	chooseFolderAvailable,
	onChooseFolder,
	wall,
	regionRef,
	onOpen,
	highlightedAssetId = null,
}: SourceCanvasProps) {
	const picks = usePickList();
	const pickOrigin = usePickListOrigin();
	const handleTogglePick = useCallback(
		(asset: WallAsset) => {
			if (!pickOrigin) return;
			void picks.toggle(asset, pickOrigin).catch(() => {});
		},
		[pickOrigin, picks],
	);
	if (source)
		return (
			<PhotoWallCanvas
				highlightedAssetId={highlightedAssetId}
				isPicked={picks.isPicked}
				onOpen={onOpen}
				onTogglePick={handleTogglePick}
				regionRef={regionRef}
				source={source}
				wall={wall}
			/>
		);
	return (
		<main className={styles.canvas}>
			<div className={styles.emptyState}>
				<div aria-hidden="true" className={styles.emptyIcon}>
					<FolderOpen size={30} strokeWidth={1.35} />
				</div>
				{source ? (
					<>
						<h1 className={styles.canvasTitle}>Folder ready</h1>
						<p className={styles.canvasCopy}>
							Your photos will appear here when indexing is available.
						</p>
					</>
				) : (
					<>
						<h1 className={styles.canvasTitle}>
							{hasOpenedFolder
								? "Select a folder"
								: "A simple space for your photos."}
						</h1>
						<p className={styles.canvasCopy}>
							{hasOpenedFolder
								? "Choose a saved folder or open a new one."
								: "Open a folder to browse your photos without importing or reorganising them."}
						</p>
						<button
							className={`${styles.primaryButton} ${styles.welcomePrimaryButton}`}
							disabled={!chooseFolderAvailable}
							onClick={onChooseFolder}
							type="button"
						>
							{hasOpenedFolder ? "Add folder" : "Choose folder"}
						</button>
					</>
				)}
			</div>
		</main>
	);
}
