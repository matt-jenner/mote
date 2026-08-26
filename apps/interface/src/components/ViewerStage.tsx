import { useLayoutEffect, useRef, useState } from "react";
import type {
	DerivativeReference,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import styles from "../styles/photoViewer.module.css";

interface ViewerStageProps {
	asset: WallAsset;
	service: PhotoService;
}

export interface ViewerFrameRect {
	width: number;
	height: number;
}

export function fitViewerFrame(
	containerWidth: number,
	containerHeight: number,
	assetWidth: number,
	assetHeight: number,
): ViewerFrameRect {
	if (
		containerWidth <= 0 ||
		containerHeight <= 0 ||
		assetWidth <= 0 ||
		assetHeight <= 0
	)
		return { width: 0, height: 0 };
	const scale = Math.min(
		containerWidth / assetWidth,
		containerHeight / assetHeight,
	);
	return {
		width: assetWidth * scale,
		height: assetHeight * scale,
	};
}

function derivativeUrl(service: PhotoService, asset: WallAsset) {
	const references: Array<
		[DerivativeReference | null, "screenPreview" | "wallThumbnail"]
	> = [
		[asset.screenPreview, "screenPreview"],
		[asset.wallThumbnail, "wallThumbnail"],
	];
	return references.flatMap(([reference, layer]) => {
		if (!reference) return [];
		try {
			return [{ layer, url: service.derivativeUrl(reference) }];
		} catch {
			return [];
		}
	});
}

export function ViewerStage({ asset, service }: ViewerStageProps) {
	const stageRef = useRef<HTMLDivElement>(null);
	const [stageSize, setStageSize] = useState({ width: 0, height: 0 });
	const candidates = derivativeUrl(service, asset);
	const [candidateIndex, setCandidateIndex] = useState(0);
	const candidateKey = `${asset.id}:${asset.screenPreview?.key ?? ""}:${asset.wallThumbnail?.key ?? ""}`;
	useLayoutEffect(() => {
		if (candidateKey) setCandidateIndex(0);
	}, [candidateKey]);
	const selected = candidates[candidateIndex];

	useLayoutEffect(() => {
		const stage = stageRef.current;
		if (!stage) return;
		const measure = () =>
			setStageSize({ width: stage.clientWidth, height: stage.clientHeight });
		measure();
		if (typeof ResizeObserver !== "undefined") {
			const observer = new ResizeObserver(measure);
			observer.observe(stage);
			return () => observer.disconnect();
		}
		window.addEventListener("resize", measure);
		return () => window.removeEventListener("resize", measure);
	}, []);

	const frame = fitViewerFrame(
		stageSize.width,
		stageSize.height,
		asset.width,
		asset.height,
	);
	const frameStyle =
		frame.width > 0
			? { width: `${frame.width}px`, height: `${frame.height}px` }
			: undefined;

	return (
		<div className={styles.viewerStage} ref={stageRef}>
			<div
				className={styles.viewerFrame}
				data-testid="viewer-frame"
				style={frameStyle}
			>
				{selected ? (
					<img
						alt={asset.displayName}
						className={styles.viewerImage}
						data-viewer-layer={selected.layer}
						decoding="async"
						draggable={false}
						onError={() => setCandidateIndex((index) => index + 1)}
						src={selected.url}
					/>
				) : (
					<div
						aria-label={`${asset.displayName} representative colour`}
						className={styles.viewerColour}
						data-viewer-layer="representativeColour"
						role="img"
						style={{
							backgroundColor:
								asset.representativeRgb === null
									? "var(--canvas-elevated)"
									: `rgb(${(asset.representativeRgb >> 16) & 255}, ${(asset.representativeRgb >> 8) & 255}, ${asset.representativeRgb & 255})`,
						}}
					/>
				)}
			</div>
		</div>
	);
}
