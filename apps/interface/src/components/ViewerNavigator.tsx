import {
	type MouseEvent,
	type PointerEvent,
	useEffect,
	useRef,
	useState,
} from "react";
import styles from "../styles/photoViewer.module.css";
import type { ViewerPoint } from "../viewer/viewerTransform";

export interface ViewerNavigatorProps {
	assetName: string;
	imageUrl: string | null;
	imageWidth: number;
	imageHeight: number;
	visibleRect: { x: number; y: number; width: number; height: number };
	visible: boolean;
	interactive: boolean;
	onRecenter: (focal: ViewerPoint) => void;
	onInteraction: () => void;
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
	imageWidth,
	imageHeight,
	visibleRect,
	visible,
	interactive,
	onRecenter,
	onInteraction,
}: ViewerNavigatorProps) {
	const imageRef = useRef<HTMLDivElement>(null);
	const pointerIdRef = useRef<number | null>(null);
	const movedRef = useRef(false);
	const [displayUrl, setDisplayUrl] = useState<string | null>(imageUrl);

	useEffect(() => {
		if (!imageUrl) {
			setDisplayUrl(null);
			return;
		}
		if (displayUrl === imageUrl) return;
		let active = true;
		const image = new Image();
		image.src = imageUrl;
		const ready = image.decode ? image.decode() : Promise.resolve();
		void ready.then(
			() => {
				if (active) setDisplayUrl(imageUrl);
			},
			() => undefined,
		);
		return () => {
			active = false;
		};
	}, [displayUrl, imageUrl]);

	if (!visible) return null;
	const imageBounds = navigatorImageBounds(200, 120, imageWidth, imageHeight);
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
			aria-hidden={!interactive}
			aria-label={`Navigator for ${assetName}`}
			className={styles.viewerNavigator}
			data-viewer-navigator="true"
			onClick={handleClick}
			onPointerCancel={endPointer}
			onPointerDown={handlePointerDown}
			onPointerMove={handlePointerMove}
			onPointerUp={endPointer}
			role="img"
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
