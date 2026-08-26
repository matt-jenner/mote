import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type {
	DerivativeReference,
	PhotoService,
	WallAsset,
} from "../services/photoService";
import styles from "../styles/photoViewer.module.css";

interface ViewerStageProps {
	asset: WallAsset;
	service: PhotoService;
	baseUrl?: string | null;
	currentUrl?: string | null;
	largePreviewUnavailable?: boolean;
	onPreviewFailure?: () => void;
	previewGeneration?: number;
	viewportWidth?: number;
	viewportHeight?: number;
}

export interface ViewerFrameRect {
	width: number;
	height: number;
}

export function fitViewerFrame(
	containerWidth: number,
	containerHeight: number,
	assetWidth: number,
	assetHeight: number,
): ViewerFrameRect {
	if (
		containerWidth <= 0 ||
		containerHeight <= 0 ||
		assetWidth <= 0 ||
		assetHeight <= 0
	)
		return { width: 0, height: 0 };
	const scale = Math.min(
		containerWidth / assetWidth,
		containerHeight / assetHeight,
	);
	return {
		width: assetWidth * scale,
		height: assetHeight * scale,
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
}: ViewerStageProps) {
	const stageRef = useRef<HTMLDivElement>(null);
	const screenImageRef = useRef<HTMLImageElement>(null);
	const activeDecodeRef = useRef("");
	const [stageSize, setStageSize] = useState({ width: 0, height: 0 });
	const [decodedToken, setDecodedToken] = useState<string | null>(null);
	const [screenFailed, setScreenFailed] = useState(false);
	const [baseFailed, setBaseFailed] = useState(false);
	const baseUrl =
		suppliedBaseUrl === undefined
			? safeDerivativeUrl(service, asset.wallThumbnail)
			: suppliedBaseUrl;
	const currentUrl =
		suppliedCurrentUrl === undefined
			? safeDerivativeUrl(service, asset.screenPreview)
			: suppliedCurrentUrl;
	const decodeToken = `${asset.id}:${previewGeneration}:${currentUrl ?? ""}`;

	useLayoutEffect(() => {
		const stage = stageRef.current;
		if (!stage) return;
		const measure = () =>
			setStageSize({
				width:
					viewportWidth > 0
						? Math.min(stage.clientWidth, viewportWidth)
						: stage.clientWidth,
				height:
					viewportHeight > 0
						? Math.min(stage.clientHeight, viewportHeight)
						: stage.clientHeight,
			});
		measure();
		if (typeof ResizeObserver !== "undefined") {
			const observer = new ResizeObserver(measure);
			observer.observe(stage);
			return () => observer.disconnect();
		}
		window.addEventListener("resize", measure);
		return () => window.removeEventListener("resize", measure);
	}, [viewportHeight, viewportWidth]);

	useEffect(() => {
		activeDecodeRef.current = decodeToken;
		setDecodedToken(null);
		setScreenFailed(false);
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
			},
			() => {
				if (activeDecodeRef.current !== decodeToken) return;
				setScreenFailed(true);
				onPreviewFailure?.();
			},
		);
	}, [currentUrl, decodeToken, onPreviewFailure]);

	// The URL is part of the base-layer identity; reset its error state when it changes.
	// biome-ignore lint/correctness/useExhaustiveDependencies: baseUrl is the identity being watched.
	useEffect(() => {
		setBaseFailed(false);
	}, [baseUrl]);

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
	const hasBaseImage = Boolean(baseUrl) && !baseFailed;
	const showPreview =
		Boolean(currentUrl) && decodedToken === decodeToken && !screenFailed;

	return (
		<div
			className={styles.viewerStage}
			data-current-asset={asset.id}
			data-large-preview-unavailable={
				largePreviewUnavailable || screenFailed ? "true" : "false"
			}
			data-testid="viewer-stage"
			ref={stageRef}
		>
			<div
				className={styles.viewerFrame}
				data-testid="viewer-frame"
				style={frameStyle}
			>
				{hasBaseImage ? (
					<img
						alt={asset.displayName}
						className={`${styles.viewerImage} ${styles.viewerBase}`}
						data-viewer-layer="wallThumbnail"
						decoding="async"
						draggable={false}
						onError={() => setBaseFailed(true)}
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
						onError={() => {
							if (activeDecodeRef.current === decodeToken) {
								setScreenFailed(true);
								onPreviewFailure?.();
							}
						}}
						ref={screenImageRef}
						src={currentUrl}
					/>
				) : null}
			</div>
		</div>
	);
}
