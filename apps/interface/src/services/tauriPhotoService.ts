import { Channel, invoke } from "@tauri-apps/api/core";
import {
	cloneSavedFolders,
	emptySavedFolders,
	type SavedFolderSnapshot,
	sortSavedFolders,
} from "../folders/savedFolders";
import type {
	PickItem,
	PickListSnapshot,
	PickReference,
} from "../picks/pickList";
import {
	type Appearance,
	type BootstrapState,
	type ChooseFolderResult,
	type CopyProgress,
	type CopyResult,
	type DerivativeReference,
	type DerivativeRequest,
	type GalleryScope,
	type PhotoService,
	PhotoServiceError,
	type SortDirection,
	type WallQueryRequest,
	type WallUpdate,
} from "./photoService";

const internalErrorMessage = "Mote could not complete that request.";

const nativeErrorMessages: Readonly<Record<string, string>> = {
	copyInProgress: "An original copy is already in progress.",
	copyDestinationIsSource: "Choose a destination outside your source folders.",
	copyDestinationUnavailable: "The copy destination is unavailable.",
	copyDestinationMissing: "The destination folder no longer exists.",
	copyPreparationFailed: "Mote could not prepare these originals.",
	folderUnavailable: "The selected folder is unavailable.",
	folderNotDirectory: "Choose a folder, not a file.",
	folderOverlapsSource: "That folder overlaps an existing source.",
	folderOverlapsLocalState: "That folder overlaps Mote's local data.",
	localStateUnavailable: "Mote cannot open its local data.",
	invalidLimit: "The requested wall page is not valid.",
	assetNotFound: "That photo is no longer available.",
	derivativeUnavailable: "Some requested previews could not be generated.",
	internal: internalErrorMessage,
};

const internalError = () =>
	new PhotoServiceError("internal", internalErrorMessage);

const wallWatchRetryDelayMs = 100;
const maxWallWatchRetryDelayMs = 1_000;
const maxPickBatchSize = 250;

export type WallSubscriptionId = string;

export type InvokeCommand = <T>(
	command: string,
	args?: Record<string, unknown>,
) => Promise<T>;

export interface ServiceChannel<T> {
	onmessage: (response: T) => void;
	toJSON(): string;
}

export type ChannelFactory = (
	listener: (update: WallUpdate) => void,
) => ServiceChannel<WallUpdate>;

interface NativePickListSnapshot {
	revision: number;
	items: PickItem[];
}

const clonePicks = (snapshot: PickListSnapshot): PickListSnapshot =>
	structuredClone(snapshot);

const fromNativePicks = (
	snapshot: NativePickListSnapshot,
): PickListSnapshot => ({
	revision: snapshot.revision,
	items: structuredClone(snapshot.items),
	persistenceError: null,
});

