import { Minus, Plus } from "lucide-react";
import styles from "../styles/photoViewer.module.css";

interface ViewerZoomControlsProps {
	label: "Fit" | `${number}%`;
	canZoomIn: boolean;
	canZoomOut: boolean;
	visible: boolean;
	onZoomIn: () => void;
	onZoomOut: () => void;
	onReset: () => void;
}

export function ViewerZoomControls({
	label,
	canZoomIn,
	canZoomOut,
	visible,
	onZoomIn,
	onZoomOut,
	onReset,
}: ViewerZoomControlsProps) {
	const tabIndex = visible ? 0 : -1;
	return (
		<div
			aria-hidden={!visible}
			className={`${styles.viewerZoomControls} ${!visible ? styles.viewerZoomControlsHidden : ""}`}
			data-viewer-zoom-controls="true"
		>
			<button
				aria-label="Zoom out"
				className={styles.viewerZoomButton}
				disabled={!canZoomOut}
				onClick={onZoomOut}
				tabIndex={tabIndex}
				type="button"
			>
				<Minus aria-hidden="true" size={18} strokeWidth={1.8} />
			</button>
			<button
				aria-label="Reset zoom"
				className={styles.viewerZoomReset}
				onClick={onReset}
				tabIndex={tabIndex}
				type="button"
			>
				{label}
			</button>
			<button
				aria-label="Zoom in"
				className={styles.viewerZoomButton}
				disabled={!canZoomIn}
				onClick={onZoomIn}
				tabIndex={tabIndex}
				type="button"
			>
				<Plus aria-hidden="true" size={18} strokeWidth={1.8} />
			</button>
		</div>
	);
}
