import { useCallback, useEffect, useRef, useState } from "react";

export type ViewerControlInput = "mouse" | "touch";

export function controlHideDelay(input: ViewerControlInput): number {
	return input === "touch" ? 3500 : 2500;
}

interface ViewerControlsOptions {
	controlsVisible: boolean;
	onShow: () => void;
	onHide: () => void;
	onToggleTouch: () => void;
}

interface ViewerControls {
	controlsVisible: boolean;
	showForInput: (input: ViewerControlInput) => void;
	toggleTouch: () => void;
	keepVisible: () => void;
	coarsePointer: boolean;
	prefersReducedMotion: boolean;
}

function mediaMatches(query: string): boolean {
	return typeof window !== "undefined" && window.matchMedia(query).matches;
}

export function useViewerControls({
	controlsVisible,
	onShow,
	onHide,
	onToggleTouch,
}: ViewerControlsOptions): ViewerControls {
	const hideTimer = useRef<number | null>(null);
	const [coarsePointer, setCoarsePointer] = useState(() =>
		mediaMatches("(pointer: coarse)"),
	);
	const [prefersReducedMotion, setPrefersReducedMotion] = useState(() =>
		mediaMatches("(prefers-reduced-motion: reduce)"),
	);

	const clearHideTimer = useCallback(() => {
		if (hideTimer.current !== null) {
			window.clearTimeout(hideTimer.current);
			hideTimer.current = null;
		}
	}, []);

	const showForInput = useCallback(
		(input: ViewerControlInput) => {
			clearHideTimer();
			if (!controlsVisible) onShow();
			hideTimer.current = window.setTimeout(() => {
				hideTimer.current = null;
				onHide();
			}, controlHideDelay(input));
		},
		[clearHideTimer, controlsVisible, onHide, onShow],
	);

	const keepVisible = useCallback(() => {
		clearHideTimer();
		if (!controlsVisible) onShow();
	}, [clearHideTimer, controlsVisible, onShow]);

	const toggleTouch = useCallback(() => {
		clearHideTimer();
		onToggleTouch();
		hideTimer.current = window.setTimeout(() => {
			hideTimer.current = null;
			onHide();
		}, controlHideDelay("touch"));
	}, [clearHideTimer, onHide, onToggleTouch]);

	useEffect(() => {
		const pointerQuery = window.matchMedia("(pointer: coarse)");
		const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
		const updatePointer = (event: MediaQueryListEvent) =>
			setCoarsePointer(event.matches);
		const updateMotion = (event: MediaQueryListEvent) =>
			setPrefersReducedMotion(event.matches);
		pointerQuery.addEventListener("change", updatePointer);
		motionQuery.addEventListener("change", updateMotion);
		return () => {
			pointerQuery.removeEventListener("change", updatePointer);
			motionQuery.removeEventListener("change", updateMotion);
		};
	}, []);

	useEffect(() => clearHideTimer, [clearHideTimer]);

	return {
		controlsVisible,
		showForInput,
		toggleTouch,
		keepVisible,
		coarsePointer,
		prefersReducedMotion,
	};
}
