import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { consumePickToastAction, PickToast } from "../components/PickToast";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";
import type {
	CopyResult,
	DerivativeRequest,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import { PhotoServiceError } from "../services/photoService";
import {
	PickListProvider,
	pickOriginFromSavedFolders,
	usePickListOrigin,
} from "./PickListContext";
import type { PickListSnapshot } from "./pickList";
import { createPickListStore } from "./usePickList";

const asset = (id: string): WallAsset => ({
	id,
	displayName: `${id}.jpg`,
	mediaKind: "jpeg",
	provisionalOrder: 0,
	capturedAtUtc: null,
	dateState: "settled",
	width: 1600,
	height: 1200,
	representativeRgb: null,
	shapeState: "ready",
	availability: "available",
	warning: null,
	wallThumbnail: null,
	screenPreview: null,
	rating: null,
});

const origin = { sourceFolderId: "folder-a", sourceLabel: "Holiday" };
const reference = (item: WallAsset) => ({
	assetId: item.id,
	sourceFolderId: origin.sourceFolderId,
	sourceLabel: origin.sourceLabel,
});
const snapshot = (
	items: WallAsset[],
	revision = 0,
	persistenceError: string | null = null,
): PickListSnapshot => ({
	revision,
	items: items.map((item) => ({ ...reference(item), asset: item })),
	persistenceError,
});

const partialCopy: CopyResult = {
	kind: "complete",
	copiedCount: 1,
	failedCount: 1,
	warningCode: null,
	items: [
		{
			assetId: "first",
			status: "copied",
			destinationName: "first.jpg",
			errorCode: null,
		},
		{
			assetId: "second",
			status: "failed",
			destinationName: null,
			errorCode: "source_unavailable",
		},
	],
};

async function copyFixture(
	copyPickedOriginals: PhotoService["copyPickedOriginals"],
	cancelOriginalCopy?: PhotoService["cancelOriginalCopy"],
) {
	const memory = createInMemoryPhotoService({
		wallAssets: [asset("first"), asset("second"), asset("later")],
	});
	await memory.addPick(reference(asset("first")));
	await memory.addPick(reference(asset("second")));
	let shown = 0;
	const store = createPickListStore({
		...memory,
		copyPickedOriginals,
		cancelOriginalCopy: cancelOriginalCopy ?? memory.cancelOriginalCopy,
		showLastCopyDestination: async () => {
			shown += 1;
		},
	});
	const stop = store.start();
	await Promise.resolve();
	return { store, memory, stop, shown: () => shown };
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, reject, resolve };
}

afterEach(() => {
	vi.useRealTimers();
});

