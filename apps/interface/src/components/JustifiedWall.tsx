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
	showEmpty: boolean;
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
	showEmpty,
	regionRef: forwardedRegionRef,
}: JustifiedWallProps) {
	const localRegionRef = useRef<HTMLElement>(null);
	const regionRef = forwardedRegionRef ?? localRegionRef;
	const sentinelRef = useRef<HTMLDivElement>(null);
	const renderedAssetKey = assets.map((asset) => asset.id).join("\u0000");

	useEffect(() => {
		const root = regionRef.current;
		if (!root) return;
		const visibleIds = new Set<string>();
		const nearIds = new Set<string>();
		const renderedIds = new Set(
			renderedAssetKey ? renderedAssetKey.split("\u0000") : [],
		);
		let frame: number | null = null;
		const flush = () => {
			frame = null;
			const visible = [...visibleIds];
			const near = [...nearIds].filter((id) => !visibleIds.has(id));
			visibleIds.clear();
			nearIds.clear();
			if (visible.length > 0) requestVisibleDerivatives(visible);
			if (near.length > 0) requestNearViewportDerivatives(near);
		};
		const schedule = () => {
			if (frame === null) frame = window.requestAnimationFrame(flush);
		};
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
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const id = (entry.target as HTMLElement).dataset.assetId;
					if (id) visibleIds.add(id);
				}
				schedule();
			},
			{ root, rootMargin: "0px" },
		);
		const nearObserver = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const id = (entry.target as HTMLElement).dataset.assetId;
					if (id) nearIds.add(id);
				}
				schedule();
			},
			{ root, rootMargin: "720px 0px" },
		);
		for (const tile of root.querySelectorAll<HTMLElement>("[data-asset-id]")) {
			if (!renderedIds.has(tile.dataset.assetId ?? "")) continue;
			visibleObserver.observe(tile);
			nearObserver.observe(tile);
		}
		return () => {
			if (frame !== null) window.cancelAnimationFrame(frame);
			loadObserver.disconnect();
			visibleObserver.disconnect();
			nearObserver.disconnect();
		};
	}, [
		loadMore,
		requestNearViewportDerivatives,
		requestVisibleDerivatives,
		regionRef.current,
		renderedAssetKey,
	]);

	useEffect(() => {
		const root = regionRef.current;
		if (!root) return;
		const report = () => setWallInteraction(true);
		const events = [
			"pointerdown",
			"pointermove",
			"touchstart",
			"wheel",
			"scroll",
		];
		for (const event of events)
			root.addEventListener(event, report, { passive: true });
		window.addEventListener("keydown", report, { passive: true });
		return () => {
			for (const event of events) root.removeEventListener(event, report);
			window.removeEventListener("keydown", report);
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
				{showEmpty && rows.length === 0 && assets.length === 0 ? (
					<div className={styles.emptyWall}>No photos found</div>
				) : null}
			</div>
		</section>
	);
}
