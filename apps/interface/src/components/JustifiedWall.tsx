import {
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type {
	OrderState,
	PhotoService,
	WallAsset,
} from "../services/photoService";
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
	orderState: OrderState;
	setWallInteraction: (active: boolean) => void;
	showEmpty: boolean;
	scrollEpoch: number;
	busy: boolean;
	regionRef?: RefObject<HTMLElement | null>;
	onOpen: (assetId: string) => void;
	isPicked: (assetId: string) => boolean;
	onTogglePick: (asset: WallAsset) => void;
	highlightedAssetId?: string | null;
}

interface ViewportRowPass {
	visibleIds: string[];
	nearIds: string[];
}

const BACKGROUND_REQUEST_BATCH_SIZE = 50;
const BACKGROUND_REQUEST_WINDOW_SIZE = 200;

function getViewportRowPass(
	root: HTMLElement,
	rows: readonly JustifiedRow[],
): ViewportRowPass {
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
	const idsInRows = (indices: readonly number[]) =>
		indices.flatMap(
			(index) => rows[index]?.items.map((item) => item.asset.id) ?? [],
		);
	return { visibleIds: idsInRows(visible), nearIds: idsInRows(nearRows) };
}

export function JustifiedWall({
	rows,
	assets,
	service,
	loadMore,
	requestVisibleDerivatives,
	requestNearViewportDerivatives,
	orderState,
	setWallInteraction,
	showEmpty,
	scrollEpoch,
	busy,
	regionRef: forwardedRegionRef,
	onOpen,
	isPicked,
	onTogglePick,
	highlightedAssetId = null,
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
	const missingWallIdsRef = useRef<Set<string>>(new Set());
	missingWallIdsRef.current = new Set(
		assets
			.filter((asset) => asset.wallThumbnail === null)
			.map((asset) => asset.id),
	);

	useEffect(() => {
		if (!root || rows.length === 0 || scrollEpoch < 0) return;
		const viewportPass = getViewportRowPass(root, rows);
		const visibleIds = viewportPass.visibleIds;
		const nearIds = viewportPass.nearIds;
		const claimed = new Set([...visibleIds, ...nearIds]);
		const backgroundWindowIds = rows
			.flatMap((row) =>
				row.items
					.filter(
						(item) =>
							item.asset.wallThumbnail === null && !claimed.has(item.asset.id),
					)
					.map((item) => item.asset.id),
			)
			.filter((id, index, ids) => ids.indexOf(id) === index)
			.slice(0, BACKGROUND_REQUEST_WINDOW_SIZE);
		const currentMissing = (ids: readonly string[]) =>
			ids.filter((id) => missingWallIdsRef.current.has(id));
		const visibleMissing = currentMissing(visibleIds);
		const nearMissing = currentMissing(nearIds);
		if (visibleMissing.length > 0) requestVisibleDerivatives(visibleMissing);
		if (nearMissing.length > 0) requestNearViewportDerivatives(nearMissing);

		let idleHandle: number | null = null;
		let timerHandle: number | null = null;
		let offset = 0;
		const scheduleRemaining = () => {
			if (offset >= backgroundWindowIds.length) return;
			const run = () => {
				idleHandle = null;
				timerHandle = null;
				const batch = backgroundWindowIds.slice(
					offset,
					offset + BACKGROUND_REQUEST_BATCH_SIZE,
				);
				offset += batch.length;
				const missingBatch = currentMissing(batch);
				if (missingBatch.length > 0)
					requestNearViewportDerivatives(missingBatch);
				if (offset < backgroundWindowIds.length) scheduleRemaining();
			};
			const requestIdle = (
				window as Window & {
					requestIdleCallback?: (callback: () => void) => number;
				}
			).requestIdleCallback;
			if (requestIdle) idleHandle = requestIdle(run);
			else timerHandle = window.setTimeout(run, 0);
		};
		if (orderState === "settled") scheduleRemaining();
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
		orderState,
		root,
		rows,
		scrollEpoch,
	]);

	useEffect(() => {
		if (!root) return;
		const visibleIds = new Set<string>();
		const nearIds = new Set<string>();
		let frame: number | null = null;
		let scrollPassFrame: number | null = null;
		const scheduleScrollPass = () => {
			if (orderState !== "provisional" || scrollPassFrame !== null) return;
			scrollPassFrame = window.requestAnimationFrame(() => {
				scrollPassFrame = null;
				const viewportPass = getViewportRowPass(root, rows);
				const visible = viewportPass.visibleIds.filter((id) =>
					missingWallIdsRef.current.has(id),
				);
				const near = viewportPass.nearIds.filter((id) =>
					missingWallIdsRef.current.has(id),
				);
				if (visible.length > 0) requestVisibleDerivatives(visible);
				if (near.length > 0) requestNearViewportDerivatives(near);
			});
		};
		const flush = () => {
			frame = null;
			const visible = [...visibleIds].filter((id) =>
				missingWallIdsRef.current.has(id),
			);
			const near = [...nearIds].filter(
				(id) => !visibleIds.has(id) && missingWallIdsRef.current.has(id),
			);
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
					if (id && missingWallIdsRef.current.has(id)) visibleIds.add(id);
				}
				schedule();
			},
			{ root, rootMargin: "0px" },
		);
		const nearObserver = new IntersectionObserver(
			(entries) => {
				const provisionalNearIds =
					orderState === "provisional"
						? new Set(getViewportRowPass(root, rows).nearIds)
						: null;
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const id = (entry.target as HTMLElement).dataset.assetId;
					if (
						id &&
						missingWallIdsRef.current.has(id) &&
						(orderState === "settled" || provisionalNearIds?.has(id))
					)
						nearIds.add(id);
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
		root.addEventListener("scroll", scheduleScrollPass, { passive: true });
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
			if (scrollPassFrame !== null)
				window.cancelAnimationFrame(scrollPassFrame);
			root.removeEventListener("scroll", scheduleScrollPass);
			mutationObserver?.disconnect();
			loadObserver.disconnect();
			visibleObserver.disconnect();
			nearObserver.disconnect();
		};
	}, [
		loadMore,
		orderState,
		requestNearViewportDerivatives,
		requestVisibleDerivatives,
		root,
		rows,
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
			tabIndex={-1}
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
								onOpen={onOpen}
								onTogglePick={onTogglePick}
								picked={isPicked(item.asset.id)}
								positioned={item}
								service={service}
								highlighted={highlightedAssetId === item.asset.id}
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
