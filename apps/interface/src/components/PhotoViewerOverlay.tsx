import { ChevronLeft } from "lucide-react";
import { useEffect, useRef } from "react";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import type { ViewerState } from "../viewer/viewerReducer";
import { ViewerStage } from "./ViewerStage";

interface PhotoViewerOverlayProps {
	state: ViewerState;
	assets: readonly WallAsset[];
	service: PhotoService;
	onClose: () => void;
}

export function PhotoViewerOverlay({
	state,
	assets,
	service,
	onClose,
}: PhotoViewerOverlayProps) {
	const backRef = useRef<HTMLButtonElement>(null);
	const asset = assets.find((item) => item.id === state.currentAssetId);

	useEffect(() => {
		backRef.current?.focus();
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				onClose();
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [onClose]);

	if (!asset) return null;
	return (
		<section
			aria-label="Photo viewer"
			aria-modal="true"
			className={styles.viewerOverlay}
			role="dialog"
		>
			<div className={styles.viewerChrome}>
				<button
					aria-label="Back to photos"
					className={styles.viewerBack}
					onClick={onClose}
					ref={backRef}
					type="button"
				>
					<ChevronLeft aria-hidden="true" size={22} strokeWidth={1.7} />
				</button>
			</div>
			<ViewerStage asset={asset} service={service} />
		</section>
	);
}
