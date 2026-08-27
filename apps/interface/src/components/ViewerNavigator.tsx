import {
	type MouseEvent,
	type PointerEvent,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import styles from "../styles/photoViewer.module.css";
import type { ViewerPoint } from "../viewer/viewerTransform";

type CandidateDecodeStatus = "idle" | "pending" | "ready" | "failed";

export interface ViewerNavigatorProps {
	assetName: string;
	imageUrl: string | null;
	fallbackUrl?: string | null;
	imageWidth: number;
	imageHeight: number;
	visibleRect: { x: number; y: number; width: number; height: number };
	visible: boolean;
	interactive: boolean;
	onRecenter: (focal: ViewerPoint) => void;
	onInteraction: () => void;
	onManipulationChange?: (active: boolean) => void;
}

export interface NavigatorBounds {
	left: number;
	top: number;
	width: number;
	height: number;
}

export function navigatorPointToFocal(
	point: ViewerPoint,
	bounds: NavigatorBounds,
): ViewerPoint {
	const x =
		Number.isFinite(point?.x) && Number.isFinite(bounds.width)
			? (point.x - bounds.left) / bounds.width
			: 0.5;
	const y =
		Number.isFinite(point?.y) && Number.isFinite(bounds.height)
			? (point.y - bounds.top) / bounds.height
			: 0.5;
	return {
		x: Math.min(1, Math.max(0, Number.isFinite(x) ? x : 0.5)),
		y: Math.min(1, Math.max(0, Number.isFinite(y) ? y : 0.5)),
	};
}

export function navigatorImageBounds(
	containerWidth: number,
	containerHeight: number,
	imageWidth: number,
	imageHeight: number,
): NavigatorBounds {
	if (
		!Number.isFinite(containerWidth) ||
		!Number.isFinite(containerHeight) ||
		!Number.isFinite(imageWidth) ||
		!Number.isFinite(imageHeight) ||
		containerWidth <= 0 ||
		containerHeight <= 0 ||
		imageWidth <= 0 ||
		imageHeight <= 0
	)
		return { left: 0, top: 0, width: 0, height: 0 };
	const scale = Math.min(
		containerWidth / imageWidth,
		containerHeight / imageHeight,
	);
	const width = imageWidth * scale;
	const height = imageHeight * scale;
	return {
		left: (containerWidth - width) / 2,
		top: (containerHeight - height) / 2,
		width,
		height,
	};
}

export function navigatorContentBounds(
	outerWidth: number,
	outerHeight: number,
	borderLeft = 1,
	borderRight = 1,
	borderTop = 1,
	borderBottom = 1,
): NavigatorBounds {
	const width = Math.max(0, outerWidth - borderLeft - borderRight);
	const height = Math.max(0, outerHeight - borderTop - borderBottom);
	return { left: borderLeft, top: borderTop, width, height };
}

export function navigatorViewportStyle(rect: {
	x: number;
	y: number;
	width: number;
	height: number;
}): Record<string, string> {
	const clamp = (value: number) =>
		Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
	return {
		left: `${clamp(rect.x) * 100}%`,
		top: `${clamp(rect.y) * 100}%`,
		width: `${clamp(rect.width) * 100}%`,
		height: `${clamp(rect.height) * 100}%`,
	};
}

function focalForPointer(
	event: PointerEvent<HTMLElement> | MouseEvent<HTMLElement>,
	imageBounds: HTMLElement,
): ViewerPoint {
	return navigatorPointToFocal(
		{ x: event.clientX, y: event.clientY },
		imageBounds.getBoundingClientRect(),
	);
}

export function ViewerNavigator({
	assetName,
	imageUrl,
	fallbackUrl = null,
	imageWidth,
	imageHeight,
	visibleRect,
	visible,
	interactive,
	onRecenter,
	onInteraction,
	onManipulationChange,
}: ViewerNavigatorProps) {
	const navigatorRef = useRef<HTMLDivElement>(null);
	const imageRef = useRef<HTMLDivElement>(null);
	const requestedUrlRef = useRef<string | null>(null);
	const pointerIdRef = useRef<number | null>(null);
	const movedRef = useRef(false);
	const [displayUrl, setDisplayUrl] = useState<string | null>(
		fallbackUrl ?? imageUrl,
	);
	const [candidateDecodeStatus, setCandidateDecodeStatus] =
		useState<CandidateDecodeStatus>(imageUrl ? "pending" : "idle");
	const [navigatorSize, setNavigatorSize] = useState({
		width: 198,
		height: 118,
	});

	useLayoutEffect(() => {
		const node = navigatorRef.current;
		if (!node) return;
		const measure = () => {
			const bounds = node.getBoundingClientRect();
			const computed = window.getComputedStyle(node);
			const content = navigatorContentBounds(
				bounds.width,
				bounds.height,
				Number.parseFloat(computed.borderLeftWidth) || 0,
				Number.parseFloat(computed.borderRightWidth) || 0,
				Number.parseFloat(computed.borderTopWidth) || 0,
				Number.parseFloat(computed.borderBottomWidth) || 0,
			);
			const next = { width: content.width, height: content.height };
			if (next.width <= 0 || next.height <= 0) return;
			setNavigatorSize((previous) =>
				previous.width === next.width && previous.height === next.height
					? previous
					: next,
			);
		};
		measure();
		if (typeof ResizeObserver === "undefined") {
			window.addEventListener("resize", measure);
			return () => window.removeEventListener("resize", measure);
		}
		const observer = new ResizeObserver(measure);
		observer.observe(node);
		return () => observer.disconnect();
	}, []);

	useEffect(() => {
		if (!imageUrl) {
			requestedUrlRef.current = null;
			setDisplayUrl(fallbackUrl);
			setCandidateDecodeStatus("idle");
			return;
		}
		if (requestedUrlRef.current === imageUrl) return;
		requestedUrlRef.current = imageUrl;
		setCandidateDecodeStatus("pending");
		let active = true;
		const image = new Image();
		image.src = imageUrl;
		const ready = image.decode ? image.decode() : Promise.resolve();
		void ready.then(
			() => {
				if (!active) return;
				setDisplayUrl(imageUrl);
				setCandidateDecodeStatus("ready");
			},
			() => {
				if (!active) return;
				setDisplayUrl(fallbackUrl);
				setCandidateDecodeStatus("failed");
			},
		);
		return () => {
			active = false;
		};
	}, [fallbackUrl, imageUrl]);

	const imageBounds = navigatorImageBounds(
		navigatorSize.width,
		navigatorSize.height,
		imageWidth,
		imageHeight,
	);
	const viewportStyle = navigatorViewportStyle(visibleRect);
	const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
		if (!interactive || event.pointerType === "touch" || event.button !== 0)
			return;
		const target = imageRef.current;
		if (!target) return;
		event.preventDefault();
		event.stopPropagation();
		pointerIdRef.current = event.pointerId;
		movedRef.current = false;
		onManipulationChange?.(true);
		try {
			target.setPointerCapture?.(event.pointerId);
		} catch {
			// Synthetic or cancelled pointers may not be capturable.
		}
		onRecenter(focalForPointer(event, target));
		onInteraction();
	};
	const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
		if (
			pointerIdRef.current !== event.pointerId ||
			event.pointerType === "touch"
		)
			return;
		const target = imageRef.current;
		if (!target) return;
		movedRef.current = true;
		event.preventDefault();
		event.stopPropagation();
		onRecenter(focalForPointer(event, target));
		onInteraction();
	};
	const endPointer = (event: PointerEvent<HTMLDivElement>) => {
		if (pointerIdRef.current !== event.pointerId) return;
		event.stopPropagation();
		pointerIdRef.current = null;
		onManipulationChange?.(false);
		try {
			imageRef.current?.releasePointerCapture?.(event.pointerId);
		} catch {
			// The pointer may already have been released by the browser.
		}
	};
	const handleClick = (event: MouseEvent<HTMLDivElement>) => {
		if (!interactive || event.detail === 0 || movedRef.current) {
			movedRef.current = false;
			return;
		}
		const target = imageRef.current;
		if (!target) return;
		event.stopPropagation();
		onRecenter(focalForPointer(event, target));
		onInteraction();
	};

	return (
		<div
			aria-hidden={!interactive || !visible}
			aria-label={`Navigator for ${assetName}`}
			className={`${styles.viewerNavigator} ${!visible ? styles.viewerNavigatorHidden : ""}`}
			data-viewer-navigator-decode-status={candidateDecodeStatus}
			data-viewer-navigator="true"
			onClick={handleClick}
			onPointerCancel={endPointer}
			onPointerDown={handlePointerDown}
			onPointerMove={handlePointerMove}
			onPointerUp={endPointer}
			onLostPointerCapture={endPointer}
			role="img"
			ref={navigatorRef}
			tabIndex={-1}
		>
			<div
				className={styles.viewerNavigatorImage}
				ref={imageRef}
				style={{
					left: `${imageBounds.left}px`,
					top: `${imageBounds.top}px`,
					width: `${imageBounds.width}px`,
					height: `${imageBounds.height}px`,
				}}
			>
				{displayUrl ? (
					<img
						alt=""
						className={styles.viewerNavigatorImageContent}
						draggable={false}
						data-viewer-navigator-layer={
							displayUrl === imageUrl ? "screenPreview" : "wallThumbnail"
						}
						src={displayUrl}
					/>
				) : null}
				<div
					aria-hidden="true"
					className={styles.viewerNavigatorViewport}
					data-viewer-navigator-viewport="true"
					style={viewportStyle}
				/>
			</div>
		</div>
	);
}
