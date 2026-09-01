import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
	DerivativeClass,
	DerivativePriority,
	GalleryScope,
	ScanProgressDto,
	SortDirection,
	WallPage,
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

interface DerivativeRequestRecord {
	sourceGeneration: number;
	epoch: number;
	priority: DerivativePriority;
	attempt: number;
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

function isWallThumbnailWarning(code: string): boolean {
	return (
		code === "wallThumbnailUnavailable" || code === "derivativeUnavailable"
	);
}

function isScreenPreviewWarning(code: string): boolean {
	return (
		code === "screenPreviewUnavailable" ||
		code === "screenPreviewCacheUnavailable" ||
		code === "derivativeUnavailable"
	);
}

function isSweepWarning(code: string, kind: DerivativeClass): boolean {
	return kind === "wallThumbnail"
		? isWallThumbnailWarning(code)
		: isScreenPreviewWarning(code);
}

const countFormatter = new Intl.NumberFormat();
const HOSTED_BACKGROUND_PAGE_SIZE = 250;
const HOSTED_BACKGROUND_PENDING_LIMIT = 200;
const HOSTED_BACKGROUND_REFILL_SIZE = 50;

interface RetainedScanProgress {
	sourceId: string;
	generation: number;
	progress: ScanProgressDto;
}

function rememberScanProgress(
	current: RetainedScanProgress | null,
	sourceId: string,
	generation: number,
	incoming: ScanProgressDto,
): RetainedScanProgress {
	if (
		current === null ||
		current.sourceId !== sourceId ||
		generation > current.generation
	) {
		return { sourceId, generation, progress: incoming };
	}
	if (generation < current.generation) return current;
	return {
		sourceId,
		generation,
		progress: {
			discovered: Math.max(current.progress.discovered, incoming.discovered),
			shaped: Math.max(current.progress.shaped, incoming.shaped),
			enriched: Math.max(current.progress.enriched, incoming.enriched),
			directTotal: incoming.directTotal ?? current.progress.directTotal,
			total: incoming.total ?? current.progress.total,
		},
	};
}

function progressForScope(
	progress: ScanProgressDto,
	scope: GalleryScope,
): ScanProgressDto {
	return {
		...progress,
		total:
			scope === "currentFolder"
				? (progress.directTotal ?? progress.total)
				: progress.total,
	};
}

function wallProgress(state: typeof initialWallState): WallProgress {
	if (state.error)
		return {
			status: state.error,
			value: null,
			max: null,
			busy: state.activeRequest !== null || state.sortPending,
		};
	const known = state.items.length;
	const total = Math.max(known, state.totalCount ?? 0);
	const wallReady = Math.min(
		total,
		state.previewCounts?.wallReady ??
			state.items.filter((item) => item.wallThumbnail).length,
	);
	const screenReady = Math.min(
		total,
		state.previewCounts?.screenReady ??
			state.items.filter((item) => item.screenPreview).length,
	);
	const missingWall = total - wallReady;
	const missingScreen = total - screenReady;
	const busy =
		state.activeRequest !== null || missingWall > 0 || state.sortPending;
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
	if (state.derivativeRetrying)
		return {
			status: `Retrying previews · ${countFormatter.format(wallReady)} of ${countFormatter.format(total)}`,
			value: wallReady,
			max: total > 0 ? total : null,
			busy: true,
		};
	if (
		!state.scanComplete &&
		state.scanProgress &&
		state.scanProgress.total !== null
	) {
		const progress = state.scanProgress;
		const suffix =
			progress && progress.total !== null
				? ` · ${countFormatter.format(progress.shaped)} indexed of ${countFormatter.format(progress.total)}`
				: "";
		return {
			status: `${state.items.length === 0 ? "Folder ready · " : ""}Indexing photos${suffix}`,
			value: progress?.total === null ? null : (progress?.shaped ?? null),
			max: progress?.total ?? null,
			busy,
		};
	}
	if (missingWall > 0)
		return {
			status: `Preparing previews · ${countFormatter.format(wallReady)} of ${countFormatter.format(total)}`,
			value: wallReady,
			max: total > 0 ? total : null,
			busy: true,
		};
	if (missingScreen > 0)
		return {
			status: `Photos ready · preparing larger previews · ${countFormatter.format(screenReady)} of ${countFormatter.format(total)}`,
			value: screenReady,
			max: total > 0 ? total : null,
			busy,
		};
	if (!state.scanComplete)
		return {
			status: `${state.items.length === 0 ? "Folder ready · " : ""}Indexing photos`,
			value: null,
			max: null,
			busy,
		};
	if (known === 0 && state.pagesExhausted && !state.activeRequest)
		return { status: "No photos found", value: null, max: null, busy: false };
	return {
		status: `${countFormatter.format(total)} photos ready`,
		value: total,
		max: total,
		busy,
	};
}

export function usePhotoWall(
	sourceId: string | null,
	galleryScope: GalleryScope = "includeSubfolders",
): PhotoWallController {
	const service = usePhotoService();
	const [state, dispatch] = useReducer(wallReducer, {
		...initialWallState,
		direction: service.initialSortDirection(),
	});
	const stateRef = useRef(state);
	stateRef.current = state;
	const requestNumber = useRef(0);
	const sourceIdRef = useRef(sourceId);
	const galleryScopeRef = useRef(galleryScope);
	galleryScopeRef.current = galleryScope;
	const sourceGeneration = useRef(0);
	const ownerRef = useRef<RequestOwner | null>(null);
	const settlementPending = useRef<number | null>(null);
	const latestScanProgress = useRef<RetainedScanProgress | null>(null);
	const failedCursor = useRef<string | null>(null);
	const readyWallIds = useRef(new Set<string>());
	const derivativeRequests = useRef(new Map<string, DerivativeRequestRecord>());
	const derivativeAttempt = useRef(0);
	const derivativeRetryTimer = useRef<number | null>(null);
	const derivativeRetryQueue = useRef(
		new Map<
			string,
			{ priority: DerivativePriority; sourceGeneration: number; epoch: number }
		>(),
	);
	const derivativeRetryAttempts = useRef(new Map<string, number>());
	const hostedSweepPending = useRef(new Set<string>());
	const hostedSweepFailed = useRef(new Set<string>());
	const hostedSweepKind = useRef<DerivativeClass | null>(null);
	const hostedSweepWake = useRef<(() => void) | null>(null);
	const requestDerivativesRef = useRef<
		(assetIds: readonly string[], priority: DerivativePriority) => void
	>(() => undefined);
	const wallInteractionTimer = useRef<number | null>(null);
	const wallInteractionActive = useRef(false);

	const isLive = useCallback(
		(generation: number, expectedSourceId: string) =>
			sourceGeneration.current === generation &&
			sourceIdRef.current === expectedSourceId,
		[],
	);
	const cancelDerivativeRetry = useCallback(() => {
		if (derivativeRetryTimer.current !== null) {
			window.clearTimeout(derivativeRetryTimer.current);
			derivativeRetryTimer.current = null;
		}
		derivativeRetryQueue.current.clear();
		derivativeRetryAttempts.current.clear();
		dispatch({
			type: "derivativeRetrying",
			sourceGeneration: sourceGeneration.current,
			retrying: false,
		});
	}, []);

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
					direction: stateRef.current.direction,
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
							totalCount: page.totalCount,
							previewCounts: page.previewCounts,
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
							totalCount: page.totalCount,
							previewCounts: page.previewCounts,
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
		const expectedGalleryScope = galleryScope;
		const generation = sourceGeneration.current + 1;
		sourceGeneration.current = generation;
		sourceIdRef.current = expectedSourceId;
		ownerRef.current = null;
		settlementPending.current = null;
		failedCursor.current = null;
		readyWallIds.current.clear();
		derivativeRequests.current.clear();
		hostedSweepPending.current.clear();
		hostedSweepFailed.current.clear();
		hostedSweepKind.current = null;
		hostedSweepWake.current?.();
		cancelDerivativeRetry();
		if (wallInteractionTimer.current !== null) {
			window.clearTimeout(wallInteractionTimer.current);
			wallInteractionTimer.current = null;
		}
		if (wallInteractionActive.current) {
			wallInteractionActive.current = false;
			void service.setWallInteraction(false);
		}
		const retainedProgress =
			latestScanProgress.current?.sourceId === expectedSourceId
				? latestScanProgress.current
				: null;
		if (retainedProgress === null) latestScanProgress.current = null;
		dispatch({
			type: "resetSource",
			sourceGeneration: generation,
			selectionId: expectedSourceId ?? undefined,
		});
		if (!expectedSourceId) return;
		if (retainedProgress) {
			dispatch({
				type: "progress",
				selectionId: expectedSourceId,
				generation: retainedProgress.generation,
				progress: progressForScope(
					retainedProgress.progress,
					expectedGalleryScope,
				),
			});
		}
		const stop = service.watchWallUpdates((update: WallUpdate) => {
			if (galleryScopeRef.current !== expectedGalleryScope) return;
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
				case "catalogBatch": {
					const retained = rememberScanProgress(
						latestScanProgress.current,
						expectedSourceId,
						update.generation,
						update.progress,
					);
					latestScanProgress.current = retained;
					dispatch({
						type: "catalogBatch",
						...update,
						progress: progressForScope(retained.progress, expectedGalleryScope),
					});
					break;
				}
				case "progress": {
					const retained = rememberScanProgress(
						latestScanProgress.current,
						expectedSourceId,
						update.generation,
						update.progress,
					);
					latestScanProgress.current = retained;
					dispatch({
						type: "progress",
						...update,
						progress: progressForScope(retained.progress, expectedGalleryScope),
					});
					break;
				}
				case "derivativesReady":
					{
						let backgroundWallReady = 0;
						let backgroundScreenReady = 0;
						for (const derivative of update.derivatives) {
							if (derivative.kind === "wallThumbnail") {
								if (
									stateRef.current.items.some(
										(item) => item.id === derivative.assetId,
									)
								)
									readyWallIds.current.add(derivative.assetId);
								derivativeRequests.current.delete(derivative.assetId);
								derivativeRetryQueue.current.delete(derivative.assetId);
								derivativeRetryAttempts.current.delete(derivative.assetId);
							}
							if (
								hostedSweepKind.current === derivative.kind &&
								hostedSweepPending.current.delete(derivative.assetId)
							) {
								hostedSweepFailed.current.delete(derivative.assetId);
								if (derivative.kind === "wallThumbnail")
									backgroundWallReady += 1;
								else backgroundScreenReady += 1;
								hostedSweepWake.current?.();
							}
						}
						const currentCounts = stateRef.current.previewCounts;
						const optimisticCounts =
							currentCounts &&
							(backgroundWallReady > 0 || backgroundScreenReady > 0)
								? {
										wallReady: Math.min(
											stateRef.current.totalCount ?? Number.MAX_SAFE_INTEGER,
											currentCounts.wallReady + backgroundWallReady,
										),
										screenReady: Math.min(
											stateRef.current.totalCount ?? Number.MAX_SAFE_INTEGER,
											currentCounts.screenReady + backgroundScreenReady,
										),
									}
								: null;
						dispatch({
							type: "derivativesReady",
							derivatives: update.derivatives,
							previewCounts: update.previewCounts ?? optimisticCounts,
						});
					}
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
					if (
						update.assetId !== null &&
						update.warning.retryable &&
						isWallThumbnailWarning(update.warning.code)
					)
						readyWallIds.current.delete(update.assetId);
					if (
						update.assetId !== null &&
						update.warning.retryable &&
						isWallThumbnailWarning(update.warning.code)
					)
						derivativeRequests.current.delete(update.assetId);
					if (
						update.assetId !== null &&
						hostedSweepKind.current !== null &&
						isSweepWarning(update.warning.code, hostedSweepKind.current)
					) {
						hostedSweepFailed.current.add(update.assetId);
						if (hostedSweepPending.current.delete(update.assetId))
							hostedSweepWake.current?.();
					}
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
			if (
				galleryScopeRef.current === expectedGalleryScope &&
				isLive(generation, expectedSourceId)
			)
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
	}, [
		cancelDerivativeRetry,
		galleryScope,
		isLive,
		loadPage,
		service,
		sourceId,
	]);

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

	const setDirection = useCallback(
		(direction: SortDirection) => {
			if (stateRef.current.direction === direction) return;
			ownerRef.current = null;
			settlementPending.current = null;
			cancelDerivativeRetry();
			for (const asset of stateRef.current.items) {
				if (!asset.wallThumbnail) derivativeRequests.current.delete(asset.id);
			}
			dispatch({ type: "setDirection", direction });
			service.rememberSortDirection(direction);
		},
		[cancelDerivativeRetry, service],
	);

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
			const attempts: Array<{
				assetId: string;
				record: DerivativeRequestRecord;
			}> = [];
			const sourceGenerationAtRequest = sourceGeneration.current;
			const epoch = stateRef.current.scrollEpoch;
			for (const assetId of new Set(assetIds)) {
				if (
					readyWallIds.current.has(assetId) ||
					stateRef.current.items.find((item) => item.id === assetId)
						?.wallThumbnail
				) {
					derivativeRequests.current.delete(assetId);
					derivativeRetryQueue.current.delete(assetId);
					derivativeRetryAttempts.current.delete(assetId);
					continue;
				}
				const previous = derivativeRequests.current.get(assetId);
				if (
					previous &&
					previous.sourceGeneration === sourceGenerationAtRequest &&
					previous.epoch === epoch &&
					(previous.priority === "visible" || previous.priority === priority)
				)
					continue;
				const record: DerivativeRequestRecord = {
					sourceGeneration: sourceGenerationAtRequest,
					epoch,
					priority,
					attempt: ++derivativeAttempt.current,
				};
				derivativeRequests.current.set(assetId, record);
				next.push(assetId);
				attempts.push({ assetId, record });
			}
			if (next.length > 0) {
				void Promise.resolve()
					.then(() =>
						service.requestDerivatives({
							assetIds: next,
							priority,
							kind: "wallThumbnail",
						}),
					)
					.catch(() => {
						let retryDelay = 50;
						for (const { assetId, record } of attempts) {
							if (derivativeRequests.current.get(assetId) !== record) continue;
							derivativeRequests.current.delete(assetId);
							const attempt =
								(derivativeRetryAttempts.current.get(assetId) ?? 0) + 1;
							derivativeRetryAttempts.current.set(assetId, attempt);
							if (attempt > 3) continue;
							derivativeRetryQueue.current.set(assetId, {
								priority: record.priority,
								sourceGeneration: record.sourceGeneration,
								epoch: record.epoch,
							});
							retryDelay = Math.max(retryDelay, 50 * 2 ** (attempt - 1));
						}
						if (derivativeRetryQueue.current.size === 0) {
							dispatch({
								type: "derivativeRetrying",
								sourceGeneration: sourceGenerationAtRequest,
								retrying: false,
							});
							return;
						}
						dispatch({
							type: "derivativeRetrying",
							sourceGeneration: sourceGenerationAtRequest,
							retrying: true,
						});
						if (derivativeRetryTimer.current !== null) return;
						derivativeRetryTimer.current = window.setTimeout(() => {
							derivativeRetryTimer.current = null;
							const retryGroups = new Map<DerivativePriority, string[]>();
							for (const [assetId, queued] of derivativeRetryQueue.current) {
								if (
									queued.sourceGeneration !== sourceGeneration.current ||
									queued.epoch !== stateRef.current.scrollEpoch ||
									stateRef.current.items.find((item) => item.id === assetId)
										?.wallThumbnail
								)
									continue;
								const group = retryGroups.get(queued.priority) ?? [];
								group.push(assetId);
								retryGroups.set(queued.priority, group);
							}
							derivativeRetryQueue.current.clear();
							dispatch({
								type: "derivativeRetrying",
								sourceGeneration: sourceGeneration.current,
								retrying: false,
							});
							for (const [retryPriority, retryIds] of retryGroups)
								requestDerivativesRef.current(retryIds, retryPriority);
						}, retryDelay);
					});
			}
		},
		[service],
	);
	requestDerivativesRef.current = requestDerivatives;
	const requestVisibleDerivatives = useCallback(
		(ids: readonly string[]) => requestDerivatives(ids, "visible"),
		[requestDerivatives],
	);
	const requestNearViewportDerivatives = useCallback(
		(ids: readonly string[]) => requestDerivatives(ids, "nearViewport"),
		[requestDerivatives],
	);

	useEffect(() => {
		if (
			service.capabilities.folderSelection !== "hosted" ||
			!sourceId ||
			!state.scanComplete ||
			state.orderState !== "settled"
		)
			return;
		const expectedSourceId = sourceId;
		const expectedGeneration = sourceGeneration.current;
		const expectedDirection = state.direction;
		let cancelled = false;
		let wakeTimer: number | null = null;
		let wakeResolve: (() => void) | null = null;
		const isCurrent = () =>
			!cancelled &&
			isLive(expectedGeneration, expectedSourceId) &&
			galleryScopeRef.current === galleryScope &&
			stateRef.current.direction === expectedDirection;
		const wake = () => {
			if (!wakeResolve) return;
			const resolve = wakeResolve;
			wakeResolve = null;
			if (wakeTimer !== null) window.clearTimeout(wakeTimer);
			wakeTimer = null;
			resolve();
		};
		const waitForProgress = () =>
			new Promise<void>((resolve) => {
				wakeResolve = resolve;
				wakeTimer = window.setTimeout(wake, 1_000);
			});
		const pruneFinishedRequests = (kind: DerivativeClass) => {
			if (kind !== "wallThumbnail") return;
			for (const assetId of hostedSweepPending.current) {
				if (
					!derivativeRequests.current.has(assetId) &&
					!derivativeRetryQueue.current.has(assetId)
				)
					hostedSweepPending.current.delete(assetId);
			}
		};
		hostedSweepWake.current = wake;

		const runPass = async (kind: DerivativeClass) => {
			hostedSweepKind.current = kind;
			hostedSweepPending.current.clear();
			hostedSweepFailed.current.clear();
			let cursor: string | null = null;
			let traversalComplete = false;
			let requestedDuringTraversal = false;
			const deferredIds: string[] = [];
			while (isCurrent()) {
				pruneFinishedRequests(kind);
				if (
					hostedSweepPending.current.size >
					HOSTED_BACKGROUND_PENDING_LIMIT - HOSTED_BACKGROUND_REFILL_SIZE
				) {
					await waitForProgress();
					continue;
				}

				if (deferredIds.length === 0 && !traversalComplete) {
					let page: WallPage;
					try {
						page = await service.queryWall({
							cursor,
							limit: HOSTED_BACKGROUND_PAGE_SIZE,
							direction: expectedDirection,
						});
					} catch {
						await waitForProgress();
						continue;
					}
					if (!isCurrent()) return;
					dispatch({
						type: "derivativesReady",
						derivatives: [],
						previewCounts: page.previewCounts,
					});
					const readyCount =
						kind === "wallThumbnail"
							? page.previewCounts.wallReady
							: page.previewCounts.screenReady;
					if (
						readyCount >= page.totalCount &&
						hostedSweepPending.current.size === 0
					)
						return;
					deferredIds.push(
						...page.items
							.filter(
								(asset) =>
									(kind === "wallThumbnail"
										? asset.wallThumbnail === null
										: asset.screenPreview === null) &&
									!hostedSweepPending.current.has(asset.id) &&
									!hostedSweepFailed.current.has(asset.id) &&
									(kind !== "wallThumbnail" ||
										(derivativeRetryAttempts.current.get(asset.id) ?? 0) <= 3),
							)
							.map((asset) => asset.id),
					);
					cursor = page.nextCursor;
					traversalComplete = cursor === null;
				}

				const capacity =
					HOSTED_BACKGROUND_PENDING_LIMIT - hostedSweepPending.current.size;
				const batch = deferredIds.splice(
					0,
					Math.min(HOSTED_BACKGROUND_REFILL_SIZE, capacity),
				);
				if (batch.length > 0) {
					requestedDuringTraversal = true;
					for (const assetId of batch) hostedSweepPending.current.add(assetId);
					if (kind === "wallThumbnail")
						requestDerivativesRef.current(batch, "nearViewport");
					else
						void service
							.requestDerivatives({
								assetIds: batch,
								priority: "nearViewport",
								kind,
							})
							.catch(() => {
								for (const assetId of batch) {
									hostedSweepPending.current.delete(assetId);
									hostedSweepFailed.current.add(assetId);
								}
								hostedSweepWake.current?.();
							});
					continue;
				}

				if (!traversalComplete) continue;
				if (hostedSweepPending.current.size > 0) {
					await waitForProgress();
					continue;
				}
				if (!requestedDuringTraversal) return;
				requestedDuringTraversal = false;
				traversalComplete = false;
				cursor = null;
				await waitForProgress();
			}
		};
		void (async () => {
			await runPass("wallThumbnail");
			if (isCurrent()) await runPass("screenPreview");
			if (isCurrent()) hostedSweepKind.current = null;
		})();

		return () => {
			cancelled = true;
			wake();
			if (hostedSweepWake.current === wake) hostedSweepWake.current = null;
			hostedSweepPending.current.clear();
			hostedSweepKind.current = null;
		};
	}, [
		galleryScope,
		isLive,
		service,
		sourceId,
		state.direction,
		state.orderState,
		state.scanComplete,
	]);

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
			if (derivativeRetryTimer.current !== null)
				window.clearTimeout(derivativeRetryTimer.current);
			derivativeRetryTimer.current = null;
			derivativeRetryQueue.current.clear();
			derivativeRetryAttempts.current.clear();
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
