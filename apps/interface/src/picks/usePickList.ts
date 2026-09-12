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
	message: string;
	action?: PickToastAction;
}

export interface PickListController {
	copy: PickCopyState;
	copyOriginals(): Promise<void>;
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
	phase: "idle" | "choosing" | "copying" | "complete" | "partial";
	completed: number;
	total: number;
	copiedCount: number;
	failedAssetIds: readonly string[];
	message: string;
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
const confirmationDurationMs = 3_000;

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
	private copy: PickCopyState = {
		phase: "idle",
		completed: 0,
		total: 0,
		copiedCount: 0,
		failedAssetIds: [],
		message: "",
	};
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
	private toastId = 0;
	private toastTimer: ReturnType<typeof setTimeout> | null = null;
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
				this.activeStop = null;
			}
		};
		this.activeStop = stop;
		return stop;
	};

	private buildState(): PickListController {
		const snapshot = applyOperations(this.authoritative, this.operations);
		return {
			copy: this.copy,
			copyOriginals: this.copyOriginals,
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

	private copyOriginals = async (): Promise<void> => {
		if (this.copy.phase === "choosing" || this.copy.phase === "copying") return;
		const previous = this.copy;
		const assetIds =
			previous.phase === "partial"
				? [...previous.failedAssetIds]
				: this.state.snapshot.items.map((item) => item.assetId);
		if (assetIds.length === 0) return;
		this.copy = {
			...previous,
			phase: "choosing",
			total: assetIds.length,
			completed: 0,
		};
		this.publish();
		try {
			const result = await this.service.copyPickedOriginals(
				assetIds,
				(progress: CopyProgress) => {
					this.copy = {
						...this.copy,
						phase: "copying",
						completed: progress.completed,
						total: progress.total,
					};
					this.publish();
				},
			);
			if (result.kind === "cancelled") {
				this.copy = previous;
				this.publish();
				return;
			}
			const total = result.items.length;
			const message =
				result.failedCount > 0
					? `Copied ${result.copiedCount} of ${total}`
					: `Copied ${result.copiedCount} originals`;
			this.copy = {
				phase: result.failedCount > 0 ? "partial" : "complete",
				total,
				completed: total,
				copiedCount: result.copiedCount,
				failedAssetIds: result.items
					.filter((item) => item.status === "failed")
					.map((item) => item.assetId),
				message,
			};
			this.announcement = message;
			this.announcementId += 1;
			this.toast = {
				id: ++this.toastId,
				message,
				...(result.copiedCount > 0
					? { action: { label: "Show folder", run: this.showCopyFolder } }
					: {}),
			};
			this.scheduleToastDismissal(confirmationDurationMs);
			this.publish();
		} catch (error) {
			this.copy = previous;
			this.publishMessage(
				error instanceof PhotoServiceError
					? error.message
					: "Couldn't copy originals",
			);
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

	private publishMessage(message: string, temporary = true): void {
		this.announcement = message;
		this.announcementId += 1;
		if (!this.undo) {
			this.toast = { id: ++this.toastId, message };
			if (temporary) this.scheduleToastDismissal(confirmationDurationMs);
		}
		this.publish();
	}

	private scheduleToastDismissal(delay: number): void {
		this.clearToastTimer();
		this.toastTimer = setTimeout(() => {
			this.toastTimer = null;
			this.toast = null;
			this.undo = null;
			this.publish();
		}, delay);
	}

	private clearToastTimer(): void {
		if (this.toastTimer !== null) clearTimeout(this.toastTimer);
		this.toastTimer = null;
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
		this.publish();
		const lifecycleId = this.lifecycleId;
		const mutation = this.enqueueMutation(async () => {
			try {
				const snapshot = await persist();
				const announce = this.lifecycleIsActive(lifecycleId);
				this.acceptMutation(operation.id, snapshot, announce);
				if (announce) {
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
					message: "Picks cleared",
					action: { label: "Undo", run: this.undoClear },
				};
				this.scheduleToastDismissal(undoDurationMs);
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
