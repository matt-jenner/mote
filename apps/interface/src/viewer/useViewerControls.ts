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
	resume: () => void;
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
	const lastInput = useRef<ViewerControlInput>(
		mediaMatches("(pointer: coarse)") ? "touch" : "mouse",
	);
	const controlsVisibleRef = useRef(controlsVisible);
	const onHideRef = useRef(onHide);
	const onShowRef = useRef(onShow);
	const onToggleTouchRef = useRef(onToggleTouch);
	controlsVisibleRef.current = controlsVisible;
	onHideRef.current = onHide;
	onShowRef.current = onShow;
	onToggleTouchRef.current = onToggleTouch;
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
	const scheduleHide = useCallback(
		(input: ViewerControlInput) => {
			clearHideTimer();
			lastInput.current = input;
			hideTimer.current = window.setTimeout(() => {
				hideTimer.current = null;
				onHideRef.current();
			}, controlHideDelay(input));
		},
		[clearHideTimer],
	);

	const showForInput = useCallback(
		(input: ViewerControlInput) => {
			if (!controlsVisibleRef.current) onShowRef.current();
			scheduleHide(input);
		},
		[scheduleHide],
	);

	const keepVisible = useCallback(() => {
		clearHideTimer();
		if (!controlsVisibleRef.current) onShowRef.current();
	}, [clearHideTimer]);

	const resume = useCallback(() => {
		scheduleHide(lastInput.current);
	}, [scheduleHide]);

	const toggleTouch = useCallback(() => {
		onToggleTouchRef.current();
		scheduleHide("touch");
	}, [scheduleHide]);

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

	useEffect(() => {
		scheduleHide(lastInput.current);
		return clearHideTimer;
	}, [clearHideTimer, scheduleHide]);

	return {
		controlsVisible,
		showForInput,
		toggleTouch,
		keepVisible,
		resume,
		coarsePointer,
		prefersReducedMotion,
	};
}
