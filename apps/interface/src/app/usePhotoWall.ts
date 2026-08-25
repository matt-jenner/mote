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

export interface PhotoWallController {
	state: typeof initialWallState;
	loading: boolean;
	status: string;
	loadMore: () => void;
	setDirection: (direction: SortDirection) => void;
	requestVisibleDerivatives: (assetIds: readonly string[]) => void;
	requestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	setWallInteraction: (active: boolean) => void;
	layoutComplete: boolean;
}

function wallStatus(state: typeof initialWallState, loading: boolean): string {
	if (state.items.length === 0 && !loading && state.pagesExhausted)
		return "No photos found · Folder ready";
	if (!state.scanComplete) return "Indexing photos";
	if (loading) return `${state.items.length} photos loading`;
	return `${state.items.length} photos ready`;
}

export function usePhotoWall(sourceId: string | null): PhotoWallController {
	const service = usePhotoService();
	const [state, dispatch] = useReducer(wallReducer, initialWallState);
	const requestNumber = useRef(0);
	const requestInFlight = useRef(false);
	const settlementPending = useRef(false);
	const derivativeRequests = useRef(new Map<string, DerivativePriority>());
	const wallInteractionTimer = useRef<number | null>(null);
	const wallInteractionActive = useRef(false);
	const stateRef = useRef(state);
	stateRef.current = state;

	const loadPage = useCallback(
		(cursor: string | null, settle = false) => {
			if (!sourceId || requestInFlight.current) return;
			requestInFlight.current = true;
			const requestId: WallRequestId = ++requestNumber.current;
			const requestEpoch = stateRef.current.scrollEpoch;
			const requestCursor = cursor;
			// This action must precede the service call so every response is fenced.
			dispatch({
				type: "pageRequestStarted",
				requestId,
				requestCursor,
				requestEpoch,
			});
			void service
				.queryWall({
					cursor: requestCursor,
					limit: 100,
					direction: stateRef.current.direction,
				})
				.then((page) => {
					if (settle) {
						dispatch({
							type: "metadataSettled",
							assets: page.items,
							nextCursor: page.nextCursor,
							requestEpoch,
							requestCursor,
							requestId,
						});
					} else {
						dispatch({
							type: "pageLoaded",
							assets: page.items,
							orderState: page.orderState,
							nextCursor: page.nextCursor,
							requestEpoch,
							requestCursor,
							requestId,
						});
					}
				})
				.catch(() => {
					// The service reports recoverable warnings through its update stream.
				})
				.finally(() => {
					requestInFlight.current = false;
					if (settlementPending.current) {
						settlementPending.current = false;
						queueMicrotask(() => loadPage(null, true));
					}
				});
		},
		[sourceId, service],
	);

	useEffect(() => {
		if (!sourceId) return;
		const stop = service.watchWallUpdates((update: WallUpdate) => {
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
					if (requestInFlight.current) settlementPending.current = true;
					else loadPage(null, true);
					break;
				default:
					break;
			}
		});
		void (service as ScanCapableService).startFixtureScan?.();
		loadPage(null);
		return stop;
	}, [loadPage, service, sourceId]);

	const setDirection = useCallback((direction: SortDirection) => {
		dispatch({ type: "setDirection", direction });
		requestInFlight.current = false;
	}, []);

	useEffect(() => {
		if (state.scrollEpoch === 0 || !sourceId) return;
		loadPage(null);
	}, [loadPage, sourceId, state.scrollEpoch]);

	const loadMore = useCallback(() => {
		if (state.pagesExhausted || requestInFlight.current) return;
		loadPage(state.cursor);
	}, [loadPage, state.cursor, state.pagesExhausted]);

	const requestDerivatives = useCallback(
		(assetIds: readonly string[], priority: DerivativePriority) => {
			const unique = [...new Set(assetIds)].filter(Boolean);
			const next: string[] = [];
			for (const assetId of unique) {
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
			status: wallStatus(state, state.activeRequest !== null),
			loadMore,
			setDirection,
			requestVisibleDerivatives: (ids: readonly string[]) =>
				requestDerivatives(ids, "visible"),
			requestNearViewportDerivatives: (ids: readonly string[]) =>
				requestDerivatives(ids, "nearViewport"),
			setWallInteraction,
			layoutComplete: isWallLayoutComplete(state),
		}),
		[loadMore, requestDerivatives, setDirection, setWallInteraction, state],
	);
}
