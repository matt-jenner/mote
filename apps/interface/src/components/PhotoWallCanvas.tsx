import { useEffect, useMemo, useRef, useState } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import type { PhotoWallController } from "../app/usePhotoWall";
import type { SourceSummary } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import { layoutJustifiedRows } from "../wall/layoutJustifiedRows";
import { JustifiedWall } from "./JustifiedWall";

interface PhotoWallCanvasProps {
	source: SourceSummary;
	wall: PhotoWallController;
}

export function PhotoWallCanvas({
	source: _source,
	wall,
}: PhotoWallCanvasProps) {
	const service = usePhotoService();
	const regionRef = useRef<HTMLElement>(null);
	const [containerWidth, setContainerWidth] = useState(0);

	useEffect(() => {
		const region = regionRef.current;
		if (!region) return;
		const measure = () => {
			const padding = Number.parseFloat(
				getComputedStyle(region).getPropertyValue("--wall-content-padding"),
			);
			const inset = Number.isFinite(padding) ? padding : 0;
			const width = region.clientWidth || window.innerWidth;
			setContainerWidth(Math.max(0, width - inset * 2));
		};
		measure();
		if (typeof ResizeObserver !== "undefined") {
			const observer = new ResizeObserver(measure);
			observer.observe(region);
			return () => observer.disconnect();
		}
		window.addEventListener("resize", measure);
		return () => window.removeEventListener("resize", measure);
	}, []);

	useEffect(() => {
		const reportResize = () => wall.setWallInteraction(true);
		window.addEventListener("resize", reportResize, { passive: true });
		return () => window.removeEventListener("resize", reportResize);
	}, [wall.setWallInteraction]);

	const scrollEpoch = wall.state.scrollEpoch;
	useEffect(() => {
		if (scrollEpoch >= 0 && regionRef.current) regionRef.current.scrollTop = 0;
	}, [scrollEpoch]);

	const rows = useMemo(() => {
		if (containerWidth <= 0 || wall.state.items.length === 0) return [];
		const layoutAssets = wall.state.items.map((asset) =>
			asset.shapeState === "fallback"
				? { ...asset, width: 4, height: 3 }
				: asset,
		);
		return layoutJustifiedRows(layoutAssets, {
			containerWidth,
			targetRowHeight: containerWidth < 560 ? 150 : 220,
			gap: 4,
			layoutComplete: wall.layoutComplete,
		});
	}, [containerWidth, wall.layoutComplete, wall.state.items]);

	return (
		<main className={styles.wallCanvas}>
			<JustifiedWall
				assets={wall.state.items}
				loadMore={wall.loadMore}
				regionRef={regionRef}
				requestNearViewportDerivatives={wall.requestNearViewportDerivatives}
				requestVisibleDerivatives={wall.requestVisibleDerivatives}
				rows={rows}
				service={service}
				setWallInteraction={wall.setWallInteraction}
				showEmpty={wall.layoutComplete && !wall.loading && !wall.state.error}
			/>
		</main>
	);
}
