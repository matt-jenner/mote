import { TriangleAlert } from "lucide-react";
import { createContext, useContext } from "react";
import type { WallAsset } from "../services/photoService";
import styles from "../styles/folderWarning.module.css";
export const SourceUnavailableContext = createContext<boolean | null>(null);
export const useSourceUnavailable = () => useContext(SourceUnavailableContext);
export function SourceWarningBadge() {
	return (
		<span
			role="img"
			aria-label="Source unavailable. Showing cached image."
			title="Source unavailable. Showing cached image."
			className={styles.badge}
		>
			<TriangleAlert aria-hidden="true" size={16} strokeWidth={1.8} />
		</span>
	);
}

export const imageSourceUnavailable = (
	context: boolean | null,
	asset: WallAsset,
) =>
	context === true ||
	(asset.availability !== "available" &&
		(context !== false || asset.warning?.code !== "sourceUnavailable"));

export const visibleImageWarning = (
	context: boolean | null,
	asset: WallAsset,
) =>
	context === false && asset.warning?.code === "sourceUnavailable"
		? null
		: asset.warning;
