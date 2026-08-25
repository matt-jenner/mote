import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
	DerivativePriority,
	SortDirection,
	WallUpdate,
} from "../services/photoService";
import {
	initialWallState,
	isWallLayoutComplete,
	type WallRequestId,
	wallReducer,
} from "../wall/wallReducer";
import { usePhotoService } from "./PhotoServiceContext";

type ScanCapableService = ReturnType<typeof usePhotoService> & {
	startFixtureScan?: () => Promise<void>;
};

interface RequestOwner {
	sourceId: string;
	generation: number;
	requestId: WallRequestId;
	cursor: string | null;
	epoch: number;
}

export interface PhotoWallController {
	state: typeof initialWallState;
	loading: boolean;
	status: string;
	retry: () => void;
	loadMore: () => void;
	setDirection: (direction: SortDirection) => void;
	requestVisibleDerivatives: (assetIds: readonly string[]) => void;
	requestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	setWallInteraction: (active: boolean) => void;
	layoutComplete: boolean;
}

function wallStatus(state: typeof initialWallState): string {
	if (state.error) return state.error;
	if (!state.scanComplete)
		return state.items.length === 0
			? "Folder ready · Indexing photos"
			: "Indexing photos";
	if (state.items.length === 0 && state.pagesExhausted && !state.activeRequest)
		return "No photos found";
	if (state.activeRequest) return `${state.items.length} photos loading`;
	return `${state.items.length} photos ready`;
}

