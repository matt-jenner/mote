import { invoke } from "@tauri-apps/api/core";
import type {
	Appearance,
	BootstrapState,
	ChooseFolderResult,
	PhotoService,
} from "./photoService";

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
			invokeCommand<BootstrapState>("get_bootstrap_state", undefined),
		chooseFolder: () =>
			invokeCommand<ChooseFolderResult>("choose_folder", undefined),
		updateAppearance: (appearance: Appearance) =>
			invokeCommand<BootstrapState>("update_appearance", { appearance }),
	};
}
