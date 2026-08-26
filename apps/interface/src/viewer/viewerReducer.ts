export interface ViewerReturnAnchor {
	assetId: string;
	scrollTop: number;
}

export interface ViewerState {
	open: boolean;
	currentAssetId: string | null;
	returnAnchor: ViewerReturnAnchor | null;
	infoOpen: boolean;
	controlsVisible: boolean;
	filmstripVisible: boolean;
	previewGeneration: number;
}

export type ViewerAction =
	| {
			type: "open";
			assetId: string;
			anchor: ViewerReturnAnchor;
	  }
	| { type: "close" }
	| { type: "select"; assetId: string }
	| { type: "setInfoOpen"; open: boolean }
	| { type: "showControls" }
	| { type: "hideControls" }
	| { type: "toggleTouchControls" };

export const initialViewerState: ViewerState = {
	open: false,
	currentAssetId: null,
	returnAnchor: null,
	infoOpen: false,
	controlsVisible: true,
	filmstripVisible: true,
	previewGeneration: 0,
};

export function viewerReducer(
	state: ViewerState,
	action: ViewerAction,
): ViewerState {
	switch (action.type) {
		case "open":
			return {
				...state,
				open: true,
				currentAssetId: action.assetId,
				returnAnchor: action.anchor,
				previewGeneration: state.previewGeneration + 1,
			};
		case "close":
			return {
				...initialViewerState,
				previewGeneration: state.previewGeneration + 1,
			};
		case "select":
			return {
				...state,
				currentAssetId: action.assetId,
				previewGeneration: state.previewGeneration + 1,
			};
		case "setInfoOpen":
			return { ...state, infoOpen: action.open };
		case "showControls":
			return { ...state, controlsVisible: true };
		case "hideControls":
			return { ...state, controlsVisible: false };
		case "toggleTouchControls": {
			const controlsVisible = !state.controlsVisible;
			return {
				...state,
				controlsVisible,
				filmstripVisible: controlsVisible,
			};
		}
	}
}
