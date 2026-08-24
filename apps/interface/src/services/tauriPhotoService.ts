import { invoke } from "@tauri-apps/api/core";
import {
	type Appearance,
	type BootstrapState,
	type ChooseFolderResult,
	type PhotoService,
	PhotoServiceError,
} from "./photoService";

const internalErrorMessage = "Photo Viewer could not complete that request.";

const nativeErrorMessages: Readonly<Record<string, string>> = {
	folderUnavailable: "The selected folder is unavailable.",
	folderNotDirectory: "Choose a folder, not a file.",
	folderOverlapsSource: "That folder overlaps an existing source.",
	folderOverlapsLocalState: "That folder overlaps Photo Viewer's local data.",
	localStateUnavailable: "Photo Viewer cannot open its local data.",
	internal: internalErrorMessage,
};

const internalError = () =>
	new PhotoServiceError("internal", internalErrorMessage);

export type InvokeCommand = <T>(
	command: string,
	args?: Record<string, unknown>,
) => Promise<T>;

export function createTauriPhotoService(
	invokeCommand: InvokeCommand = invoke,
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
