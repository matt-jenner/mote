import { FolderOpen } from "lucide-react";
import styles from "../styles/appShell.module.css";

interface NavigationRailProps {
	onChooseFolder: () => void;
	chooseFolderAvailable: boolean;
	className?: string;
	inert?: boolean;
}

export function NavigationRail({
	onChooseFolder,
	chooseFolderAvailable,
	className = "",
	inert = false,
}: NavigationRailProps) {
	return (
		<nav
			aria-label="Sources"
			className={`${styles.rail} ${className}`}
			inert={inert}
		>
			<button
				aria-label="Folders"
				className={`${styles.railButton} ${styles.railButtonSelected}`}
				disabled={!chooseFolderAvailable}
				onClick={onChooseFolder}
				type="button"
			>
				<FolderOpen aria-hidden="true" size={21} strokeWidth={1.6} />
				<span className={styles.railLabel}>Folders</span>
			</button>
		</nav>
	);
}
