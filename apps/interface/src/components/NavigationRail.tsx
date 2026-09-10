import { Plus } from "lucide-react";
import type { Ref } from "react";
import moteWordmark from "../../../../docs/brand/svg/mote-wordmark-light.svg";
import {
	emptySavedFolders,
	type SavedFolderSnapshot,
} from "../folders/savedFolders";
import type { PhotoServiceCapabilities } from "../services/photoService";
import styles from "../styles/appShell.module.css";
import { SavedFolderList } from "./SavedFolderList";

interface NavigationRailProps {
	savedFolders?: SavedFolderSnapshot;
	onActivate?: (id: string) => Promise<void>;
	onRename?: (id: string, label: string) => Promise<void>;
	onRemove?: (id: string) => Promise<void>;
	onChooseFolder: () => void;
	chooseFolderAvailable: boolean;
	className?: string;
	inert?: boolean;
	folderButtonRef?: Ref<HTMLButtonElement>;
	folderBrowserOpen?: boolean;
	folderSelection: PhotoServiceCapabilities["folderSelection"];
}

export function NavigationRail({
	savedFolders = emptySavedFolders(),
	onActivate = async () => {},
	onRename = async () => {},
	onRemove = async () => {},
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
			<div aria-label="Mote" className={styles.railBrand} role="img">
				<img alt="" src={moteWordmark} />
			</div>
			<div className={styles.foldersHeader}>
				<span>Folders</span>
				<button
					aria-controls={
						folderSelection === "hosted" ? "hosted-folder-browser" : undefined
					}
					aria-expanded={
						folderSelection === "hosted" ? folderBrowserOpen : undefined
					}
					aria-haspopup={folderSelection === "hosted" ? "dialog" : undefined}
					aria-label="Add folder"
					data-add-folder
					className={styles.addFolderButton}
					disabled={!chooseFolderAvailable}
					onClick={onChooseFolder}
					ref={folderButtonRef}
					type="button"
				>
					<Plus aria-hidden="true" size={16} strokeWidth={1.6} />
					<span>Add folder</span>
				</button>
			</div>
			<SavedFolderList
				snapshot={savedFolders}
				onActivate={onActivate}
				onRename={onRename}
				onRemove={onRemove}
			/>
		</nav>
	);
}
