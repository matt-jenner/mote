import { useCallback, useEffect, useRef, useState } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import type { DerivativeReference, WallAsset } from "../services/photoService";
import { buildViewerPreviewPlan } from "./viewerPreviewPlan";

export interface ViewerPreviewOptions {
	assets: readonly WallAsset[];
	currentIndex: number;
	previewGeneration: number;
}

export interface ViewerPreviewState {
	currentUrl: string | null;
	baseUrl: string | null;
	largePreviewUnavailable: boolean;
	reportInteraction: () => void;
}

interface RequestRecord {
	warningCode: string | null;
	retryable: boolean;
	generation: number;
	priority: "visible" | "nearViewport";
	attempt: number;
	failed: boolean;
}

const MAX_PREVIEW_ATTEMPTS = 3;
const PREVIEW_RETRY_DELAYS_MS = [100, 250] as const;
export const viewerIdleFallbackDelayMs = 250;
const interactionQuietPeriodMs = 200;

interface CancelableSchedule {
	cancel: () => void;
}

function scheduleViewerIdle(
	callback: () => void,
	isInteractionActive: () => boolean,
): CancelableSchedule {
	const browserWindow = window as Window & {
		requestIdleCallback?: (callback: () => void) => number;
		cancelIdleCallback?: (handle: number) => void;
	};
	let cancelled = false;
	let idleHandle: number | null = null;
	let timerHandle: number | null = null;

	if (browserWindow.requestIdleCallback) {
		const schedule = () => {
			if (cancelled) return;
			idleHandle = browserWindow.requestIdleCallback(() => {
				idleHandle = null;
				if (cancelled) return;
				if (isInteractionActive()) {
					schedule();
					return;
				}
				callback();
			});
		};
		schedule();
		return {
			cancel: () => {
				cancelled = true;
				if (idleHandle !== null) browserWindow.cancelIdleCallback?.(idleHandle);
			},
		};
	}

	const schedule = () => {
		if (cancelled) return;
		timerHandle = window.setTimeout(() => {
			timerHandle = null;
			if (cancelled) return;
			if (isInteractionActive()) {
				schedule();
				return;
			}
			callback();
		}, viewerIdleFallbackDelayMs);
	};
	schedule();
	return {
		cancel: () => {
			cancelled = true;
			if (timerHandle !== null) window.clearTimeout(timerHandle);
		},
	};
}

function toUrl(
	service: ReturnType<typeof usePhotoService>,
	reference: DerivativeReference | null,
): string | null {
	if (!reference) return null;
	try {
		return service.derivativeUrl(reference);
	} catch {
		return null;
	}
}

function requestKey(asset: WallAsset, generation: number): string {
	return `${asset.id}:${generation}:${asset.screenPreview?.key ?? "none"}`;
}

