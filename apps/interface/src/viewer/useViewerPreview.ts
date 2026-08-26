import { useEffect, useRef, useState } from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import type { WallAsset } from "../services/photoService";
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
}

interface RequestRecord {
	warningCode: string | null;
	retryable: boolean;
	generation: number;
	priority: "visible" | "nearViewport";
}

const idle = (callback: () => void): { cancel: () => void } => {
	const browserWindow = window as Window & {
		requestIdleCallback?: (callback: () => void) => number;
		cancelIdleCallback?: (handle: number) => void;
	};
	if (browserWindow.requestIdleCallback) {
		const handle = browserWindow.requestIdleCallback(callback);
		return {
			cancel: () => browserWindow.cancelIdleCallback?.(handle),
		};
	}
	const handle = window.setTimeout(callback, 1);
	return { cancel: () => window.clearTimeout(handle) };
};

function toUrl(
	service: ReturnType<typeof usePhotoService>,
	reference: WallAsset["screenPreview"],
): string | null {
	if (!reference) return null;
	try {
		return service.derivativeUrl(reference);
	} catch {
		return null;
	}
}

export function useViewerPreview({
	assets,
	currentIndex,
	previewGeneration,
}: ViewerPreviewOptions): ViewerPreviewState {
	const service = usePhotoService();
	const [retryTick, setRetryTick] = useState(0);
	const requestRecords = useRef(new Map<string, RequestRecord>());
	const retryTimers = useRef(new Set<number>());

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
				const key = `${asset.id}:screenPreview`;
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
				const record = {
					warningCode,
					retryable,
					generation: previewGeneration,
					priority,
				};
				requestRecords.current.set(key, record);
				attempts.set(asset.id, record);
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
					for (const assetId of ids) {
						const key = `${assetId}:screenPreview`;
						if (requestRecords.current.get(key) === attempts.get(assetId))
							requestRecords.current.delete(key);
					}
					if (cancelled) return;
					const timer = window.setTimeout(() => {
						retryTimers.current.delete(timer);
						if (!cancelled) setRetryTick((tick) => tick + 1);
					}, 250);
					retryTimers.current.add(timer);
				});
		};

		for (const plan of plans.filter((candidate) => !candidate.idle))
			requestPlan(plan.assetIds, plan.priority);
		const idlePlan = plans.find((candidate) => candidate.idle);
		const scheduled = idlePlan
			? idle(() => requestPlan(idlePlan.assetIds, idlePlan.priority))
			: null;

		return () => {
			cancelled = true;
			scheduled?.cancel();
			for (const timer of retryTimers.current) window.clearTimeout(timer);
			retryTimers.current.clear();
		};
	}, [assets, currentIndex, previewGeneration, retryTick, service]);

	const current = assets[Math.trunc(currentIndex)];
	const currentUrl = toUrl(service, current?.screenPreview ?? null);
	const baseUrl = toUrl(service, current?.wallThumbnail ?? null);
	return {
		currentUrl,
		baseUrl,
		largePreviewUnavailable: Boolean(
			current &&
				((current.screenPreview !== null && currentUrl === null) ||
					(current.screenPreview === null &&
						current.warning?.retryable === false)),
		),
	};
}
