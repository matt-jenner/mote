import {
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
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
	scrollEpoch: number;
	busy: boolean;
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
	scrollEpoch,
	busy,
	regionRef: forwardedRegionRef,
}: JustifiedWallProps) {
	const localRegionRef = useRef<HTMLElement>(null);
	const [root, setRoot] = useState<HTMLElement | null>(null);
	const assignRegion = useCallback(
		(node: HTMLElement | null) => {
			localRegionRef.current = node;
			if (forwardedRegionRef) forwardedRegionRef.current = node;
			setRoot(node);
		},
		[forwardedRegionRef],
	);
	const sentinelRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (!root || rows.length === 0 || scrollEpoch < 0) return;
		const rowNodes = [
			...root.querySelectorAll<HTMLElement>("[data-testid^='photo-row-']"),
		];
		const rootRect = root.getBoundingClientRect();
		const viewportTop = root.scrollTop;
		const viewportBottom = viewportTop + root.clientHeight;
		const visibleRows: number[] = [];
		for (let index = 0; index < rows.length; index += 1) {
			const row = rows[index];
			const node = rowNodes[index];
			if (!row) continue;
			const top = node
				? node.getBoundingClientRect().top - rootRect.top + root.scrollTop
				: rows.slice(0, index).reduce((sum, item) => sum + item.height + 4, 16);
			const bottom = top + (node?.getBoundingClientRect().height || row.height);
			if (top < viewportBottom && bottom > viewportTop) visibleRows.push(index);
		}
		const visible = visibleRows.length > 0 ? visibleRows : [0];
		const lastVisible = visible.at(-1) ?? 0;
		const nearRows = [lastVisible + 1, lastVisible + 2].filter(
			(index) => index < rows.length,
		);
		const visibleIds = visible.flatMap(
			(index) =>
				rows[index]?.items
					.filter((item) => item.asset.wallThumbnail === null)
					.map((item) => item.asset.id) ?? [],
		);
		const nearIds = nearRows.flatMap(
			(index) =>
				rows[index]?.items
					.filter((item) => item.asset.wallThumbnail === null)
					.map((item) => item.asset.id) ?? [],
		);
		const claimed = new Set([...visibleIds, ...nearIds]);
		const remainingIds = rows
			.flatMap((row) =>
				row.items
					.filter(
						(item) =>
							item.asset.wallThumbnail === null && !claimed.has(item.asset.id),
					)
					.map((item) => item.asset.id),
			)
			.filter((id, index, ids) => ids.indexOf(id) === index);
		if (visibleIds.length > 0) requestVisibleDerivatives(visibleIds);
		if (nearIds.length > 0) requestNearViewportDerivatives(nearIds);

		let idleHandle: number | null = null;
		let timerHandle: number | null = null;
		let offset = 0;
		const scheduleRemaining = () => {
			if (offset >= remainingIds.length) return;
			const run = () => {
				idleHandle = null;
				timerHandle = null;
				const batch = remainingIds.slice(offset, offset + 50);
				offset += batch.length;
				if (batch.length > 0) requestNearViewportDerivatives(batch);
				if (offset < remainingIds.length) scheduleRemaining();
			};
			const requestIdle = (
				window as Window & {
					requestIdleCallback?: (callback: () => void) => number;
				}
			).requestIdleCallback;
			if (requestIdle) idleHandle = requestIdle(run);
			else timerHandle = window.setTimeout(run, 0);
		};
		scheduleRemaining();
		return () => {
			if (idleHandle !== null) {
				const cancelIdle = (
					window as Window & {
						cancelIdleCallback?: (handle: number) => void;
					}
				).cancelIdleCallback;
				cancelIdle?.(idleHandle);
			}
			if (timerHandle !== null) window.clearTimeout(timerHandle);
		};
	}, [
		requestNearViewportDerivatives,
		requestVisibleDerivatives,
		root,
		rows,
		scrollEpoch,
	]);

	useEffect(() => {
		if (!root) return;
		const visibleIds = new Set<string>();
		const nearIds = new Set<string>();
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
		const visitTiles = (node: Node, visit: (tile: HTMLElement) => void) => {
			if (!(node instanceof Element)) return;
			if (node.matches("[data-asset-id]")) visit(node as HTMLElement);
			for (const tile of node.querySelectorAll<HTMLElement>("[data-asset-id]"))
				visit(tile);
		};
		const observeTile = (tile: HTMLElement) => {
			visibleObserver.observe(tile);
			nearObserver.observe(tile);
		};
		const unobserveTile = (tile: HTMLElement) => {
			visibleObserver.unobserve(tile);
			nearObserver.unobserve(tile);
		};
		for (const tile of root.querySelectorAll<HTMLElement>("[data-asset-id]"))
			observeTile(tile);
		const mutationObserver =
			typeof MutationObserver === "undefined"
				? null
				: new MutationObserver((records) => {
						for (const record of records) {
							for (const node of record.addedNodes)
								visitTiles(node, observeTile);
							for (const node of record.removedNodes)
								visitTiles(node, unobserveTile);
						}
					});
		mutationObserver?.observe(root, { childList: true, subtree: true });
		return () => {
			if (frame !== null) window.cancelAnimationFrame(frame);
			mutationObserver?.disconnect();
			loadObserver.disconnect();
			visibleObserver.disconnect();
			nearObserver.disconnect();
		};
	}, [
		loadMore,
		requestNearViewportDerivatives,
		requestVisibleDerivatives,
		root,
	]);

	useEffect(() => {
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
	}, [root, setWallInteraction]);

	return (
		<section
			aria-busy={busy ? "true" : "false"}
			aria-label="Photos"
			className={styles.wallRegion}
			ref={assignRegion}
		>
			<div className={styles.wallContent}>
				{rows.map((row, rowIndex) => (
					<div
						className={styles.row}
						data-testid={`photo-row-${rowIndex}`}
						key={row.items[0]?.asset.id ?? rowIndex}
					>
						{row.items.map((item) => (
							<PhotoTile
								key={`${row.items[0]?.asset.id ?? "row"}:${item.asset.id}`}
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
