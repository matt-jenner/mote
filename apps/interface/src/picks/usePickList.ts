import { useEffect, useMemo, useSyncExternalStore } from "react";
import type {
	DerivativeClass,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import type { PickItem, PickListSnapshot, PickReference } from "./pickList";

export interface PickOrigin {
	sourceFolderId: string;
	sourceLabel: string;
}

export interface PickToastAction {
	label: string;
	run(): void;
}

export interface PickToastState {
	id: number;
	message: string;
	action?: PickToastAction;
}

export interface PickListController {
	snapshot: PickListSnapshot;
	count: number;
	isPicked(assetId: string): boolean;
	toggle(asset: WallAsset, origin: PickOrigin): Promise<void>;
	remove(assetId: string): Promise<void>;
	clear(): Promise<void>;
	undoClear(): Promise<void>;
	requestDerivatives(assetIds: readonly string[], kind: DerivativeClass): void;
	announcement: string;
	announcementId: number;
	toast: PickToastState | null;
	dismissToast(): void;
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

	private acceptSnapshot(next: PickListSnapshot): void {
		const current = this.authoritative;
		if (next.revision < current.revision) return;
		this.authoritative = cloneSnapshot(next);
		if (
			next.persistenceError &&
			next.persistenceError !== current.persistenceError
		) {
			this.publishMessage(next.persistenceError, false);
			return;
		}
		this.publish();
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

	private acceptMutation(id: number, snapshot: PickListSnapshot): void {
		this.removeOperation(id);
		this.acceptSnapshot(snapshot);
	}

	private rejectMutation(id: number): void {
		this.removeOperation(id);
		this.publishMessage("Couldn't update picks");
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
		const mutation = persist().then(
			(snapshot) => {
				this.acceptMutation(operation.id, snapshot);
				this.publishMessage(message);
			},
			(error: unknown) => {
				this.rejectMutation(operation.id);
				throw error;
			},
		);
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
		const mutation = this.service.clearPicks().then(
			(snapshot) => {
				this.acceptMutation(operation.id, snapshot);
				this.undo = {
					items: cleared,
					expiresAt: Date.now() + undoDurationMs,
				};
				this.announcement = "Picks cleared";
				this.announcementId += 1;
				this.toast = {
					id: ++this.toastId,
					message: "Picks cleared",
					action: { label: "Undo", run: () => void this.undoClear() },
				};
				this.scheduleToastDismissal(undoDurationMs);
				this.publish();
			},
			(error: unknown) => {
				this.rejectMutation(operation.id);
				throw error;
			},
		);
		this.clearPromise = mutation;
		const release = () => {
			if (this.clearPromise === mutation) this.clearPromise = null;
		};
		void mutation.then(release, release);
		return mutation;
	};

	private undoClear = async (): Promise<void> => {
		const undo = this.undo;
		if (!undo || Date.now() >= undo.expiresAt) return;
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
		try {
			const restored = await this.service.restorePicks(
				undo.items.map(({ assetId, sourceFolderId, sourceLabel }) => ({
					assetId,
					sourceFolderId,
					sourceLabel,
				})),
			);
			this.acceptMutation(operation.id, restored);
			this.publishMessage("Picks restored");
		} catch (error) {
			this.rejectMutation(operation.id);
			throw error;
		}
	};

	private requestDerivatives = (
		assetIds: readonly string[],
		kind: DerivativeClass,
	): void => {
		void this.service
			.requestPickDerivatives({
				assetIds: [...assetIds],
				kind,
				priority: "visible",
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
