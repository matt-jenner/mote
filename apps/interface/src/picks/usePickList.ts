import { useEffect, useMemo, useSyncExternalStore } from "react";
import type {
	CopyProgress,
	DerivativeClass,
	DerivativePriority,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import { PhotoServiceError } from "../services/photoService";
import type { PickItem, PickListSnapshot, PickReference } from "./pickList";

export interface PickOrigin {
	sourceFolderId: string;
	sourceLabel: string;
}

export interface PickToastAction {
	label: string;
	run(): void | Promise<void>;
}

export interface PickToastState {
	id: number;
	phase: "visible" | "exiting";
	message: string;
	action?: PickToastAction;
}

export interface PickListController {
	copy: PickCopyState;
	copyOriginals(): Promise<void>;
	cancelCopy(): Promise<void>;
	showCopyFolder(): Promise<void>;
	snapshot: PickListSnapshot;
	count: number;
	isPicked(assetId: string): boolean;
	toggle(asset: WallAsset, origin: PickOrigin): Promise<void>;
	remove(assetId: string): Promise<void>;
	clear(): Promise<void>;
	undoClear(): Promise<void>;
	requestDerivatives(
		assetIds: readonly string[],
		kind: DerivativeClass,
		priority?: DerivativePriority,
	): void;
	announcement: string;
	announcementId: number;
	toast: PickToastState | null;
	dismissToast(): void;
}

export interface PickCopyState {
	phase:
		| "idle"
		| "choosing"
		| "copying"
		| "cancelling"
		| "complete"
		| "partial";
	completed: number;
	total: number;
	copiedCount: number;
	failedAssetIds: readonly string[];
	failures: readonly PickCopyFailure[];
	message: string;
}

export type PickCopyFailureCode =
	| "source_unavailable"
	| "destination_unavailable"
	| "destination_is_source"
	| "copy_failed";

export interface PickCopyFailure {
	assetId: string;
	code: PickCopyFailureCode;
}

function boundedCopyFailureCode(code: string | null): PickCopyFailureCode {
	switch (code) {
		case "source_unavailable":
		case "destination_unavailable":
		case "destination_is_source":
			return code;
		default:
			return "copy_failed";
	}
}

type OptimisticOperation =
	| { id: number; kind: "add"; item: PickItem }
	| { id: number; kind: "remove"; assetId: string }
	| { id: number; kind: "clear" }
	| { id: number; kind: "restore"; items: PickItem[] };

interface UndoState {
	items: PickItem[];
	expiresAt: number;
}

interface QueuedMutation {
	run(): Promise<void>;
	resolve(): void;
	reject(error: unknown): void;
}

export interface PickListStore {
	getState(): PickListController;
	subscribe(listener: () => void): () => void;
	start(): () => void;
}

const undoDurationMs = 5_000;
const confirmationDurationMs = 1_000;
const copyFailureDurationMs = 5_000;
const toastFadeDurationMs = 200;
const initialCopy: PickCopyState = {
	phase: "idle",
	completed: 0,
	total: 0,
	copiedCount: 0,
	failedAssetIds: [],
	failures: [],
	message: "",
};

function cloneSnapshot(snapshot: PickListSnapshot): PickListSnapshot {
	return {
		...snapshot,
		items: snapshot.items.map((item) => ({ ...item })),
	};
}

function contains(items: readonly PickItem[], assetId: string): boolean {
	return items.some((item) => item.assetId === assetId);
}

function mergeRestored(
	restored: readonly PickItem[],
	current: readonly PickItem[],
): PickItem[] {
	const merged: PickItem[] = [];
	for (const item of [...restored, ...current]) {
		if (!contains(merged, item.assetId)) merged.push({ ...item });
	}
	return merged;
}

function applyOperations(
	authoritative: PickListSnapshot,
	operations: readonly OptimisticOperation[],
): PickListSnapshot {
	let items = authoritative.items.map((item) => ({ ...item }));
	for (const operation of operations) {
		switch (operation.kind) {
			case "add":
				if (!contains(items, operation.item.assetId))
					items = [...items, { ...operation.item }];
				break;
			case "remove":
				items = items.filter((item) => item.assetId !== operation.assetId);
				break;
			case "clear":
				items = [];
				break;
			case "restore":
				items = mergeRestored(operation.items, items);
				break;
		}
	}
	return { ...authoritative, items };
}

class PickListStoreImplementation implements PickListStore {
	private copy: PickCopyState = initialCopy;
	private copyPromise: Promise<void> | null = null;
	private cancelCommand: Promise<void> | null = null;
	private activeCopyId: number | null = null;
	private authoritative: PickListSnapshot;
	private operations: OptimisticOperation[] = [];
	private readonly listeners = new Set<() => void>();
	private readonly pendingAssets = new Map<string, Promise<void>>();
	private activeStop: (() => void) | null = null;
	private operationId = 0;
	private lifecycleId = 0;
	private announcement = "";
	private announcementId = 0;
	private toast: PickToastState | null = null;
	private deferredCopyToast: PickToastState | null = null;
	private deferredCopyToastDurationMs = confirmationDurationMs;
	private toastId = 0;
	private toastTimer: ReturnType<typeof setTimeout> | null = null;
	private toastFadeTimer: ReturnType<typeof setTimeout> | null = null;
	private undoTimer: ReturnType<typeof setTimeout> | null = null;
	private undoToast: PickToastState | null = null;
	private undo: UndoState | null = null;
	private clearPromise: Promise<void> | null = null;
	private readonly mutationQueue: QueuedMutation[] = [];
	private mutationRunning = false;
	private state: PickListController;

	constructor(private readonly service: PhotoService) {
		this.authoritative = cloneSnapshot(service.getPicks());
		if (this.authoritative.persistenceError) {
			this.announcement = this.authoritative.persistenceError;
			this.announcementId = 1;
			this.toast = {
				id: ++this.toastId,
				phase: "visible",
				message: this.authoritative.persistenceError,
			};
		}
		this.state = this.buildState();
	}

	getState = (): PickListController => this.state;

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};

	start = (): (() => void) => {
		this.activeStop?.();
		const lifecycleId = ++this.lifecycleId;
		const unsubscribe = this.service.watchPicks((snapshot) => {
			if (lifecycleId === this.lifecycleId) this.acceptSnapshot(snapshot);
		});
		void this.service.loadPicks().then(
			(snapshot) => {
				if (lifecycleId === this.lifecycleId) this.acceptSnapshot(snapshot);
			},
			() => {
				if (lifecycleId === this.lifecycleId)
					this.publishMessage("Couldn't load picks");
			},
		);

		let stopped = false;
		const stop = () => {
			if (stopped) return;
			stopped = true;
			unsubscribe();
			if (lifecycleId === this.lifecycleId) {
				this.lifecycleId += 1;
				this.clearToastTimer();
				this.clearUndoTimer();
				this.activeStop = null;
			}
		};
		this.activeStop = stop;
		return stop;
	};

	private buildState(): PickListController {
		const snapshot = applyOperations(this.authoritative, this.operations);
		// A running attempt owns its snapshot. Reconcile retry eligibility only
		// after it settles, using saved membership so rejected removals stay retryable.
		if (this.copy.phase === "partial") {
			const savedIds = new Set(
				this.authoritative.items.map((item) => item.assetId),
			);
			const failures = this.copy.failures.filter((failure) =>
				savedIds.has(failure.assetId),
			);
			if (failures.length !== this.copy.failures.length) {
				this.copy = {
					...this.copy,
					failures,
					failedAssetIds: failures.map((failure) => failure.assetId),
					phase: failures.length > 0 ? "partial" : "complete",
				};
			}
		}
		return {
			copy: this.copy,
			copyOriginals: this.copyOriginals,
			cancelCopy: this.cancelCopy,
			showCopyFolder: this.showCopyFolder,
			snapshot,
			count: snapshot.items.length,
			isPicked: this.isPicked,
			toggle: this.toggle,
			remove: this.remove,
			clear: this.clear,
			undoClear: this.undoClear,
			requestDerivatives: this.requestDerivatives,
			announcement: this.announcement,
			announcementId: this.announcementId,
			toast: this.toast,
			dismissToast: this.dismissToast,
		};
	}

	private publish(): void {
		this.state = this.buildState();
		for (const listener of this.listeners) listener();
	}

	private copyOriginals = (): Promise<void> => {
		if (this.copyPromise) return this.copyPromise;
		const operation = this.runCopy();
		this.copyPromise = operation;
		void operation.then(() => {
			if (this.copyPromise === operation) {
				this.copyPromise = null;
				this.cancelCommand = null;
			}
		});
		return operation;
	};

	private cancelCopy = (): Promise<void> => {
		if (this.copy.phase !== "copying" && this.copy.phase !== "cancelling")
			return Promise.resolve();
		if (!this.cancelCommand) {
			const copyId = this.activeCopyId;
			this.copy = { ...this.copy, phase: "cancelling" };
			const command: Promise<void> = this.service
				.cancelOriginalCopy()
				.catch((error) => {
					if (this.activeCopyId !== copyId || this.cancelCommand !== command)
						return;
					this.cancelCommand = null;
					if (this.copy.phase === "cancelling")
						this.copy = { ...this.copy, phase: "copying" };
					this.publishMessage(
						error instanceof PhotoServiceError
							? error.message
							: "Couldn't cancel the copy",
						true,
						false,
						copyFailureDurationMs,
					);
				});
			this.cancelCommand = command;
			this.publish();
		}
		return Promise.all([this.cancelCommand, this.copyPromise]).then(() => {});
	};

	private runCopy = async (): Promise<void> => {
		const previous = this.copy;
		const assetIds: readonly string[] | null =
			previous.phase === "partial" ? [...previous.failedAssetIds] : null;
		const total =
			assetIds === null ? this.state.snapshot.items.length : assetIds.length;
		if (total === 0) return;
		const copyId = ++this.operationId;
		this.activeCopyId = copyId;
		this.copy = {
			...previous,
			phase: "choosing",
			total,
			completed: 0,
		};
		this.publish();
		try {
			const result = await this.service.copyPickedOriginals(
				assetIds,
				(progress: CopyProgress) => {
					if (this.activeCopyId !== copyId) return;
					this.copy = {
						...this.copy,
						phase: this.copy.phase === "cancelling" ? "cancelling" : "copying",
						completed: progress.completed,
						total: progress.total,
					};
					this.publish();
				},
			);
			// Stay in this continuation when settling, so a newly accepted retry
			// cannot appear between draining cancellation and publishing completion.
			while (this.cancelCommand) {
				const command = this.cancelCommand;
				await command;
				if (this.cancelCommand === command) break;
			}
			if (
				result.kind === "selectionCancelled" ||
				result.kind === "copyCancelled"
			) {
				this.copy = previous;
				if (result.kind === "copyCancelled")
					this.publishMessage("Copy cancelled", true, true);
				else this.publish();
				return;
			}
			const total = result.items.length;
			const message =
				result.failedCount > 0
					? `Copied ${result.copiedCount} of ${total}`
					: `Copied ${result.copiedCount} originals`;
			const failures = result.items
				.filter((item) => item.status === "failed")
				.map((item) => ({
					assetId: item.assetId,
					code: boundedCopyFailureCode(item.errorCode),
				}));
			this.copy = {
				phase: result.failedCount > 0 ? "partial" : "complete",
				total,
				completed: total,
				copiedCount: result.copiedCount,
				failedAssetIds: failures.map((failure) => failure.assetId),
				failures,
				message,
			};
			this.announcement = message;
			this.announcementId += 1;
			const completionToast = {
				id: ++this.toastId,
				phase: "visible" as const,
				message,
				...(result.copiedCount > 0
					? { action: { label: "Show folder", run: this.showCopyFolder } }
					: {}),
			};
			if (this.undo && Date.now() < this.undo.expiresAt) {
				this.deferredCopyToast = completionToast;
				this.deferredCopyToastDurationMs =
					result.failedCount > 0
						? copyFailureDurationMs
						: confirmationDurationMs;
			} else {
				this.toast = completionToast;
				this.deferredCopyToast = null;
				this.scheduleToastDismissal(
					result.failedCount > 0
						? copyFailureDurationMs
						: confirmationDurationMs,
				);
			}
			this.publish();
		} catch (error) {
			while (this.cancelCommand) {
				const command = this.cancelCommand;
				await command;
				if (this.cancelCommand === command) break;
			}
			this.copy = previous;
			this.publishMessage(
				error instanceof PhotoServiceError
					? error.message
					: "Couldn't copy originals",
				true,
				false,
				copyFailureDurationMs,
			);
		} finally {
			if (this.activeCopyId === copyId) this.activeCopyId = null;
		}
	};

	private showCopyFolder = async (): Promise<void> => {
		if (this.copy.copiedCount === 0) return;
		try {
			await this.service.showLastCopyDestination();
		} catch (error) {
			this.publishMessage(
				error instanceof PhotoServiceError
					? error.message
					: "Couldn't open the copy destination",
				true,
				false,
				copyFailureDurationMs,
			);
		}
	};

	private rebuildState(notifyListeners: boolean): void {
		if (notifyListeners) {
			this.publish();
			return;
		}
		this.state = this.buildState();
	}

	private acceptSnapshot(
		next: PickListSnapshot,
		notifyListeners = true,
		announceWarning = true,
	): void {
		const current = this.authoritative;
		if (next.revision < current.revision) {
			this.rebuildState(notifyListeners);
			return;
		}
		this.authoritative = cloneSnapshot(next);
		if (
			announceWarning &&
			next.persistenceError &&
			next.persistenceError !== current.persistenceError
		) {
			this.publishMessage(next.persistenceError, false);
			return;
		}
		this.rebuildState(notifyListeners);
	}

	private publishMessage(
		message: string,
		temporary = true,
		interruptUndo = false,
		durationMs = confirmationDurationMs,
	): void {
		this.announcement = message;
		this.announcementId += 1;
		if (!this.undo || interruptUndo) {
			this.toast = { id: ++this.toastId, message, phase: "visible" };
			if (temporary) this.scheduleToastDismissal(durationMs);
		}
		this.publish();
	}

	private scheduleToastDismissal(delay: number, ownsUndo = false): void {
		this.clearToastTimer();
		if (ownsUndo) {
			this.clearUndoTimer();
			this.undoToast = this.toast;
			this.undoTimer = setTimeout(() => {
				this.undoTimer = null;
				this.undo = null;
				if (this.toast?.id === this.undoToast?.id) {
					this.toast = this.deferredCopyToast;
					this.deferredCopyToast = null;
					if (this.toast) {
						const deferredDuration = this.deferredCopyToastDurationMs;
						this.deferredCopyToastDurationMs = confirmationDurationMs;
						this.scheduleToastDismissal(deferredDuration);
					}
				}
				this.undoToast = null;
				this.publish();
			}, delay);
			return;
		}
		const id = this.toast?.id;
		this.toastFadeTimer = setTimeout(
			() => {
				this.toastFadeTimer = null;
				if (!this.toast || this.toast.id !== id) return;
				this.toast = { ...this.toast, phase: "exiting" };
				this.publish();
			},
			Math.max(0, delay - toastFadeDurationMs),
		);
		this.toastTimer = setTimeout(() => {
			this.toastTimer = null;
			if (this.toast?.id !== id) return;
			if (this.undo && Date.now() < this.undo.expiresAt)
				this.toast = this.undoToast;
			else {
				this.toast = this.deferredCopyToast;
				this.deferredCopyToast = null;
				if (this.toast) {
					const deferredDuration = this.deferredCopyToastDurationMs;
					this.deferredCopyToastDurationMs = confirmationDurationMs;
					this.scheduleToastDismissal(deferredDuration);
				}
			}
			this.publish();
		}, delay);
	}

	private clearToastTimer(): void {
		if (this.toastTimer !== null) clearTimeout(this.toastTimer);
		if (this.toastFadeTimer !== null) clearTimeout(this.toastFadeTimer);
		this.toastTimer = null;
		this.toastFadeTimer = null;
	}

	private clearUndoTimer(): void {
		if (this.undoTimer !== null) clearTimeout(this.undoTimer);
		this.undoTimer = null;
	}

	private removeOperation(id: number): void {
		this.operations = this.operations.filter(
			(operation) => operation.id !== id,
		);
	}

	private acceptMutation(
		id: number,
		snapshot: PickListSnapshot,
		announce: boolean,
	): void {
		this.removeOperation(id);
		this.acceptSnapshot(snapshot, this.activeStop !== null, announce);
	}

	private rejectMutation(id: number, announce: boolean): void {
		this.removeOperation(id);
		if (announce) {
			this.publishMessage("Couldn't update picks");
			return;
		}
		this.rebuildState(this.activeStop !== null);
	}

	private lifecycleIsActive(lifecycleId: number): boolean {
		return this.activeStop !== null && lifecycleId === this.lifecycleId;
	}

	private enqueueMutation(run: () => Promise<void>): Promise<void> {
		const mutation = new Promise<void>((resolve, reject) => {
			this.mutationQueue.push({ run, resolve, reject });
		});
		this.runNextMutation();
		return mutation;
	}

	private runNextMutation(): void {
		if (this.mutationRunning) return;
		const mutation = this.mutationQueue.shift();
		if (!mutation) return;
		this.mutationRunning = true;
		let result: Promise<void>;
		try {
			result = mutation.run();
		} catch (error) {
			this.finishMutation();
			mutation.reject(error);
			return;
		}
		void result.then(
			() => {
				this.finishMutation();
				mutation.resolve();
			},
			(error: unknown) => {
				this.finishMutation();
				mutation.reject(error);
			},
		);
	}

	private finishMutation(): void {
		this.mutationRunning = false;
		this.runNextMutation();
	}

	private mutateAsset(
		assetId: string,
		operation: OptimisticOperation,
		persist: () => Promise<PickListSnapshot>,
		message: string,
	): Promise<void> {
		const current = this.pendingAssets.get(assetId);
		if (current) return current;

		this.operations = [...this.operations, operation];
		if (operation.kind === "add") this.publishMessage(message, true, true);
		else this.publish();
		const lifecycleId = this.lifecycleId;
		const mutation = this.enqueueMutation(async () => {
			try {
				const snapshot = await persist();
				const announce = this.lifecycleIsActive(lifecycleId);
				this.acceptMutation(operation.id, snapshot, announce);
				if (announce && operation.kind !== "add") {
					this.publishMessage(message);
				}
			} catch (error) {
				this.rejectMutation(operation.id, this.lifecycleIsActive(lifecycleId));
				throw error;
			}
		});
		this.pendingAssets.set(assetId, mutation);
		const release = () => {
			if (this.pendingAssets.get(assetId) === mutation)
				this.pendingAssets.delete(assetId);
		};
		void mutation.then(release, release);
		return mutation;
	}

	private isPicked = (assetId: string): boolean =>
		contains(this.state.snapshot.items, assetId);

	private toggle = (asset: WallAsset, origin: PickOrigin): Promise<void> => {
		const id = ++this.operationId;
		if (this.isPicked(asset.id)) {
			return this.mutateAsset(
				asset.id,
				{ id, kind: "remove", assetId: asset.id },
				() => this.service.removePick(asset.id),
				"Removed from picks",
			);
		}
		const reference: PickReference = {
			assetId: asset.id,
			sourceFolderId: origin.sourceFolderId,
			sourceLabel: origin.sourceLabel,
		};
		return this.mutateAsset(
			asset.id,
			{ id, kind: "add", item: { ...reference, asset } },
			() => this.service.addPick(reference),
			"Added to picks",
		);
	};

	private remove = (assetId: string): Promise<void> => {
		const pending = this.pendingAssets.get(assetId);
		if (pending) return pending;
		if (!this.isPicked(assetId)) return Promise.resolve();
		const id = ++this.operationId;
		return this.mutateAsset(
			assetId,
			{ id, kind: "remove", assetId },
			() => this.service.removePick(assetId),
			"Removed from picks",
		);
	};

	private clear = (): Promise<void> => {
		if (
			this.copy.phase === "copying" ||
			this.copy.phase === "cancelling" ||
			this.copy.phase === "choosing"
		)
			return Promise.resolve();
		if (this.clearPromise) return this.clearPromise;
		const cleared = this.state.snapshot.items.map((item) => ({ ...item }));
		if (cleared.length === 0) return Promise.resolve();
		const operation: OptimisticOperation = {
			id: ++this.operationId,
			kind: "clear",
		};
		this.operations = [...this.operations, operation];
		this.publish();
		const lifecycleId = this.lifecycleId;
		const mutation = this.enqueueMutation(async () => {
			try {
				const snapshot = await this.service.clearPicks();
				this.copy = initialCopy;
				this.deferredCopyToast = null;
				const announce = this.lifecycleIsActive(lifecycleId);
				this.acceptMutation(operation.id, snapshot, announce);
				if (!announce) return;
				this.undo = {
					items: cleared,
					expiresAt: Date.now() + undoDurationMs,
				};
				this.announcement = "Picks cleared";
				this.announcementId += 1;
				this.toast = {
					id: ++this.toastId,
					phase: "visible",
					message: "Picks cleared",
					action: { label: "Undo", run: this.undoClear },
				};
				this.scheduleToastDismissal(undoDurationMs, true);
				this.publish();
			} catch (error) {
				this.rejectMutation(operation.id, this.lifecycleIsActive(lifecycleId));
				throw error;
			}
		});
		this.clearPromise = mutation;
		const release = () => {
			if (this.clearPromise === mutation) this.clearPromise = null;
		};
		void mutation.then(release, release);
		return mutation;
	};

	private undoClear = (): Promise<void> => {
		const undo = this.undo;
		if (!undo || Date.now() >= undo.expiresAt) return Promise.resolve();
		this.undo = null;
		this.undoToast = null;
		this.clearUndoTimer();
		this.clearToastTimer();
		this.toast = null;
		const operation: OptimisticOperation = {
			id: ++this.operationId,
			kind: "restore",
			items: undo.items,
		};
		this.operations = [...this.operations, operation];
		this.publish();
		const lifecycleId = this.lifecycleId;
		return this.enqueueMutation(async () => {
			try {
				const restored = await this.service.restorePicks(
					undo.items.map(({ assetId, sourceFolderId, sourceLabel }) => ({
						assetId,
						sourceFolderId,
						sourceLabel,
					})),
				);
				const announce = this.lifecycleIsActive(lifecycleId);
				this.acceptMutation(operation.id, restored, announce);
				if (announce) {
					this.publishMessage("Picks restored");
				}
			} catch (error) {
				this.rejectMutation(operation.id, this.lifecycleIsActive(lifecycleId));
				throw error;
			}
		});
	};

	private requestDerivatives = (
		assetIds: readonly string[],
		kind: DerivativeClass,
		priority: DerivativePriority = "visible",
	): void => {
		void this.service
			.requestPickDerivatives({
				assetIds: [...assetIds],
				kind,
				priority,
			})
			.catch(() => {});
	};

	private dismissToast = (): void => {
		this.clearToastTimer();
		this.clearUndoTimer();
		this.undoToast = null;
		this.toast = null;
		this.undo = null;
		this.publish();
	};
}

export function createPickListStore(service: PhotoService): PickListStore {
	return new PickListStoreImplementation(service);
}

export function usePickListController(
	service: PhotoService,
): PickListController {
	const store = useMemo(() => createPickListStore(service), [service]);
	useEffect(() => store.start(), [store]);
	return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}
