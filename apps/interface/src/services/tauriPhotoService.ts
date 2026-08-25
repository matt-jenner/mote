import { Channel, invoke } from "@tauri-apps/api/core";
import {
	type Appearance,
	type BootstrapState,
	type ChooseFolderResult,
	type DerivativeReference,
	type DerivativeRequest,
	type PhotoService,
	PhotoServiceError,
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
	internal: internalErrorMessage,
};

const internalError = () =>
	new PhotoServiceError("internal", internalErrorMessage);

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
	return {
		capabilities: { chooseFolder: true, locateFolder: false },
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
		queryWall: (request: WallQueryRequest) =>
			invokePhotoCommand(invokeCommand, "query_wall", { request }),
		requestDerivatives: (request: DerivativeRequest) =>
			invokePhotoCommand(invokeCommand, "request_derivatives", { request }),
		setWallInteraction: (active: boolean) =>
			invokePhotoCommand(invokeCommand, "set_wall_interaction", { active }),
		watchWallUpdates(listener) {
			const channel = channelFactory(listener);
			void invokePhotoCommand<void>(invokeCommand, "watch_wall_updates", {
				onEvent: channel,
			}).catch(() => undefined);
			channel.onmessage = listener;
			return () => {
				channel.onmessage = () => undefined;
			};
		},
		derivativeUrl(reference: DerivativeReference) {
			return `photo-derivative://localhost/${reference.assetId}/${reference.kind}/${reference.key}`;
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
