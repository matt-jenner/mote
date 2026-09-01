import { useCallback, useEffect, useRef, useState } from "react";

export interface ViewerViewportSize {
	width: number;
	height: number;
}

export interface ViewerViewportState extends ViewerViewportSize {
	revision: number;
}

function usableSize(
	size: ViewerViewportSize | null | undefined,
): ViewerViewportSize | null {
	if (!size || !Number.isFinite(size.width) || !Number.isFinite(size.height))
		return null;
	if (size.width <= 0 || size.height <= 0) return null;
	return { width: size.width, height: size.height };
}

export function chooseViewerViewport(
	visualViewport: ViewerViewportSize | null,
	documentViewport: ViewerViewportSize,
): ViewerViewportSize {
	return (
		usableSize(visualViewport) ??
		usableSize(documentViewport) ?? { width: 0, height: 0 }
	);
}

function readViewport(): ViewerViewportSize {
	if (typeof window === "undefined" || typeof document === "undefined") {
		return { width: 0, height: 0 };
	}
	const visual = window.visualViewport;
	return chooseViewerViewport(
		visual ? { width: visual.width, height: visual.height } : null,
		{
			width: document.documentElement.clientWidth,
			height: document.documentElement.clientHeight,
		},
	);
}

export function useViewerViewport(): ViewerViewportState {
	const [state, setState] = useState<ViewerViewportState>(() => ({
		...readViewport(),
		revision: 0,
	}));
	const frame = useRef<number | null>(null);
	const orientationFollowUp = useRef<number | null>(null);
	const latest = useRef(state);
	latest.current = state;

	const measure = useCallback(() => {
		frame.current = null;
		const next = readViewport();
		if (
			next.width === latest.current.width &&
			next.height === latest.current.height
		)
			return;
		setState((previous) => ({
			...next,
			revision: previous.revision + 1,
		}));
	}, []);

	const schedule = useCallback(() => {
		if (frame.current !== null) return;
		if (typeof window.requestAnimationFrame === "function") {
			frame.current = window.requestAnimationFrame(measure);
		} else {
			frame.current = window.setTimeout(measure, 0);
		}
	}, [measure]);

	const scheduleOrientation = useCallback(() => {
		schedule();
		if (orientationFollowUp.current !== null)
			window.clearTimeout(orientationFollowUp.current);
		orientationFollowUp.current = window.setTimeout(() => {
			orientationFollowUp.current = null;
			schedule();
		}, 150);
	}, [schedule]);

	useEffect(() => {
		const visual = window.visualViewport;
		visual?.addEventListener("resize", schedule);
		visual?.addEventListener("scroll", schedule);
		window.addEventListener("resize", schedule);
		window.addEventListener("orientationchange", scheduleOrientation);
		return () => {
			visual?.removeEventListener("resize", schedule);
			visual?.removeEventListener("scroll", schedule);
			window.removeEventListener("resize", schedule);
			window.removeEventListener("orientationchange", scheduleOrientation);
			if (frame.current !== null) {
				window.cancelAnimationFrame?.(frame.current);
				window.clearTimeout(frame.current);
				frame.current = null;
			}
			if (orientationFollowUp.current !== null) {
				window.clearTimeout(orientationFollowUp.current);
				orientationFollowUp.current = null;
			}
		};
	}, [schedule, scheduleOrientation]);

	return state;
}