export function usePhotoWall(sourceId: string | null): PhotoWallController {
	const service = usePhotoService();
	const [state, dispatch] = useReducer(wallReducer, initialWallState);
	const stateRef = useRef(state);
	stateRef.current = state;
	const requestNumber = useRef(0);
	const sourceIdRef = useRef(sourceId);
	const sourceGeneration = useRef(0);
	const ownerRef = useRef<RequestOwner | null>(null);
	const settlementPending = useRef(false);
	const failedCursor = useRef<string | null>(null);
	const derivativeRequests = useRef(new Map<string, DerivativePriority>());
	const wallInteractionTimer = useRef<number | null>(null);
	const wallInteractionActive = useRef(false);

	const isLive = useCallback(
		(generation: number, expectedSourceId: string) =>
			sourceGeneration.current === generation &&
			sourceIdRef.current === expectedSourceId,
		[],
	);

	const loadPage = useCallback(
		(
			cursor: string | null,
			settle = false,
			generationOverride?: number,
			initialSourceQuery = false,
		) => {
			const expectedSourceId = sourceId;
			if (!expectedSourceId) return;
			const generation = generationOverride ?? sourceGeneration.current;
			if (!isLive(generation, expectedSourceId) || ownerRef.current) return;
			const requestId: WallRequestId = ++requestNumber.current;
			const owner: RequestOwner = {
				sourceId: expectedSourceId,
				generation,
				requestId,
				cursor,
				epoch: initialSourceQuery ? 0 : stateRef.current.scrollEpoch,
			};
			ownerRef.current = owner;
			dispatch({
				type: "pageRequestStarted",
				requestId,
				requestCursor: cursor,
				requestEpoch: owner.epoch,
				sourceGeneration: generation,
			});
			void service
				.queryWall({
					cursor,
					limit: 100,
					direction: initialSourceQuery
						? "oldestFirst"
						: stateRef.current.direction,
				})
				.then((page) => {
					if (
						ownerRef.current !== owner ||
						!isLive(generation, expectedSourceId)
					)
						return;
					if (settle) {
						dispatch({
							type: "metadataSettled",
							assets: page.items,
							nextCursor: page.nextCursor,
							requestEpoch: owner.epoch,
							requestCursor: cursor,
							requestId,
							sourceGeneration: generation,
						});
					} else {
						dispatch({
							type: "pageLoaded",
							assets: page.items,
							orderState: page.orderState,
							nextCursor: page.nextCursor,
							requestEpoch: owner.epoch,
							requestCursor: cursor,
							requestId,
							sourceGeneration: generation,
						});
					}
				})
				.catch((_error: unknown) => {
					if (
						ownerRef.current !== owner ||
						!isLive(generation, expectedSourceId)
					)
						return;
					failedCursor.current = cursor;
					dispatch({
						type: "pageRequestFailed",
						requestId,
						requestCursor: cursor,
						requestEpoch: owner.epoch,
						sourceGeneration: generation,
						error: "Unable to load photos. Try again.",
					});
				})
				.finally(() => {
					if (
						ownerRef.current !== owner ||
						!isLive(generation, expectedSourceId)
					)
						return;
					ownerRef.current = null;
					if (settlementPending.current) {
						settlementPending.current = false;
						queueMicrotask(() => loadPage(null, true, generation));
					}
				});
		},
		[isLive, service, sourceId],
	);

	useEffect(() => {
		const expectedSourceId = sourceId;
		const generation = sourceGeneration.current + 1;
		sourceGeneration.current = generation;
		sourceIdRef.current = expectedSourceId;
		ownerRef.current = null;
		settlementPending.current = false;
		failedCursor.current = null;
		derivativeRequests.current.clear();
		if (wallInteractionTimer.current !== null) {
			window.clearTimeout(wallInteractionTimer.current);
			wallInteractionTimer.current = null;
		}
		if (wallInteractionActive.current) {
			wallInteractionActive.current = false;
			void service.setWallInteraction(false);
		}
		dispatch({ type: "resetSource", sourceGeneration: generation });
		if (!expectedSourceId) return;
		const stop = service.watchWallUpdates((update: WallUpdate) => {
			if (!isLive(generation, expectedSourceId)) return;
			if ("sourceId" in update && update.sourceId !== expectedSourceId) return;
			switch (update.kind) {
				case "catalogBatch":
					dispatch({ type: "catalogBatch", ...update });
					break;
				case "derivativesReady":
					dispatch({
						type: "derivativesReady",
						derivatives: update.derivatives,
					});
					break;
				case "metadataSettled":
					if (ownerRef.current) settlementPending.current = true;
					else loadPage(null, true, generation);
					break;
				case "sourceUnavailable":
					dispatch({
						type: "wallError",
						sourceGeneration: generation,
						error: "Source unavailable. Try again.",
					});
					break;
				default:
					break;
			}
		});
		void (service as ScanCapableService).startFixtureScan?.();
		queueMicrotask(() => {
			if (isLive(generation, expectedSourceId))
				loadPage(null, false, generation, true);
		});
		return () => {
			stop();
			if (sourceGeneration.current === generation) {
				sourceGeneration.current += 1;
				sourceIdRef.current = null;
				ownerRef.current = null;
				settlementPending.current = false;
			}
		};
	}, [isLive, loadPage, service, sourceId]);

	useEffect(() => {
		if (
			state.scrollEpoch === 0 ||
			!sourceId ||
			state.sourceGeneration !== sourceGeneration.current
		)
			return;
		ownerRef.current = null;
		loadPage(null);
	}, [loadPage, sourceId, state.scrollEpoch, state.sourceGeneration]);

	const setDirection = useCallback((direction: SortDirection) => {
		ownerRef.current = null;
		settlementPending.current = false;
		dispatch({ type: "setDirection", direction });
	}, []);

	const loadMore = useCallback(() => {
		if (state.pagesExhausted || state.error || ownerRef.current) return;
		loadPage(state.cursor);
	}, [loadPage, state.cursor, state.error, state.pagesExhausted]);

	const retry = useCallback(() => {
		if (!state.error) return;
		ownerRef.current = null;
		dispatch({ type: "retryStarted" });
		loadPage(failedCursor.current);
	}, [loadPage, state.error]);

	const requestDerivatives = useCallback(
		(assetIds: readonly string[], priority: DerivativePriority) => {
			const next: string[] = [];
			for (const assetId of new Set(assetIds)) {
				const previous = derivativeRequests.current.get(assetId);
				if (previous === "visible" || previous === priority) continue;
				derivativeRequests.current.set(assetId, priority);
				next.push(assetId);
			}
			if (next.length > 0)
				void service.requestDerivatives({ assetIds: next, priority });
		},
		[service],
	);
	const requestVisibleDerivatives = useCallback(
		(ids: readonly string[]) => requestDerivatives(ids, "visible"),
		[requestDerivatives],
	);
	const requestNearViewportDerivatives = useCallback(
		(ids: readonly string[]) => requestDerivatives(ids, "nearViewport"),
		[requestDerivatives],
	);

	const setWallInteraction = useCallback(
		(active: boolean) => {
			if (wallInteractionTimer.current !== null) {
				window.clearTimeout(wallInteractionTimer.current);
				wallInteractionTimer.current = null;
			}
			if (active) {
				if (!wallInteractionActive.current) {
					wallInteractionActive.current = true;
					void service.setWallInteraction(true);
				}
				wallInteractionTimer.current = window.setTimeout(() => {
					wallInteractionActive.current = false;
					void service.setWallInteraction(false);
					wallInteractionTimer.current = null;
				}, 200);
			} else if (wallInteractionActive.current) {
				wallInteractionActive.current = false;
				void service.setWallInteraction(false);
			}
		},
		[service],
	);

	useEffect(
		() => () => {
			if (wallInteractionTimer.current !== null)
				window.clearTimeout(wallInteractionTimer.current);
			if (wallInteractionActive.current) void service.setWallInteraction(false);
		},
		[service],
	);

	return useMemo(
		() => ({
			state,
			loading: state.activeRequest !== null,
			status: wallStatus(state),
			retry,
			loadMore,
			setDirection,
			requestVisibleDerivatives,
			requestNearViewportDerivatives,
			setWallInteraction,
			layoutComplete: isWallLayoutComplete(state),
		}),
		[
			loadMore,
			requestNearViewportDerivatives,
			requestVisibleDerivatives,
			retry,
			setDirection,
			setWallInteraction,
			state,
		],
	);
}
