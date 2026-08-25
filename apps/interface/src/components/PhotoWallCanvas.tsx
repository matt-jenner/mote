import { useEffect, useMemo, useRef, useState } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import { usePhotoWall } from "../app/usePhotoWall";
import type { SourceSummary } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import { layoutJustifiedRows } from "../wall/layoutJustifiedRows";
import { JustifiedWall } from "./JustifiedWall";
import { WallToolbar } from "./WallToolbar";

interface PhotoWallCanvasProps {
	source: SourceSummary;
}

export function PhotoWallCanvas({ source }: PhotoWallCanvasProps) {
	const service = usePhotoService();
	const wall = usePhotoWall(source.id);
	const regionRef = useRef<HTMLElement>(null);
	const [containerWidth, setContainerWidth] = useState(0);

	useEffect(() => {
		const region = regionRef.current;
		if (!region) return;
		const measure = () =>
			setContainerWidth(Math.max(0, region.clientWidth - 32));
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

	const changeDirection = (direction: "oldestFirst" | "newestFirst") => {
		if (direction === wall.state.direction) return;
		if (regionRef.current) regionRef.current.scrollTop = 0;
		wall.setDirection(direction);
	};

	return (
		<main className={styles.wallCanvas}>
			<WallToolbar
				direction={wall.state.direction}
				onDirectionChange={changeDirection}
				status={wall.status}
			/>
			<JustifiedWall
				assets={wall.state.items}
				loadMore={wall.loadMore}
				regionRef={regionRef}
				requestNearViewportDerivatives={wall.requestNearViewportDerivatives}
				requestVisibleDerivatives={wall.requestVisibleDerivatives}
				rows={rows}
				service={service}
				setWallInteraction={wall.setWallInteraction}
			/>
		</main>
	);
}
