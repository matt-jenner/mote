import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import type {
	DerivativeReference,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import styles from "../styles/photoViewer.module.css";
import type {
	ViewerDrawableSize,
	ViewerNaturalSize,
} from "../viewer/useViewerTransform";
import {
	fitViewerFrame,
	type ViewerPoint,
	type ViewerTransformGeometry,
} from "../viewer/viewerTransform";

export { fitViewerFrame } from "../viewer/viewerTransform";

interface ViewerStageProps {
	asset: WallAsset;
	service: PhotoService;
	baseUrl?: string | null;
	currentUrl?: string | null;
	largePreviewUnavailable?: boolean;
	onPreviewFailure?: (failureKey: string) => void;
	previewGeneration?: number;
	viewportWidth?: number;
	viewportHeight?: number;
	transform?: ViewerTransformGeometry;
	onDrawableSizeChange?: (size: ViewerDrawableSize) => void;
	onNaturalSizeChange?: (size: ViewerNaturalSize) => void;
	onWheel?: (event: WheelEvent, point: ViewerPoint) => boolean;
	onDoubleClick?: (point: ViewerPoint) => void;
	panning?: boolean;
}

export interface ViewerFrameRect {
	width: number;
	height: number;
}

export interface ViewerInsets {
	top: number;
	right: number;
	bottom: number;
	left: number;
}

export function drawableViewerBox(
	containerWidth: number,
	containerHeight: number,
	insets: ViewerInsets,
): ViewerFrameRect {
	return {
		width: Math.max(0, containerWidth - insets.left - insets.right),
		height: Math.max(0, containerHeight - insets.top - insets.bottom),
	};
}

function safeDerivativeUrl(
	service: PhotoService,
	reference: DerivativeReference | null,
): string | null {
	if (!reference) return null;
	try {
		return service.derivativeUrl(reference);
	} catch {
		return null;
	}
}

export function ViewerStage({
	asset,
	service,
	baseUrl: suppliedBaseUrl,
	currentUrl: suppliedCurrentUrl,
	largePreviewUnavailable = false,
	onPreviewFailure,
	previewGeneration = 0,
	viewportWidth = 0,
	viewportHeight = 0,
	transform,
	onDrawableSizeChange,
	onNaturalSizeChange,
	onWheel,
	onDoubleClick,
	panning = false,
}: ViewerStageProps) {
	const measureRef = useRef<HTMLDivElement>(null);
	const stageRef = useRef<HTMLDivElement>(null);
	const screenImageRef = useRef<HTMLImageElement>(null);
	const activeDecodeRef = useRef("");
	const baseNaturalSizeRef = useRef<{
		token: string;
		size: ViewerNaturalSize;
	} | null>(null);
	const [stageSize, setStageSize] = useState({ width: 0, height: 0 });
	const [decodedToken, setDecodedToken] = useState<string | null>(null);
	const [failedScreenToken, setFailedScreenToken] = useState<string | null>(
		null,
	);
	const [failedBaseUrl, setFailedBaseUrl] = useState<string | null>(null);
	const baseUrl =
		suppliedBaseUrl === undefined
			? safeDerivativeUrl(service, asset.wallThumbnail)
			: suppliedBaseUrl;
	const currentUrl =
		suppliedCurrentUrl === undefined
			? safeDerivativeUrl(service, asset.screenPreview)
			: suppliedCurrentUrl;
	const decodeToken = `${asset.id}:${previewGeneration}:${currentUrl ?? ""}`;
	const baseToken = `${asset.id}:${previewGeneration}:${baseUrl ?? ""}`;

	useEffect(() => {
		if (baseNaturalSizeRef.current?.token !== baseToken)
			baseNaturalSizeRef.current = null;
	}, [baseToken]);

	useLayoutEffect(() => {
		const measureNode = measureRef.current;
		if (!measureNode) return;
		const measure = () => {
			const next = {
				width:
					viewportWidth > 0
						? Math.min(measureNode.clientWidth, viewportWidth)
						: measureNode.clientWidth,
				height:
					viewportHeight > 0
						? Math.min(measureNode.clientHeight, viewportHeight)
						: measureNode.clientHeight,
			};
			setStageSize((previous) =>
				previous.width === next.width && previous.height === next.height
					? previous
					: next,
			);
		};
		measure();
		if (typeof ResizeObserver !== "undefined") {
			const observer = new ResizeObserver(measure);
			observer.observe(measureNode);
			return () => observer.disconnect();
		}
		window.addEventListener("resize", measure);
		return () => window.removeEventListener("resize", measure);
	}, [viewportHeight, viewportWidth]);

	useEffect(() => {
		onDrawableSizeChange?.(stageSize);
	}, [onDrawableSizeChange, stageSize]);

	useEffect(() => {
		const stage = stageRef.current;
		if (!stage || !onWheel) return;
		const handleWheel = (event: WheelEvent) => {
			const drawable = measureRef.current?.getBoundingClientRect();
			if (!drawable) return;
			const consumed = onWheel(event, {
				x: event.clientX - drawable.left,
				y: event.clientY - drawable.top,
			});
			if (consumed && event.cancelable) event.preventDefault();
		};
		stage.addEventListener("wheel", handleWheel, { passive: false });
		return () => stage.removeEventListener("wheel", handleWheel);
	}, [onWheel]);

	useEffect(() => {
		const stage = stageRef.current;
		if (!stage || !onDoubleClick) return;
		const handleDoubleClick = (event: MouseEvent) => {
			const drawable = measureRef.current?.getBoundingClientRect();
			if (!drawable) return;
			onDoubleClick({
				x: event.clientX - drawable.left,
				y: event.clientY - drawable.top,
			});
		};
		stage.addEventListener("dblclick", handleDoubleClick);
		return () => stage.removeEventListener("dblclick", handleDoubleClick);
	}, [onDoubleClick]);

	const naturalSizeFor = useCallback(
		(image: HTMLImageElement): ViewerNaturalSize | null =>
			image.naturalWidth > 0 && image.naturalHeight > 0
				? { width: image.naturalWidth, height: image.naturalHeight }
				: null,
		[],
	);
	const reportBaseNaturalSize = useCallback(
		(image: HTMLImageElement) => {
			const size = naturalSizeFor(image);
			if (!size) return;
			baseNaturalSizeRef.current = { token: baseToken, size };
			if (decodedToken !== decodeToken || failedScreenToken === decodeToken)
				onNaturalSizeChange?.(size);
		},
		[
			baseToken,
			decodeToken,
			decodedToken,
			failedScreenToken,
			naturalSizeFor,
			onNaturalSizeChange,
		],
	);
	const restoreBaseNaturalSize = useCallback(() => {
		const base = baseNaturalSizeRef.current;
		if (base?.token === baseToken) onNaturalSizeChange?.(base.size);
	}, [baseToken, onNaturalSizeChange]);
	const reportDecodedPreviewNaturalSize = useCallback(
		(image: HTMLImageElement) => {
			const size = naturalSizeFor(image);
			if (size) onNaturalSizeChange?.(size);
		},
		[naturalSizeFor, onNaturalSizeChange],
	);

	useEffect(() => {
		activeDecodeRef.current = decodeToken;
		setDecodedToken(null);
		if (!currentUrl) return;
		const image = screenImageRef.current;
		if (!image) return;
		let decodeResult: Promise<void>;
		try {
			decodeResult = image.decode ? image.decode() : Promise.resolve();
		} catch {
			decodeResult = Promise.reject(new Error("preview decode failed"));
		}
		void decodeResult.then(
			() => {
				if (activeDecodeRef.current !== decodeToken) return;
				setDecodedToken(decodeToken);
				reportDecodedPreviewNaturalSize(image);
			},
			() => {
				if (activeDecodeRef.current !== decodeToken) return;
				setFailedScreenToken(decodeToken);
				restoreBaseNaturalSize();
				onPreviewFailure?.(decodeToken);
			},
		);
	}, [
		currentUrl,
		decodeToken,
		onPreviewFailure,
		reportDecodedPreviewNaturalSize,
		restoreBaseNaturalSize,
	]);

	const frame = fitViewerFrame(
		stageSize.width,
		stageSize.height,
		asset.width,
		asset.height,
	);
	const frameStyle =
		frame.width > 0
			? { width: `${frame.width}px`, height: `${frame.height}px` }
			: undefined;
	const screenFailed = Boolean(currentUrl) && failedScreenToken === decodeToken;
	const hasBaseImage = Boolean(baseUrl) && failedBaseUrl !== baseUrl;
	const showPreview =
		Boolean(currentUrl) && decodedToken === decodeToken && !screenFailed;
	const appliedTransform = transform ?? {
		mode: "fit" as const,
		fitWidth: frame.width,
		fitHeight: frame.height,
		scale: 1,
		maxScale: 1,
		translateX: 0,
		translateY: 0,
		focal: { x: 0.5, y: 0.5 },
		visibleImageRect: { x: 0, y: 0, width: 1, height: 1 },
	};

	return (
		<div
			className={styles.viewerStage}
			data-current-asset={asset.id}
			data-viewer-mode={appliedTransform.mode}
			data-large-preview-unavailable={
				largePreviewUnavailable || screenFailed ? "true" : "false"
			}
			data-testid="viewer-stage"
			data-viewer-panning={panning ? "true" : "false"}
			ref={stageRef}
		>
			<div className={styles.viewerStageMeasure} ref={measureRef}>
				<div
					className={styles.viewerFrame}
					data-testid="viewer-frame"
					style={frameStyle}
				>
					<div
						className={styles.viewerTransformLayer}
						data-testid="viewer-transform-layer"
						data-viewer-mode={appliedTransform.mode}
						style={{
							transform: `translate3d(${appliedTransform.translateX}px, ${appliedTransform.translateY}px, 0) scale(${appliedTransform.scale})`,
							transformOrigin: "center",
							willChange:
								appliedTransform.mode === "zoomed" ? "transform" : "auto",
						}}
					>
						{hasBaseImage ? (
							<img
								alt={asset.displayName}
								className={`${styles.viewerImage} ${styles.viewerBase}`}
								data-viewer-layer="wallThumbnail"
								decoding="async"
								draggable={false}
								onLoad={(event) => reportBaseNaturalSize(event.currentTarget)}
								onError={() => setFailedBaseUrl(baseUrl)}
								src={baseUrl ?? undefined}
							/>
						) : (
							<div
								aria-label={`${asset.displayName} representative colour`}
								className={`${styles.viewerColour} ${styles.viewerBase}`}
								data-viewer-layer="representativeColour"
								role="img"
								style={{
									backgroundColor:
										asset.representativeRgb === null
											? "var(--canvas-elevated)"
											: `rgb(${(asset.representativeRgb >> 16) & 255}, ${(asset.representativeRgb >> 8) & 255}, ${asset.representativeRgb & 255})`,
								}}
							/>
						)}
						{currentUrl ? (
							<img
								alt=""
								className={`${styles.viewerImage} ${styles.viewerPreview}`}
								data-ready={showPreview ? "true" : "false"}
								data-viewer-layer="screenPreview"
								decoding="async"
								draggable={false}
								onLoad={(event) => {
									if (decodedToken === decodeToken && !screenFailed)
										reportDecodedPreviewNaturalSize(event.currentTarget);
								}}
								onError={() => {
									if (activeDecodeRef.current === decodeToken) {
										setFailedScreenToken(decodeToken);
										restoreBaseNaturalSize();
										onPreviewFailure?.(decodeToken);
									}
								}}
								ref={screenImageRef}
								src={currentUrl}
							/>
						) : null}
					</div>
				</div>
			</div>
		</div>
	);
}
