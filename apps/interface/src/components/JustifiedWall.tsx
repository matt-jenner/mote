import { type RefObject, useEffect, useRef } from "react";
import type { PhotoService, WallAsset } from "../services/photoService";
import styles from "../styles/photoWall.module.css";
import type { JustifiedRow } from "../wall/layoutJustifiedRows";
import { PhotoTile } from "./PhotoTile";

interface JustifiedWallProps {
	rows: readonly JustifiedRow[];
	assets: readonly WallAsset[];
	service: PhotoService;
	loadMore: () => void;
	requestVisibleDerivatives: (assetIds: readonly string[]) => void;
	requestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	setWallInteraction: (active: boolean) => void;
	regionRef?: RefObject<HTMLElement | null>;
}

export function JustifiedWall({
	rows,
	assets,
	service,
	loadMore,
	requestVisibleDerivatives,
	requestNearViewportDerivatives,
	setWallInteraction,
	regionRef: forwardedRegionRef,
}: JustifiedWallProps) {
	const localRegionRef = useRef<HTMLElement>(null);
	const regionRef = forwardedRegionRef ?? localRegionRef;
	const sentinelRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const root = regionRef.current;
		if (!root) return;
		const firstRowIds = rows[0]?.items.map((item) => item.asset.id) ?? [];
		const nearRowIds = rows
			.slice(0, 2)
			.flatMap((row) => row.items.map((item) => item.asset.id));
		if (firstRowIds.length > 0) requestVisibleDerivatives(firstRowIds);
		if (nearRowIds.length > 0) requestNearViewportDerivatives(nearRowIds);
		const sentinel = sentinelRef.current;
		if (!sentinel || typeof IntersectionObserver === "undefined") {
			return;
		}
		const loadObserver = new IntersectionObserver(
			(entries) => {
				if (entries.some((entry) => entry.isIntersecting)) loadMore();
			},
			{ root, rootMargin: "480px 0px" },
		);
		loadObserver.observe(sentinel);

		const visibleObserver = new IntersectionObserver(
			(entries) => {
				const ids = entries
					.filter((entry) => entry.isIntersecting)
					.map((entry) => (entry.target as HTMLElement).dataset.assetId)
					.filter((id): id is string => Boolean(id));
				if (ids.length > 0) requestVisibleDerivatives(ids);
			},
			{ root, rootMargin: "0px" },
		);
		const nearObserver = new IntersectionObserver(
			(entries) => {
				const ids = entries
					.filter((entry) => entry.isIntersecting)
					.map((entry) => (entry.target as HTMLElement).dataset.assetId)
					.filter((id): id is string => Boolean(id));
				if (ids.length > 0) requestNearViewportDerivatives(ids);
			},
			{ root, rootMargin: "720px 0px" },
		);
		for (const tile of root.querySelectorAll<HTMLElement>("[data-asset-id]")) {
			visibleObserver.observe(tile);
			nearObserver.observe(tile);
		}
		return () => {
			loadObserver.disconnect();
			visibleObserver.disconnect();
			nearObserver.disconnect();
		};
	}, [
		loadMore,
		requestNearViewportDerivatives,
		requestVisibleDerivatives,
		rows,
		regionRef.current,
	]);

	useEffect(() => {
		const root = regionRef.current;
		if (!root) return;
		const report = () => setWallInteraction(true);
		const events = [
			"pointerdown",
			"pointermove",
			"keydown",
			"touchstart",
			"wheel",
			"scroll",
		];
		for (const event of events)
			root.addEventListener(event, report, { passive: true });
		return () => {
			for (const event of events) root.removeEventListener(event, report);
		};
	}, [setWallInteraction, regionRef.current]);

	return (
		<section aria-label="Photos" className={styles.wallRegion} ref={regionRef}>
			<div className={styles.wallContent}>
				{rows.map((row, rowIndex) => (
					<div
						className={styles.row}
						data-testid={`photo-row-${rowIndex}`}
						key={row.items[0]?.asset.id ?? rowIndex}
					>
						{row.items.map((item) => (
							<PhotoTile
								key={item.asset.id}
								positioned={item}
								service={service}
							/>
						))}
					</div>
				))}
				<div
					aria-hidden="true"
					className={styles.loadSentinel}
					ref={sentinelRef}
				/>
				{rows.length === 0 && assets.length === 0 ? (
					<div className={styles.emptyWall}>No photos found</div>
				) : null}
			</div>
		</section>
	);
}
