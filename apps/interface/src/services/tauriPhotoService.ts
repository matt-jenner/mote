import { Channel, invoke } from "@tauri-apps/api/core";
import {
	type Appearance,
	type BootstrapState,
	type ChooseFolderResult,
	type DerivativeReference,
	type DerivativeRequest,
	type GalleryScope,
	type PhotoService,
	PhotoServiceError,
	type SortDirection,
	type WallQueryRequest,
	type WallUpdate,
} from "./photoService";

const internalErrorMessage = "Photo Viewer could not complete that request.";

const nativeErrorMessages: Readonly<Record<string, string>> = {
	folderUnavailable: "The selected folder is unavailable.",
	folderNotDirectory: "Choose a folder, not a file.",
	folderOverlapsSource: "That folder overlaps an existing source.",
	folderOverlapsLocalState: "That folder overlaps Photo Viewer's local data.",
	localStateUnavailable: "Photo Viewer cannot open its local data.",
	invalidLimit: "The requested wall page is not valid.",
	assetNotFound: "That photo is no longer available.",
	derivativeUnavailable: "Some requested previews could not be generated.",
	internal: internalErrorMessage,
};

const internalError = () =>
	new PhotoServiceError("internal", internalErrorMessage);

const wallWatchRetryDelayMs = 100;
const maxWallWatchRetryDelayMs = 1_000;

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

export function createTauriPhotoService(
	invokeCommand: InvokeCommand = invoke,
	channelFactory: ChannelFactory = (listener) => new Channel(listener),
): PhotoService {
	let sortDirection: SortDirection = "oldestFirst";
	return {
		capabilities: {
			chooseFolder: true,
			folderSelection: "native",
			locateFolder: false,
		},
		getBootstrapState: () =>
			invokePhotoCommand<BootstrapState>(
				invokeCommand,
				"get_bootstrap_state",
				undefined,
			),
		chooseFolder: () =>
			invokePhotoCommand<ChooseFolderResult>(
				invokeCommand,
				"choose_folder",
				undefined,
			),
		updateAppearance: (appearance: Appearance) =>
			invokePhotoCommand<BootstrapState>(invokeCommand, "update_appearance", {
				appearance,
			}),
		updateGalleryScope: (scope: GalleryScope) =>
			invokePhotoCommand<BootstrapState>(
				invokeCommand,
				"update_gallery_scope",
				{
					scope,
				},
			),
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
