import { FolderOpen } from "lucide-react";
import type { PhotoWallController } from "../app/usePhotoWall";
import type { SourceSummary } from "../services/photoService";
import styles from "../styles/appShell.module.css";
import { PhotoWallCanvas } from "./PhotoWallCanvas";

interface SourceCanvasProps {
	source: SourceSummary | null;
	chooseFolderAvailable: boolean;
	onChooseFolder: () => void;
	wall: PhotoWallController;
}

export function SourceCanvas({
	source,
	chooseFolderAvailable,
	onChooseFolder,
	wall,
}: SourceCanvasProps) {
	if (source) return <PhotoWallCanvas source={source} wall={wall} />;
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
							Your photos, in one quiet place
						</h1>
						<p className={styles.canvasCopy}>
							Choose a folder to start this photo library.
						</p>
						<button
							className={styles.primaryButton}
							disabled={!chooseFolderAvailable}
							onClick={onChooseFolder}
							type="button"
						>
							Choose Folder
						</button>
					</>
				)}
			</div>
		</main>
	);
}