export function useViewerPreview({
	assets,
	currentIndex,
	previewGeneration,
}: ViewerPreviewOptions): ViewerPreviewState {
	const service = usePhotoService();
	const [retryTick, setRetryTick] = useState(0);
	const [failedRequestKeys, setFailedRequestKeys] = useState<Set<string>>(
		() => new Set(),
	);
	const requestRecords = useRef(new Map<string, RequestRecord>());
	const attemptCounts = useRef(new Map<string, number>());
	const retryTimers = useRef(new Set<number>());
	const interactionActive = useRef(false);
	const interactionTimer = useRef<number | null>(null);

	const reportInteraction = useCallback(() => {
		interactionActive.current = true;
		if (interactionTimer.current !== null)
			window.clearTimeout(interactionTimer.current);
		interactionTimer.current = window.setTimeout(() => {
			interactionTimer.current = null;
			interactionActive.current = false;
		}, interactionQuietPeriodMs);
	}, []);

	// Retry tick intentionally re-runs the current plan after a rejected request.
	// biome-ignore lint/correctness/useExhaustiveDependencies: retryTick is an explicit effect trigger.
	useEffect(() => {
		let cancelled = false;
		const plans = buildViewerPreviewPlan(assets, currentIndex);

		const requestPlan = (
			assetIds: readonly string[],
			priority: "visible" | "nearViewport",
		) => {
			const ids: string[] = [];
			const attempts = new Map<string, RequestRecord>();
			for (const assetId of new Set(assetIds)) {
				const asset = assets.find((candidate) => candidate.id === assetId);
				if (!asset || asset.screenPreview) continue;
				if (asset.warning && !asset.warning.retryable) continue;
				const key = requestKey(asset, previewGeneration);
				const previous = requestRecords.current.get(key);
				const warningCode = asset.warning?.code ?? null;
				const retryable = asset.warning?.retryable ?? false;
				if (
					previous &&
					previous.warningCode === warningCode &&
					previous.retryable === retryable &&
					previous.generation === previewGeneration &&
					!(priority === "visible" && previous.priority === "nearViewport")
				)
					continue;
				const attempt = (attemptCounts.current.get(key) ?? 0) + 1;
				if (attempt > MAX_PREVIEW_ATTEMPTS) continue;
				const record: RequestRecord = {
					warningCode,
					retryable,
					generation: previewGeneration,
					priority,
					attempt,
					failed: false,
				};
				attemptCounts.current.set(key, attempt);
				requestRecords.current.set(key, record);
				attempts.set(key, record);
				ids.push(asset.id);
			}
			if (ids.length === 0 || cancelled) return;
			void service
				.requestDerivatives({
					assetIds: ids,
					priority,
					kind: "screenPreview",
				})
				.catch(() => {
					if (cancelled) return;
					for (const assetId of ids) {
						const asset = assets.find((candidate) => candidate.id === assetId);
						if (!asset) continue;
						const key = requestKey(asset, previewGeneration);
						const record = attempts.get(key);
						if (!record || requestRecords.current.get(key) !== record) continue;
						record.failed = true;
						setFailedRequestKeys((previous) => {
							if (previous.has(key)) return previous;
							const next = new Set(previous);
							next.add(key);
							return next;
						});
						if (record.attempt >= MAX_PREVIEW_ATTEMPTS) continue;
						const delay =
							PREVIEW_RETRY_DELAYS_MS[record.attempt - 1] ??
							PREVIEW_RETRY_DELAYS_MS.at(-1) ??
							viewerIdleFallbackDelayMs;
						const timer = window.setTimeout(() => {
							retryTimers.current.delete(timer);
							if (cancelled) return;
							if (requestRecords.current.get(key) === record)
								requestRecords.current.delete(key);
							setRetryTick((tick) => tick + 1);
						}, delay);
						retryTimers.current.add(timer);
					}
				});
		};

		for (const plan of plans.filter((candidate) => !candidate.idle))
			requestPlan(plan.assetIds, plan.priority);
		const idlePlan = plans.find((candidate) => candidate.idle);
		const scheduled = idlePlan
			? scheduleViewerIdle(
					() => requestPlan(idlePlan.assetIds, idlePlan.priority),
					() => interactionActive.current,
				)
			: null;

		return () => {
			cancelled = true;
			scheduled?.cancel();
			for (const timer of retryTimers.current) window.clearTimeout(timer);
			retryTimers.current.clear();
		};
	}, [assets, currentIndex, previewGeneration, retryTick, service]);

	useEffect(
		() => () => {
			if (interactionTimer.current !== null)
				window.clearTimeout(interactionTimer.current);
			for (const timer of retryTimers.current) window.clearTimeout(timer);
		},
		[],
	);

	const current = assets[Math.trunc(currentIndex)];
	const currentUrl = toUrl(service, current?.screenPreview ?? null);
	const baseUrl = toUrl(service, current?.wallThumbnail ?? null);
	const currentKey = current ? requestKey(current, previewGeneration) : null;
	return {
		currentUrl,
		baseUrl,
		reportInteraction,
		largePreviewUnavailable: Boolean(
			current &&
				((currentKey !== null && failedRequestKeys.has(currentKey)) ||
					(current.screenPreview !== null && currentUrl === null) ||
					(current.screenPreview === null &&
						current.warning?.retryable === false)),
		),
	};
}
