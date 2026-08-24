export type Appearance = "system" | "light" | "dark";
export type SourceAvailability =
	| "available"
	| "rootOffline"
	| "missing"
	| "unreadable";

export interface SettingsState {
	appearance: Appearance;
}

export interface SourceSummary {
	id: string;
	displayName: string;
	availability: SourceAvailability;
}

export interface BootstrapState {
	settings: SettingsState;
	activeSource: SourceSummary | null;
}

export type ChooseFolderResult =
	| { kind: "cancelled" }
	| { kind: "selected"; state: BootstrapState };

export interface PhotoServiceCapabilities {
	chooseFolder: boolean;
	locateFolder: boolean;
}

export interface PhotoService {
	readonly capabilities: PhotoServiceCapabilities;
	getBootstrapState(): Promise<BootstrapState>;
	chooseFolder(): Promise<ChooseFolderResult>;
	updateAppearance(appearance: Appearance): Promise<BootstrapState>;
}

export class PhotoServiceError extends Error {
	constructor(
		readonly code: string,
		message: string,
	) {
		super(message);
		this.name = "PhotoServiceError";
	}
}