describe("pick list controller", () => {
	it("uses the native post-picker pick snapshot for full copies and explicit IDs for retry", async () => {
		const batches: Array<readonly string[] | null> = [];
		const { store, stop } = await copyFixture(async (ids) => {
			batches.push(ids);
			return batches.length === 1
				? partialCopy
				: { kind: "selectionCancelled" };
		});

		await store.getState().copyOriginals();
		await store.getState().copyOriginals();

		expect(batches).toEqual([null, ["second"]]);
		stop();
	});

	it.each([
		["partial result", async () => partialCopy],
		[
			"native error",
			async () => {
				throw new PhotoServiceError(
					"copyDestinationMissing",
					"The destination folder no longer exists.",
				);
			},
		],
	] as const)(
		"keeps an actionable copy %s visible for five seconds",
		async (_case, copy) => {
			vi.useFakeTimers();
			const { store, stop } = await copyFixture(copy);

			await store.getState().copyOriginals();
			expect(store.getState().toast?.phase).toBe("visible");
			vi.advanceTimersByTime(1_000);
			expect(store.getState().toast).not.toBeNull();
			vi.advanceTimersByTime(3_800);
			expect(store.getState().toast?.phase).toBe("exiting");
			vi.advanceTimersByTime(200);
			expect(store.getState().toast).toBeNull();
			stop();
		},
	);

	it("retries Cancel after its first native cancellation command rejects", async () => {
		const worker = deferred<CopyResult>();
		const firstCommand = deferred<void>();
		const retryCommand = deferred<void>();
		let calls = 0;
		const { store, stop } = await copyFixture(
			(_ids, progress) => {
				progress({ completed: 0, total: 2, item: null });
				return worker.promise;
			},
			() => (++calls === 1 ? firstCommand.promise : retryCommand.promise),
		);
		const operation = store.getState().copyOriginals();
		const firstCancel = store.getState().cancelCopy();
		firstCommand.reject(new Error("native cancellation unavailable"));
		await Promise.resolve();
		expect(store.getState().copy.phase).toBe("copying");
		expect(store.getState().toast?.message).toBe("Couldn't cancel the copy");
		const retryCancel = store.getState().cancelCopy();
		expect(calls).toBe(2);
		expect(store.getState().copy.phase).toBe("cancelling");
		retryCommand.resolve();
		worker.resolve({ kind: "copyCancelled" });
		await Promise.all([operation, firstCancel, retryCancel]);
		expect(store.getState().copy.phase).toBe("idle");
		stop();
	});

	it.each(["complete", "error"] as const)(
		"keeps cancellation ownership when worker %s arrives before the cancel command settles",
		async (terminal) => {
			const worker = deferred<CopyResult>();
			const cancellation = deferred<void>();
			let attempts = 0;
			const { store, stop } = await copyFixture(
				(_ids, progress) => {
					attempts++;
					progress({ completed: 0, total: 2, item: null });
					return worker.promise;
				},
				() => cancellation.promise,
			);
			const initial = store.getState().copy;
			const operation = store.getState().copyOriginals();
			const cancel = store.getState().cancelCopy();
			let settled = false;
			void operation.then(() => {
				settled = true;
			});
			if (terminal === "error") worker.reject(new Error("worker failed"));
			else
				worker.resolve({
					kind: "complete",
					copiedCount: 2,
					failedCount: 0,
					warningCode: null,
					items: ["first", "second"].map((assetId) => ({
						assetId,
						status: "copied",
						destinationName: `${assetId}.jpg`,
						errorCode: null,
					})),
				});
			await new Promise<void>((resolve) => setTimeout(resolve, 0));
			expect(store.getState().copy.phase).toBe("cancelling");
			expect(settled).toBe(false);
			expect(store.getState().toast).toBeNull();
			const duplicate = store.getState().copyOriginals();
			expect(duplicate).toBe(operation);
			expect(attempts).toBe(1);
			await store.getState().clear();
			expect(store.getState().count).toBe(2);
			cancellation.resolve();
			await Promise.all([operation, cancel, duplicate]);
			expect(settled).toBe(true);
			if (terminal === "error") {
				expect(store.getState().copy).toBe(initial);
				expect(store.getState().toast?.message).toBe("Couldn't copy originals");
			} else expect(store.getState().copy.phase).toBe("complete");
			stop();
		},
	);

	it("retains ownership when a failed cancellation is retried as the worker settles", async () => {
		const worker = deferred<CopyResult>();
		const firstCommand = deferred<void>();
		const retryCommand = deferred<void>();
		let calls = 0;
		const { store, stop } = await copyFixture(
			(_ids, progress) => {
				progress({ completed: 0, total: 2, item: null });
				return worker.promise;
			},
			() => (++calls === 1 ? firstCommand.promise : retryCommand.promise),
		);
		let retryScheduled = false;
		const unsubscribe = store.subscribe(() => {
			if (
				calls === 1 &&
				store.getState().copy.phase === "copying" &&
				!retryScheduled
			) {
				retryScheduled = true;
				queueMicrotask(() =>
					queueMicrotask(() => {
						void store.getState().cancelCopy();
					}),
				);
			}
		});
		const operation = store.getState().copyOriginals();
		const cancelling = store.getState().cancelCopy();
		worker.resolve(partialCopy);
		await Promise.resolve();
		firstCommand.reject(new Error("first cancel failed"));
		await new Promise<void>((resolve) => setTimeout(resolve, 0));
		// A retry arriving after terminal publication can be ignored. If it was
		// accepted, the operation must still own its pending cancellation command.
		expect(store.getState().copy.phase).toBe(
			calls === 2 ? "cancelling" : "partial",
		);
		retryCommand.resolve();
		await Promise.all([operation, cancelling]);
		expect(store.getState().copy.phase).toBe("partial");
		unsubscribe();
		stop();
	});
	it("publishes optimistic add immediately and resets its final fade within one second", async () => {
		vi.useFakeTimers();
		const memory = createInMemoryPhotoService({
			wallAssets: [asset("first"), asset("second")],
		});
		const saved = deferred<void>();
		const store = createPickListStore({
			...memory,
			addPick: async (pick) => {
				await saved.promise;
				return memory.addPick(pick);
			},
		});
		const stop = store.start();
		const first = store.getState().toggle(asset("first"), origin);
		expect(store.getState().toast).toMatchObject({
			message: "Added to picks",
			phase: "visible",
		});
		const firstId = store.getState().toast?.id;
		vi.advanceTimersByTime(850);
		expect(store.getState().toast?.phase).toBe("exiting");
		const second = store.getState().toggle(asset("second"), origin);
		expect(store.getState().toast?.id).not.toBe(firstId);
		expect(store.getState().toast?.phase).toBe("visible");
		vi.advanceTimersByTime(150);
		expect(store.getState().toast?.phase).toBe("visible");
		vi.advanceTimersByTime(850);
		expect(store.getState().toast).toBeNull();
		saved.resolve();
		await Promise.all([first, second]);
		expect(store.getState().toast).toBeNull();
		stop();
	});

	it.each([false, true])(
		"cancel waits for command and worker before restoring the exact prior state, partial=%s",
		async (partial) => {
			vi.useFakeTimers();
			const worker = deferred<CopyResult>();
			const cancelled = deferred<void>();
			let attempts = 0;
			let cancelCalls = 0;
			let progress!: Parameters<PhotoService["copyPickedOriginals"]>[1];
			const memory = createInMemoryPhotoService({
				wallAssets: [asset("first"), asset("second")],
			});
			await memory.addPick(reference(asset("first")));
			await memory.addPick(reference(asset("second")));
			const store = createPickListStore({
				...memory,
				copyPickedOriginals: async (_ids, listener) => {
					if (partial && attempts++ === 0) return partialCopy;
					progress = listener;
					listener({ completed: 0, total: 2, item: null });
					return worker.promise;
				},
				cancelOriginalCopy: () => {
					cancelCalls++;
					return cancelled.promise;
				},
			});
			const stop = store.start();
			if (partial) await store.getState().copyOriginals();
			const previous = store.getState().copy;
			const operation = store.getState().copyOriginals();
			const cancellation = store.getState().cancelCopy();
			void store.getState().cancelCopy();
			expect(cancelCalls).toBe(1);
			expect(store.getState().copy.phase).toBe("cancelling");
			progress({ completed: 1, total: 2, item: null });
			expect(store.getState().copy.phase).toBe("cancelling");
			await store.getState().clear();
			expect(store.getState().count).toBe(2);
			worker.resolve({ kind: "copyCancelled" });
			await Promise.resolve();
			await Promise.resolve();
			expect(store.getState().copy.phase).toBe("cancelling");
			cancelled.resolve();
			await Promise.all([operation, cancellation]);
			expect(store.getState().copy).toBe(previous);
			expect(store.getState().toast).toMatchObject({
				message: "Copy cancelled",
				phase: "visible",
			});
			expect(store.getState().copy.message).toBe(previous.message);
			vi.advanceTimersByTime(1_000);
			expect(store.getState().toast).toBeNull();
			stop();
		},
	);

	it("picker dismissal restores silently and Clear resets completion messages and failures", async () => {
		let attempts = 0;
		const { store, stop } = await copyFixture(async () =>
			++attempts === 1 ? { kind: "selectionCancelled" } : partialCopy,
		);
		const initial = store.getState().copy;
		await store.getState().copyOriginals();
		expect(store.getState().copy).toBe(initial);
		expect(store.getState().toast).toBeNull();
		await store.getState().copyOriginals();
		await store.getState().clear();
		expect(store.getState().copy).toEqual(initial);
		stop();
	});
	it("retires removed failures so a later pick joins a fresh copy", async () => {
		const batches: Array<readonly string[] | null> = [];
		const { store, stop } = await copyFixture(async (ids) => {
			batches.push(ids);
			return batches.length === 1
				? partialCopy
				: { kind: "selectionCancelled" };
		});
		await store.getState().copyOriginals();
		await store.getState().remove("second");
		expect(store.getState().copy.failedAssetIds).toEqual([]);
		expect(store.getState().copy.failures).toEqual([]);
		expect(store.getState().copy.phase).toBe("complete");
		await store.getState().toggle(asset("later"), origin);
		await store.getState().copyOriginals();
		expect(batches).toEqual([null, null]);
		stop();
	});

	it("retries only retained failures even when new picks arrive, then retires Clear", async () => {
		const batches: Array<readonly string[] | null> = [];
		const { store, stop } = await copyFixture(async (ids) => {
			batches.push(ids);
			return batches.length === 1
				? partialCopy
				: { kind: "selectionCancelled" };
		});
		await store.getState().copyOriginals();
		await store.getState().toggle(asset("later"), origin);
		await store.getState().copyOriginals();
		expect(batches).toEqual([null, ["second"]]);
		await store.getState().clear();
		expect(store.getState().copy).toMatchObject({
			phase: "idle",
			failedAssetIds: [],
			failures: [],
		});
		stop();
	});

	it("leaves an active retry immutable and reconciles removal when its picker cancels", async () => {
		const retry = deferred<CopyResult>();
		let attempts = 0;
		const { store, stop } = await copyFixture(async () =>
			++attempts === 1 ? partialCopy : retry.promise,
		);
		await store.getState().copyOriginals();
		const copying = store.getState().copyOriginals();
		const active = store.getState().copy;
		await store.getState().remove("second");
		expect(store.getState().copy).toBe(active);
		retry.resolve({ kind: "selectionCancelled" });
		await copying;
		expect(store.getState().copy).toMatchObject({
			phase: "complete",
			failedAssetIds: [],
			failures: [],
		});
		stop();
	});

	it("keeps the original Undo deadline and exposes queued copy feedback after it expires", async () => {
		vi.useFakeTimers();
		const result = deferred<CopyResult>();
		const { store, stop, shown, memory } = await copyFixture(
			() => result.promise,
		);
		await store.getState().clear();
		await memory.addPick(reference(asset("later")));
		const copying = store.getState().copyOriginals();
		const undoToast = store.getState().toast;
		vi.advanceTimersByTime(1_000);
		result.resolve(partialCopy);
		await copying;
		expect(store.getState().toast).toBe(undoToast);
		expect(store.getState().announcement).toBe("Copied 1 of 2");
		expect(store.getState().copy).toMatchObject({
			phase: "complete",
			failedAssetIds: [],
		});
		vi.advanceTimersByTime(3_999);
		expect(store.getState().toast).toBe(undoToast);
		vi.advanceTimersByTime(1);
		expect(store.getState().toast).toMatchObject({
			message: "Copied 1 of 2",
			action: { label: "Show folder" },
		});
		await store.getState().toast?.action?.run();
		expect(shown()).toBe(1);
		vi.advanceTimersByTime(4_800);
		expect(store.getState().toast?.phase).toBe("exiting");
		vi.advanceTimersByTime(200);
		expect(store.getState().toast).toBeNull();
		stop();
	});

	it("can still restore Clear at the end of its Undo window after copy completion", async () => {
		vi.useFakeTimers();
		const result = deferred<CopyResult>();
		const { store, stop, memory } = await copyFixture(() => result.promise);
		await store.getState().clear();
		await memory.addPick(reference(asset("later")));
		const copying = store.getState().copyOriginals();
		result.resolve(partialCopy);
		await copying;
		vi.advanceTimersByTime(4_999);
		expect(store.getState().toast?.action?.label).toBe("Undo");
		await store.getState().toast?.action?.run();
		expect(memory.getPicks().items.map((pick) => pick.assetId)).toEqual([
			"first",
			"second",
			"later",
		]);
		stop();
	});

	it("retains only bounded per-item failure reasons", async () => {
		const { store, stop } = await copyFixture(async () => ({
			kind: "complete",
			copiedCount: 0,
			failedCount: 3,
			warningCode: null,
			items: [
				{
					assetId: "first",
					status: "failed",
					destinationName: null,
					errorCode: "destination_unavailable",
				},
				{
					assetId: "second",
					status: "failed",
					destinationName: null,
					errorCode: "source_unavailable",
				},
				{
					assetId: "later",
					status: "failed",
					destinationName: null,
					errorCode: "/private/secret/debug-detail",
				},
			],
		}));
		await store.getState().toggle(asset("later"), origin);
		await store.getState().copyOriginals();
		expect(store.getState().copy.failures).toEqual([
			{ assetId: "first", code: "destination_unavailable" },
			{ assetId: "second", code: "source_unavailable" },
			{ assetId: "later", code: "copy_failed" },
		]);
		expect(JSON.stringify(store.getState().copy)).not.toContain("/private/");
		stop();
	});
	it("keeps the viewer planner priority when requesting pick derivatives", async () => {
		const requests: DerivativeRequest[] = [];
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			requestPickDerivatives: async (request) => {
				requests.push({ ...request, assetIds: [...request.assetIds] });
			},
		};
		const store = createPickListStore(service);

		(
			store.getState().requestDerivatives as (
				assetIds: readonly string[],
				kind: "screenPreview",
				priority: "nearViewport",
			) => void
		)(["forest"], "screenPreview", "nearViewport");
		await expect
			.poll(() => requests)
			.toEqual([
				{
					assetIds: ["forest"],
					kind: "screenPreview",
					priority: "nearViewport",
				},
			]);
	});

	it("starts with the synchronous snapshot and accepts the loaded snapshot", async () => {
		const first = asset("first");
		const loaded = asset("loaded");
		const load = deferred<PickListSnapshot>();
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([first], 1),
			loadPicks: () => load.promise,
		};
		const store = createPickListStore(service);

		expect(store.getState().count).toBe(1);
		expect(store.getState().isPicked(first.id)).toBe(true);
		store.start();
		load.resolve(snapshot([loaded], 2));
		await load.promise;
		await Promise.resolve();

		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["loaded"],
		);
		expect(store.getState().isPicked(first.id)).toBe(false);
	});

	it("suppresses a duplicate toggle while showing the optimistic count", async () => {
		const added = asset("added");
		const gate = deferred<PickListSnapshot>();
		const memory = createInMemoryPhotoService();
		const addPick = vi.fn(() => gate.promise);
		const service: PhotoService = { ...memory, addPick };
		const store = createPickListStore(service);
		store.start();

		const first = store.getState().toggle(added, origin);
		const duplicate = store.getState().toggle(added, origin);

		expect(store.getState().count).toBe(1);
		expect(store.getState().isPicked(added.id)).toBe(true);
		expect(addPick).toHaveBeenCalledOnce();
		gate.resolve(snapshot([added], 1));
		await Promise.all([first, duplicate]);
		expect(store.getState().count).toBe(1);
	});

	it("rolls back an optimistic toggle when persistence fails", async () => {
		const rejected = asset("rejected");
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			addPick: async () => {
				throw new Error("Disk is read-only");
			},
		};
		const store = createPickListStore(service);
		store.start();

		const mutation = store.getState().toggle(rejected, origin);
		expect(store.getState().count).toBe(1);
		await expect(mutation).rejects.toThrow("Disk is read-only");

		expect(store.getState().count).toBe(0);
		expect(store.getState().isPicked(rejected.id)).toBe(false);
		expect(store.getState().announcement).toBe("Couldn't update picks");
	});

	it("persists Clear before a later Add without hiding the optimistic Add", async () => {
		const first = asset("first");
		const later = asset("later");
		const clearGate = deferred<void>();
		const addGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [first, later] });
		await memory.addPick(reference(first));
		const clearPicks = vi.fn(async () => {
			await clearGate.promise;
			return memory.clearPicks();
		});
		const addPick = vi.fn(async (pick: ReturnType<typeof reference>) => {
			await addGate.promise;
			return memory.addPick(pick);
		});
		const service: PhotoService = { ...memory, addPick, clearPicks };
		const store = createPickListStore(service);
		const stop = store.start();

		const clearing = store.getState().clear();
		const adding = store.getState().toggle(later, origin);

		expect(clearPicks).toHaveBeenCalledOnce();
		expect(addPick).not.toHaveBeenCalled();
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["later"],
		);

		clearGate.resolve(undefined);
		await clearing;
		await Promise.resolve();
		expect(addPick).toHaveBeenCalledOnce();
		expect(memory.getPicks().items).toHaveLength(0);
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["later"],
		);

		addGate.resolve(undefined);
		await adding;
		expect(memory.getPicks().items.map((item) => item.assetId)).toEqual([
			"later",
		]);
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["later"],
		);
		stop();
	});

	it("persists Add before a later Clear", async () => {
		const added = asset("added");
		const addGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [added] });
		const addPick = vi.fn(async (pick: ReturnType<typeof reference>) => {
			await addGate.promise;
			return memory.addPick(pick);
		});
		const clearPicks = vi.fn(() => memory.clearPicks());
		const service: PhotoService = { ...memory, addPick, clearPicks };
		const store = createPickListStore(service);
		const stop = store.start();

		const adding = store.getState().toggle(added, origin);
		const clearing = store.getState().clear();

		expect(store.getState().count).toBe(0);
		expect(clearPicks).not.toHaveBeenCalled();
		addGate.resolve(undefined);
		await adding;
		await clearing;

		expect(clearPicks).toHaveBeenCalledOnce();
		expect(memory.getPicks().items).toHaveLength(0);
		expect(store.getState().count).toBe(0);
		stop();
	});

	it("persists a pending Add before overlapping Undo", async () => {
		const first = asset("first");
		const later = asset("later");
		const addGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [first, later] });
		await memory.addPick(reference(first));
		const addPick = vi.fn(async (pick: ReturnType<typeof reference>) => {
			await addGate.promise;
			return memory.addPick(pick);
		});
		const restorePicks = vi.fn(memory.restorePicks);
		const service: PhotoService = { ...memory, addPick, restorePicks };
		const store = createPickListStore(service);
		const stop = store.start();
		await store.getState().clear();

		const adding = store.getState().toggle(later, origin);
		const restoring = store.getState().undoClear();

		expect(restorePicks).not.toHaveBeenCalled();
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["first", "later"],
		);
		addGate.resolve(undefined);
		await Promise.all([adding, restoring]);

		expect(restorePicks).toHaveBeenCalledOnce();
		expect(memory.getPicks().items.map((item) => item.assetId)).toEqual([
			"first",
			"later",
		]);
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["first", "later"],
		);
		stop();
	});

	it("does no completion work when stopped before Clear resolves", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const clearGate = deferred<PickListSnapshot>();
		const memory = createInMemoryPhotoService({ wallAssets: [first] });
		await memory.addPick(reference(first));
		const service: PhotoService = {
			...memory,
			clearPicks: () => clearGate.promise,
		};
		const store = createPickListStore(service);
		const stop = store.start();
		const listener = vi.fn();
		store.subscribe(listener);
		const clearing = store.getState().clear();
		listener.mockClear();

		stop();
		clearGate.resolve(snapshot([], 2));
		await clearing;

		expect(listener).not.toHaveBeenCalled();
		expect(store.getState().toast).toBeNull();
		expect(store.getState().announcement).toBe("");
		expect(vi.getTimerCount()).toBe(0);
	});

	it("retires a Clear completed while stopped before restart and Add", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const later = asset("later");
		const clearGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [first, later] });
		await memory.addPick(reference(first));
		const service: PhotoService = {
			...memory,
			clearPicks: async () => {
				await clearGate.promise;
				return memory.clearPicks();
			},
		};
		const store = createPickListStore(service);
		const stop = store.start();
		const clearing = store.getState().clear();

		stop();
		clearGate.resolve(undefined);
		await clearing;
		const stopRestart = store.start();
		await store.getState().toggle(later, origin);

		expect(memory.getPicks().items.map((item) => item.assetId)).toEqual([
			"later",
		]);
		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["later"],
		);
		vi.advanceTimersByTime(1_000);
		expect(store.getState().toast).toBeNull();
		expect(vi.getTimerCount()).toBe(0);
		stopRestart();
	});

	it("retires an Add completed while stopped before restart and Remove", async () => {
		const added = asset("added");
		const addGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [added] });
		const service: PhotoService = {
			...memory,
			addPick: async (pick) => {
				await addGate.promise;
				return memory.addPick(pick);
			},
		};
		const store = createPickListStore(service);
		const stop = store.start();
		const adding = store.getState().toggle(added, origin);

		stop();
		addGate.resolve(undefined);
		await adding;
		const stopRestart = store.start();
		await store.getState().remove(added.id);

		expect(memory.getPicks().items).toHaveLength(0);
		expect(store.getState().count).toBe(0);
		stopRestart();
	});

	it("retires Undo completed while stopped before restart and Remove", async () => {
		const first = asset("first");
		const restoreGate = deferred<void>();
		const memory = createInMemoryPhotoService({ wallAssets: [first] });
		await memory.addPick(reference(first));
		const service: PhotoService = {
			...memory,
			restorePicks: async (cleared) => {
				await restoreGate.promise;
				return memory.restorePicks(cleared);
			},
		};
		const store = createPickListStore(service);
		const stop = store.start();
		await store.getState().clear();
		const restoring = store.getState().undoClear();

		stop();
		restoreGate.resolve(undefined);
		await restoring;
		const stopRestart = store.start();
		await store.getState().remove(first.id);

		expect(memory.getPicks().items).toHaveLength(0);
		expect(store.getState().count).toBe(0);
		stopRestart();
	});

	it("offers Clear Undo for exactly five seconds", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const service = createInMemoryPhotoService({ wallAssets: [first] });
		await service.addPick(reference(first));
		const store = createPickListStore(service);
		store.start();

		await store.getState().clear();
		expect(store.getState().count).toBe(0);
		expect(store.getState().toast?.action?.label).toBe("Undo");

		vi.advanceTimersByTime(4_999);
		expect(store.getState().toast?.action?.label).toBe("Undo");
		vi.advanceTimersByTime(1);
		expect(store.getState().toast).toBeNull();

		await store.getState().undoClear();
		expect(store.getState().count).toBe(0);
	});

	it("restores cleared entries before picks made after Clear", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const second = asset("second");
		const later = asset("later");
		const service = createInMemoryPhotoService({
			wallAssets: [first, second, later],
		});
		await service.addPick(reference(first));
		await service.addPick(reference(second));
		const store = createPickListStore(service);
		store.start();

		await store.getState().clear();
		await store.getState().toggle(later, origin);
		await store.getState().undoClear();

		expect(store.getState().snapshot.items.map((item) => item.assetId)).toEqual(
			["first", "second", "later"],
		);
		expect(store.getState().announcement).toBe("Picks restored");
	});

	it("exposes a browser persistence warning", () => {
		const warning =
			"Browser storage is unavailable. Picks may not survive reload.";
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([], 0, warning),
		};
		const store = createPickListStore(service);

		expect(store.getState().snapshot.persistenceError).toBe(warning);
		expect(store.getState().announcement).toBe(warning);
		expect(store.getState().toast?.message).toBe(warning);
	});

	it("cleans up the service subscription and Undo timer", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const memory = createInMemoryPhotoService({ wallAssets: [first] });
		await memory.addPick(reference(first));
		const unsubscribe = vi.fn();
		const service: PhotoService = {
			...memory,
			watchPicks: () => unsubscribe,
		};
		const store = createPickListStore(service);
		const stop = store.start();
		await store.getState().clear();

		stop();
		vi.advanceTimersByTime(5_000);

		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(store.getState().toast?.action?.label).toBe("Undo");
	});
});

