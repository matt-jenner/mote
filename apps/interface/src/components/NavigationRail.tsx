import { FolderOpen } from "lucide-react";
import type { Ref } from "react";
import type { PhotoServiceCapabilities } from "../services/photoService";
import styles from "../styles/appShell.module.css";

interface NavigationRailProps {
	onChooseFolder: () => void;
	chooseFolderAvailable: boolean;
	className?: string;
	inert?: boolean;
	folderButtonRef?: Ref<HTMLButtonElement>;
	folderBrowserOpen?: boolean;
	folderSelection: PhotoServiceCapabilities["folderSelection"];
}

export function NavigationRail({
	onChooseFolder,
	chooseFolderAvailable,
	className = "",
	inert = false,
	folderButtonRef,
	folderBrowserOpen = false,
	folderSelection,
}: NavigationRailProps) {
	return (
		<nav
			aria-label="Sources"
			className={`${styles.rail} ${className}`}
			inert={inert}
		>
			<button
				aria-controls={
					folderSelection === "hosted" ? "hosted-folder-browser" : undefined
				}
				aria-expanded={
					folderSelection === "hosted" ? folderBrowserOpen : undefined
				}
				aria-haspopup={folderSelection === "hosted" ? "dialog" : undefined}
				aria-label="Folders"
				className={`${styles.railButton} ${styles.railButtonSelected}`}
				disabled={!chooseFolderAvailable}
				onClick={onChooseFolder}
				ref={folderButtonRef}
				type="button"
			>
				<FolderOpen aria-hidden="true" size={21} strokeWidth={1.6} />
				<span className={styles.railLabel}>Folders</span>
			</button>
		</nav>
	);
}
