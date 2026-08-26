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
	settlementGeneration: number | null;
}

export interface PhotoWallController {
	state: typeof initialWallState;
	loading: boolean;
	status: string;
	progress: WallProgress;
	retry: () => void;
	loadMore: () => void;
	setDirection: (direction: SortDirection) => void;
	requestVisibleDerivatives: (assetIds: readonly string[]) => void;
	requestNearViewportDerivatives: (assetIds: readonly string[]) => void;
	setWallInteraction: (active: boolean) => void;
	layoutComplete: boolean;
}

export interface WallProgress {
	status: string;
	value: number | null;
	max: number | null;
	busy: boolean;
}

function wallProgress(state: typeof initialWallState): WallProgress {
	if (state.error)
		return { status: state.error, value: null, max: null, busy: false };
	const known = state.items.length;
	const wallReady = state.items.filter((item) => item.wallThumbnail).length;
	const screenReady = state.items.filter((item) => item.screenPreview).length;
	const missingWall = known - wallReady;
	const missingScreen = known - screenReady;
	const busy = state.activeRequest !== null || missingWall > 0;
	if (
		Object.keys(state.sourceWarnings).length > 0 ||
		Object.keys(state.assetWarnings).length > 0
	)
		return {
			status: "Some previews need attention",
			value: null,
			max: null,
			busy,
		};
	if (
		missingWall > 0 &&
		(state.scanComplete || (state.scanProgress?.shaped ?? 0) > 0)
	)
		return {
			status: `Preparing previews · ${wallReady} of ${known}`,
			value: wallReady,
			max: known > 0 ? known : null,
			busy: true,
		};
	if (state.scanComplete && missingScreen > 0)
		return {
			status: `Photos ready · preparing larger previews · ${screenReady} of ${known}`,
			value: screenReady,
			max: known > 0 ? known : null,
			busy,
		};
	if (!state.scanComplete) {
		const progress = state.scanProgress;
		const suffix =
			progress && progress.total !== null
				? ` · ${progress.shaped} of ${progress.total}`
				: "";
		return {
			status: `${state.items.length === 0 ? "Folder ready · " : ""}Indexing photos${suffix}`,
			value: progress?.total === null ? null : (progress?.shaped ?? null),
			max: progress?.total ?? null,
			busy: state.activeRequest !== null,
		};
	}
	if (known === 0 && state.pagesExhausted && !state.activeRequest)
		return { status: "No photos found", value: 0, max: 0, busy: false };
	return {
		status: `${known} photos ready`,
		value: known,
		max: known,
		busy,
	};
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
	const settlementPending = useRef<number | null>(null);
	const failedCursor = useRef<string | null>(null);
	const derivativeRequests = useRef(
		new Map<
			string,
			{ sourceGeneration: number; epoch: number; priority: DerivativePriority }
		>(),
	);
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
			settlementGeneration?: number,
		) => {
			const expectedSourceId = sourceId;
			if (!expectedSourceId) return;
			const generation = generationOverride ?? sourceGeneration.current;
			if (!isLive(generation, expectedSourceId) || ownerRef.current) return;
			const targetSettlementGeneration = settle
				? (settlementGeneration ?? 1)
				: null;
			const requestId: WallRequestId = ++requestNumber.current;
			const owner: RequestOwner = {
				sourceId: expectedSourceId,
				generation,
				requestId,
				cursor,
				epoch: initialSourceQuery ? 0 : stateRef.current.scrollEpoch,
				settlementGeneration: targetSettlementGeneration,
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
							sourceWarnings: page.sourceWarnings,
							requestEpoch: owner.epoch,
							requestCursor: cursor,
							requestId,
							sourceGeneration: generation,
							generation: targetSettlementGeneration ?? undefined,
						});
					} else {
						dispatch({
							type: "pageLoaded",
							assets: page.items,
							orderState: page.orderState,
							nextCursor: page.nextCursor,
							sourceWarnings: page.sourceWarnings,
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
					if (settlementPending.current !== null) {
						const pendingGeneration = settlementPending.current;
						settlementPending.current = null;
						queueMicrotask(() =>
							loadPage(null, true, generation, false, pendingGeneration),
						);
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
		settlementPending.current = null;
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
		dispatch({
			type: "resetSource",
			sourceGeneration: generation,
			selectionId: expectedSourceId ?? undefined,
		});
		if (!expectedSourceId) return;
		const stop = service.watchWallUpdates((update: WallUpdate) => {
			if (!isLive(generation, expectedSourceId)) return;
			if (
				"selectionId" in update &&
				update.selectionId &&
				update.selectionId !== expectedSourceId
			)
				return;
			const hasSelectionId =
				"selectionId" in update && Boolean(update.selectionId);
			if (
				!hasSelectionId &&
				"sourceId" in update &&
				update.sourceId !== expectedSourceId
			)
				return;
			switch (update.kind) {
				case "catalogBatch":
					dispatch({ type: "catalogBatch", ...update });
					break;
				case "progress":
					dispatch({ type: "progress", ...update });
					break;
				case "derivativesReady":
					for (const derivative of update.derivatives) {
						if (derivative.kind === "wallThumbnail")
							derivativeRequests.current.delete(derivative.assetId);
					}
					dispatch({
						type: "derivativesReady",
						derivatives: update.derivatives,
					});
					break;
				case "metadataSettled":
					{
						const settledGeneration = update.generation ?? 1;
						const knownGeneration = Math.max(
							stateRef.current.settledGeneration ?? 0,
							ownerRef.current?.settlementGeneration ?? 0,
							settlementPending.current ?? 0,
						);
						if (settledGeneration <= knownGeneration) break;
						if (ownerRef.current)
							settlementPending.current = Math.max(
								settlementPending.current ?? 0,
								settledGeneration,
							);
						else loadPage(null, true, generation, false, settledGeneration);
					}
					break;
				case "sourceUnavailable":
					dispatch({
						type: "wallError",
						sourceGeneration: generation,
						error: "Source unavailable. Try again.",
					});
					break;
				case "warning":
					if (update.assetId !== null && update.warning.retryable)
						derivativeRequests.current.delete(update.assetId);
					dispatch({ type: "warning", ...update });
					break;
				case "warningCleared":
					dispatch({ type: "warningCleared", ...update });
					break;
				case "resyncRequired":
					ownerRef.current = null;
					settlementPending.current = null;
					dispatch({ type: "resyncRequired", selectionId: expectedSourceId });
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
				settlementPending.current = null;
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
		settlementPending.current = null;
		for (const asset of stateRef.current.items) {
			if (!asset.wallThumbnail) derivativeRequests.current.delete(asset.id);
		}
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
			const sourceGenerationAtRequest = sourceGeneration.current;
			const epoch = stateRef.current.scrollEpoch;
			for (const assetId of new Set(assetIds)) {
				const previous = derivativeRequests.current.get(assetId);
				if (
					previous &&
					previous.sourceGeneration === sourceGenerationAtRequest &&
					previous.epoch === epoch &&
					(previous.priority === "visible" || previous.priority === priority)
				)
					continue;
				derivativeRequests.current.set(assetId, {
					sourceGeneration: sourceGenerationAtRequest,
					epoch,
					priority,
				});
				next.push(assetId);
			}
			if (next.length > 0) {
				void service
					.requestDerivatives({ assetIds: next, priority })
					.catch(() => {
						for (const assetId of next) {
							const record = derivativeRequests.current.get(assetId);
							if (
								record?.sourceGeneration === sourceGenerationAtRequest &&
								record.epoch === epoch
							)
								derivativeRequests.current.delete(assetId);
						}
					});
			}
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
			status: wallProgress(state).status,
			progress: wallProgress(state),
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