describe("pick list provider", () => {
	it("derives origin from the active saved-folder identity and label", () => {
		expect(
			pickOriginFromSavedFolders({
				activeEntryId: "entry-a",
				hasOpenedFolder: true,
				persistenceError: null,
				access: {},
				entries: [
					{
						id: "entry-a",
						folderId: "stable-folder-id",
						name: "Camera",
						displayPath: "/photos/camera",
						customLabel: "Summer selects",
					},
				],
			}),
		).toEqual({
			sourceFolderId: "stable-folder-id",
			sourceLabel: "Summer selects",
		});
	});

	it("renders a polite live region, visible warning, and saved-folder origin", () => {
		const warning =
			"Browser storage is unavailable. Picks may not survive reload.";
		const memory = createInMemoryPhotoService();
		const service: PhotoService = {
			...memory,
			getPicks: () => snapshot([], 0, warning),
		};
		function OriginProbe() {
			const current = usePickListOrigin();
			return (
				<span>{`${current?.sourceFolderId}:${current?.sourceLabel}`}</span>
			);
		}

		const markup = renderToStaticMarkup(
			<PhotoServiceProvider service={service}>
				<PickListProvider origin={origin}>
					<OriginProbe />
				</PickListProvider>
			</PhotoServiceProvider>,
		);

		expect(markup).toContain('aria-live="polite"');
		expect(markup).toContain('aria-label="Pick storage warning"');
		expect(markup).toContain(warning);
		expect(markup).toContain("folder-a:Holiday");
	});

	it("keeps the Undo toast action focusable without requesting focus", () => {
		const markup = renderToStaticMarkup(
			<PickToast
				toast={{
					id: 1,
					phase: "visible",
					message: "Picks cleared",
					action: { label: "Undo", run: () => {} },
				}}
			/>,
		);

		expect(markup).toContain('<button type="button">Undo</button>');
		expect(markup).not.toContain("autofocus");
		expect(markup).not.toContain('tabindex="-1"');
	});

	it("consumes a rejected Undo action after the controller announces it", async () => {
		vi.useFakeTimers();
		const first = asset("first");
		const memory = createInMemoryPhotoService({ wallAssets: [first] });
		await memory.addPick(reference(first));
		const service: PhotoService = {
			...memory,
			restorePicks: async () => {
				throw new Error("Restore failed");
			},
		};
		const store = createPickListStore(service);
		const stop = store.start();
		await store.getState().clear();
		const action = store.getState().toast?.action;
		if (!action) throw new Error("Expected Undo action");

		await expect(consumePickToastAction(action)).resolves.toBeUndefined();

		expect(store.getState().announcement).toBe("Couldn't update picks");
		stop();
	});
});
