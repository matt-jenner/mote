import type { Appearance, BootstrapState, PhotoService } from "./photoService";

interface InMemoryOptions {
	selectedFolderName?: string;
	cancelFolderPicker?: boolean;
}

export function createInMemoryPhotoService(
	options: InMemoryOptions = {},
): PhotoService {
	let state: BootstrapState = {
		settings: { appearance: "system" },
		activeSource: null,
	};
	const copy = (): BootstrapState => structuredClone(state);

	return {
		capabilities: { chooseFolder: true, locateFolder: false },
		async getBootstrapState() {
			return copy();
		},
		async chooseFolder() {
			if (options.cancelFolderPicker) return { kind: "cancelled" };
			state = {
				...state,
				activeSource: {
					id: "memory-source",
					displayName: options.selectedFolderName ?? "Selected Folder",
					availability: "available",
				},
			};
			return { kind: "selected", state: copy() };
		},
		async updateAppearance(appearance: Appearance) {
			state = { ...state, settings: { appearance } };
			return copy();
		},
	};
}