export function createTauriPhotoService(
	invokeCommand: InvokeCommand = invoke,
	channelFactory: ChannelFactory = (listener) => new Channel(listener),
	copyChannelFactory: (
		listener: (progress: CopyProgress) => void,
	) => ServiceChannel<CopyProgress> = (listener) => new Channel(listener),
): PhotoService {
	let sortDirection: SortDirection = "oldestFirst";

	let saved = emptySavedFolders();
	let lastState: BootstrapState | null = null;
	const listeners = new Set<(snapshot: SavedFolderSnapshot) => void>();
	let picks: PickListSnapshot = {
		revision: 0,
		items: [],
		persistenceError: null,
	};
	const pickListeners = new Set<(snapshot: PickListSnapshot) => void>();
	const publishPicks = (value: NativePickListSnapshot) => {
		if (value.revision < picks.revision) return clonePicks(picks);
		picks = fromNativePicks(value);
		for (const listener of pickListeners) listener(clonePicks(picks));
		return clonePicks(picks);
	};
	const invokePickCommand = (command: string, args?: Record<string, unknown>) =>
		invokePhotoCommand<NativePickListSnapshot>(invokeCommand, command, args);
	const pickCommand = async (command: string, args?: Record<string, unknown>) =>
		publishPicks(await invokePickCommand(command, args));
	const restorePicks = async (
		references: readonly PickReference[],
	): Promise<PickListSnapshot> => {
		if (references.length <= maxPickBatchSize) {
			return pickCommand("restore_photo_picks", {
				references: references.map((reference) => ({ ...reference })),
			});
		}
		const chunks: PickReference[][] = [];
		for (
			let offset = 0;
			offset < references.length;
			offset += maxPickBatchSize
		) {
			chunks.push(
				references
					.slice(offset, offset + maxPickBatchSize)
					.map((reference) => ({ ...reference })),
			);
		}
		let completedChunk = false;
		let finalSnapshot: NativePickListSnapshot | null = null;
		try {
			for (const chunk of chunks.reverse()) {
				finalSnapshot = await invokePickCommand("restore_photo_picks", {
					references: chunk,
				});
				completedChunk = true;
			}
		} catch (error) {
			if (completedChunk) {
				try {
					publishPicks(await invokePickCommand("list_photo_picks"));
				} catch {
					// Preserve the mutation failure if the recovery read also fails.
				}
			}
			throw error;
		}
		if (finalSnapshot === null) throw internalError();
		return publishPicks(finalSnapshot);
	};
	const requestPickDerivatives = async (
		request: DerivativeRequest,
	): Promise<void> => {
		const chunks =
			request.assetIds.length === 0
				? [[]]
				: Array.from(
						{
							length: Math.ceil(request.assetIds.length / maxPickBatchSize),
						},
						(_, index) =>
							request.assetIds.slice(
								index * maxPickBatchSize,
								(index + 1) * maxPickBatchSize,
							),
					);
		for (const assetIds of chunks) {
			await invokePhotoCommand<void>(
				invokeCommand,
				"request_pick_derivatives",
				{ request: { ...request, assetIds } },
			);
		}
	};
	const publish = (value: SavedFolderSnapshot) => {
		if ((value.revision ?? 0) < (saved.revision ?? 0))
			return cloneSavedFolders(saved);
		saved = cloneSavedFolders(value);
		saved.entries = sortSavedFolders(saved.entries);
		for (const listener of listeners) listener(cloneSavedFolders(saved));
		return cloneSavedFolders(saved);
	};
	const accept = (state: BootstrapState) => {
		state = { ...state, accentColor: "system" };
		if (
			(state.savedFolders?.revision ?? 0) < (saved.revision ?? 0) &&
			lastState
		)
			return { ...lastState, savedFolders: cloneSavedFolders(saved) };
		publish(state.savedFolders ?? emptySavedFolders());
		lastState = { ...state, savedFolders: cloneSavedFolders(saved) };
		return lastState;
	};
	const stateCommand = async (
		command: string,
		args?: Record<string, unknown>,
	) =>
		accept(
			await invokePhotoCommand<BootstrapState>(invokeCommand, command, args),
		);
	const selectCommand = async (
		command: string,
		args?: Record<string, unknown>,
	): Promise<ChooseFolderResult> => {
		const result = await invokePhotoCommand<ChooseFolderResult>(
			invokeCommand,
			command,
			args,
		);
		if (result.kind === "selected")
			return { kind: "selected", state: accept(result.state) };
		return result;
	};
	return {
		getPicks: () => clonePicks(picks),
		watchPicks(listener) {
			pickListeners.add(listener);
			return () => pickListeners.delete(listener);
		},
		loadPicks: () => pickCommand("list_photo_picks"),
		addPick: (reference: PickReference) =>
			pickCommand("add_photo_pick", {
				assetId: reference.assetId,
				sourceFolderId: reference.sourceFolderId,
			}),
		removePick: (assetId: string) =>
			pickCommand("remove_photo_pick", { assetId }),
		clearPicks: () => pickCommand("clear_photo_picks"),
		restorePicks,
		requestPickDerivatives,
		originalDownloadUrl: () => null,
		async copyPickedOriginals(assetIds, listener) {
			const channel = copyChannelFactory(listener);
			try {
				return await invokePhotoCommand<CopyResult>(
					invokeCommand,
					"copy_picked_originals",
					{
						assetIds: assetIds === null ? null : [...assetIds],
						onEvent: channel,
					},
				);
			} finally {
				channel.onmessage = () => {};
			}
		},
		showLastCopyDestination: () =>
			invokePhotoCommand<void>(
				invokeCommand,
				"show_last_copy_destination",
				undefined,
			),
		cancelOriginalCopy: () =>
			invokePhotoCommand<void>(invokeCommand, "cancel_original_copy", undefined),
		getSavedFolders: () => cloneSavedFolders(saved),
		watchSavedFolders: (listener) => {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
		renameSavedFolder: (id, label) =>
			stateCommand("rename_saved_folder", { id, label }),
		removeSavedFolder: (id) => stateCommand("remove_saved_folder", { id }),
		clearActiveFolder: () => stateCommand("clear_active_folder"),
		activateSavedFolder: async (id) => {
			const result = await selectCommand("activate_saved_folder", { id });
			if (result.kind === "cancelled")
				await stateCommand("check_saved_folders", { ids: [id] });
			return result;
		},
		checkSavedFolders: async (ids) =>
			(await stateCommand("check_saved_folders", { ids })).savedFolders,

		capabilities: {
			chooseFolder: true,
			folderSelection: "native",
			locateFolder: false,
			originalAction: "copy",
		},
		getBootstrapState: () => stateCommand("get_bootstrap_state"),
		chooseFolder: () => selectCommand("choose_folder"),
		updateAppearance: (appearance: Appearance) =>
			stateCommand("update_appearance", { appearance }),
		updateGalleryScope: (scope: GalleryScope) =>
			stateCommand("update_gallery_scope", { scope }),
		queryWall: (request: WallQueryRequest) =>
			invokePhotoCommand(invokeCommand, "query_wall", { request }),
		requestDerivatives: (request: DerivativeRequest) =>
			invokePhotoCommand(invokeCommand, "request_derivatives", { request }),
		setWallInteraction: (active: boolean) =>
			invokePhotoCommand(invokeCommand, "set_wall_interaction", { active }),
		watchWallUpdates(listener) {
			let active = true;
			let attempts = 0;
			let hadRegistrationFailure = false;
			let deliveredInitialResync = false;
			let registrationInFlight = false;
			let subscriptionId: WallSubscriptionId | null = null;
			let retryTimer: ReturnType<typeof setTimeout> | null = null;
			let channel: ServiceChannel<WallUpdate> | null = null;
			const unwatch = (id: WallSubscriptionId) => {
				void invokePhotoCommand<void>(invokeCommand, "unwatch_wall_updates", {
					subscriptionId: id,
				}).catch(() => undefined);
			};
			const stop = () => {
				active = false;
				if (retryTimer !== null) {
					clearTimeout(retryTimer);
					retryTimer = null;
				}
				const id = subscriptionId;
				subscriptionId = null;
				if (id !== null) unwatch(id);
				if (channel !== null) channel.onmessage = () => undefined;
			};
			const deliver = (update: WallUpdate) => {
				if (!active) return;
				if (update.kind === "sourceUnavailable") {
					const entry = saved.entries.find((e) => e.id === saved.activeEntryId);
					if (entry)
						publish({
							...saved,
							access: {
								...saved.access,
								[entry.folderId]: {
									folderId: entry.folderId,
									state: "rootOffline",
									generation: saved.access[entry.folderId]?.generation ?? 0,
									retryAfterMs: 0,
								},
							},
						});
				}
				try {
					listener(update);
				} catch {
					stop();
				}
			};
			channel = channelFactory(deliver);
			channel.onmessage = active ? deliver : () => undefined;
			const register = () => {
				if (!active || registrationInFlight || subscriptionId !== null) return;
				registrationInFlight = true;
				attempts += 1;
				void invokePhotoCommand<WallSubscriptionId>(
					invokeCommand,
					"watch_wall_updates",
					{ onEvent: channel },
				)
					.then((id) => {
						registrationInFlight = false;
						if (!active) {
							unwatch(id);
							return;
						}
						subscriptionId = id;
						if (!deliveredInitialResync || hadRegistrationFailure) {
							deliveredInitialResync = true;
							deliver({ kind: "resyncRequired", selectionId: "" });
							if (!active) return;
						}
					})
					.catch(() => {
						registrationInFlight = false;
						if (!active) return;
						hadRegistrationFailure = true;
						queueMicrotask(() => {
							if (active) deliver({ kind: "resyncRequired", selectionId: "" });
						});
						retryTimer = setTimeout(
							() => {
								retryTimer = null;
								register();
							},
							Math.min(
								wallWatchRetryDelayMs * attempts,
								maxWallWatchRetryDelayMs,
							),
						);
					});
			};
			register();
			return stop;
		},
		derivativeUrl(reference: DerivativeReference) {
			return `photo-derivative://localhost/${reference.assetId}/${reference.kind}/${reference.key}`;
		},
		async listFolders() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"This host uses its system folder picker.",
			);
		},
		async selectFolder() {
			throw new PhotoServiceError(
				"unsupportedCapability",
				"This host uses its system folder picker.",
			);
		},
		folderBrowserState() {
			return { breadcrumbs: [], initialPath: "" };
		},
		initialSortDirection() {
			return sortDirection;
		},
		rememberSortDirection(direction: SortDirection) {
			sortDirection = direction;
		},
	};
}

async function invokePhotoCommand<T>(
	invokeCommand: InvokeCommand,
	command: string,
	args: Record<string, unknown> | undefined,
): Promise<T> {
	try {
		return await invokeCommand<T>(command, args);
	} catch (reason) {
		throw toPhotoServiceError(reason);
	}
}

function toPhotoServiceError(reason: unknown): PhotoServiceError {
	if (typeof reason !== "object" || reason === null) return internalError();
	const candidate = reason as Record<string, unknown>;
	if (
		typeof candidate.code !== "string" ||
		typeof candidate.message !== "string"
	) {
		return internalError();
	}
	const expectedMessage = nativeErrorMessages[candidate.code];
	if (candidate.message !== expectedMessage) return internalError();
	return new PhotoServiceError(candidate.code, candidate.message);
}
