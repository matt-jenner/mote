import { Menu, X } from "lucide-react";
import {
	type KeyboardEvent,
	useCallback,
	useEffect,
	useLayoutEffect,
	useReducer,
	useRef,
	useState,
} from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import { useAppController } from "../app/useAppController";
import { usePhotoWall } from "../app/usePhotoWall";
import styles from "../styles/appShell.module.css";
import { initialViewerState, viewerReducer } from "../viewer/viewerReducer";
import { AppearanceMenu } from "./AppearanceMenu";
import { NavigationRail } from "./NavigationRail";
import { PhotoViewerOverlay } from "./PhotoViewerOverlay";
import { SourceCanvas } from "./SourceCanvas";
import { WallToolbar } from "./WallToolbar";

export function AppShell() {
	const controller = useAppController();
	const service = usePhotoService();
	const [drawerOpen, setDrawerOpen] = useState(false);
	const [viewer, dispatchViewer] = useReducer(
		viewerReducer,
		initialViewerState,
	);
	const [highlightedAssetId, setHighlightedAssetId] = useState<string | null>(
		null,
	);
	const wallRegionRef = useRef<HTMLElement>(null);
	const closingAnchorRef = useRef(viewer.returnAnchor);
	const drawerRef = useRef<HTMLElement>(null);
	const drawerTriggerRef = useRef<HTMLButtonElement>(null);
	const drawerCloseRef = useRef<HTMLButtonElement>(null);
	const drawerWasOpen = useRef(false);
	const source = controller.state?.activeSource ?? null;
	const wall = usePhotoWall(source?.selectionId ?? null);
	const appearance = controller.state?.settings.appearance ?? "system";
	const chooseFolder = () => controller.chooseFolder();
	const handleOpenViewer = useCallback(
		(assetId: string) => {
			const asset = wall.state.items.find((item) => item.id === assetId);
			if (
				!asset ||
				(asset.availability !== "available" &&
					!asset.wallThumbnail &&
					!asset.screenPreview)
			)
				return;
			dispatchViewer({
				type: "open",
				assetId,
				anchor: {
					assetId,
					scrollTop: wallRegionRef.current?.scrollTop ?? 0,
				},
			});
		},
		[wall.state.items],
	);
	const handleCloseViewer = useCallback(() => {
		closingAnchorRef.current = viewer.returnAnchor;
		dispatchViewer({ type: "close" });
	}, [viewer.returnAnchor]);
	const handleSelectViewerAsset = useCallback(
		(assetId: string) => {
			if (!wall.state.items.some((item) => item.id === assetId)) return;
			dispatchViewer({ type: "select", assetId });
		},
		[wall.state.items],
	);

	useLayoutEffect(() => {
		if (viewer.open) {
			if (viewer.returnAnchor && wallRegionRef.current)
				wallRegionRef.current.scrollTop = viewer.returnAnchor.scrollTop;
			return;
		}
		if (!closingAnchorRef.current) return;
		const anchor = closingAnchorRef.current;
		closingAnchorRef.current = null;
		if (wallRegionRef.current)
			wallRegionRef.current.scrollTop = anchor.scrollTop;
		const tile = [
			...document.querySelectorAll<HTMLElement>("[data-asset-id]"),
		].find((candidate) => candidate.dataset.assetId === anchor.assetId);
		tile?.focus({ preventScroll: true });
		setHighlightedAssetId(anchor.assetId);
		const timer = window.setTimeout(() => setHighlightedAssetId(null), 600);
		return () => window.clearTimeout(timer);
	}, [viewer.open, viewer.returnAnchor]);

	useEffect(() => {
		if (
			!viewer.open ||
			!viewer.currentAssetId ||
			wall.state.items.some((item) => item.id === viewer.currentAssetId)
		)
			return;
		closingAnchorRef.current = viewer.returnAnchor;
		dispatchViewer({ type: "close" });
	}, [
		viewer.currentAssetId,
		viewer.open,
		viewer.returnAnchor,
		wall.state.items,
	]);

	useEffect(() => {
		if (drawerOpen) {
			drawerWasOpen.current = true;
			drawerCloseRef.current?.focus();
			return;
		}
		if (drawerWasOpen.current) {
			drawerWasOpen.current = false;
			drawerTriggerRef.current?.focus();
		}
	}, [drawerOpen]);

	useEffect(() => {
		const phoneViewport = window.matchMedia("(max-width: 639px)");
		const closeDrawerAbovePhoneWidth = (event: MediaQueryListEvent) => {
			if (!event.matches) setDrawerOpen(false);
		};

		phoneViewport.addEventListener("change", closeDrawerAbovePhoneWidth);
		return () => {
			phoneViewport.removeEventListener("change", closeDrawerAbovePhoneWidth);
		};
	}, []);

	const handleDrawerKeyDown = (event: KeyboardEvent<HTMLElement>) => {
		if (event.key === "Escape") {
			event.preventDefault();
			setDrawerOpen(false);
			return;
		}
		if (event.key !== "Tab") return;

		const focusable = Array.from(
			drawerRef.current?.querySelectorAll<HTMLElement>(
				'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
			) ?? [],
		);
		const first = focusable[0];
		const last = focusable.at(-1);
		if (!first || !last) return;
		if (event.shiftKey && document.activeElement === first) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && document.activeElement === last) {
			event.preventDefault();
			first.focus();
		}
	};

	return (
		<div className={styles.appShell}>
			<NavigationRail
				chooseFolderAvailable={controller.capabilities.chooseFolder}
				className={styles.permanentRail}
				inert={drawerOpen || viewer.open}
				onChooseFolder={chooseFolder}
			/>
			<section
				aria-label="Photo workspace"
				className={styles.workspace}
				inert={drawerOpen || viewer.open}
			>
				<header className={styles.toolbar}>
					<button
						aria-label="Open sources"
						className={`${styles.iconButton} ${styles.drawerTrigger}`}
						onClick={() => setDrawerOpen(true)}
						ref={drawerTriggerRef}
						type="button"
					>
						<Menu aria-hidden="true" size={20} strokeWidth={1.7} />
					</button>
					<div className={styles.titleGroup}>
						<span className={styles.eyebrow}>Photo Viewer</span>
						<span className={styles.sourceTitle}>
							{source?.displayName ?? "Library"}
						</span>
					</div>
					{source ? (
						<WallToolbar
							direction={wall.state.direction}
							onDirectionChange={wall.setDirection}
							onRetry={wall.retry}
							progress={wall.progress}
							status={wall.status}
							retryable={Boolean(wall.state.error)}
						/>
					) : null}
					<AppearanceMenu
						onChange={controller.updateAppearance}
						value={appearance}
					/>
				</header>
				{controller.error ? (
					<div className={styles.errorBanner} role="alert">
						{controller.error instanceof Error
							? controller.error.message
							: "Something went wrong"}
					</div>
				) : null}
				{controller.loading ? (
					<main aria-busy="true" className={styles.canvas} />
				) : (
					<SourceCanvas
						chooseFolderAvailable={controller.capabilities.chooseFolder}
						onChooseFolder={chooseFolder}
						source={source}
						wall={wall}
						regionRef={wallRegionRef}
						onOpen={handleOpenViewer}
						highlightedAssetId={highlightedAssetId}
					/>
				)}
			</section>
			{viewer.open ? (
				<PhotoViewerOverlay
					assets={wall.state.items}
					onClose={handleCloseViewer}
					service={service}
					state={viewer}
					onLoadMore={wall.loadMore}
					onRequestNearViewportDerivatives={wall.requestNearViewportDerivatives}
					onSelectAsset={handleSelectViewerAsset}
					loading={wall.loading}
					nextCursor={wall.state.cursor}
				/>
			) : null}
			{drawerOpen ? (
				<div className={styles.drawerBackdrop}>
					<section
						aria-label="Sources drawer"
						aria-modal="true"
						className={styles.drawer}
						onKeyDown={handleDrawerKeyDown}
						ref={drawerRef}
						role="dialog"
					>
						<div className={styles.drawerHeader}>
							<span>Sources</span>
							<button
								aria-label="Close sources"
								className={styles.iconButton}
								onClick={() => setDrawerOpen(false)}
								ref={drawerCloseRef}
								type="button"
							>
								<X aria-hidden="true" size={20} strokeWidth={1.7} />
							</button>
						</div>
						<NavigationRail
							chooseFolderAvailable={controller.capabilities.chooseFolder}
							className={styles.drawerNavigation}
							onChooseFolder={() => {
								chooseFolder();
								setDrawerOpen(false);
							}}
						/>
					</section>
				</div>
			) : null}
		</div>
	);
}
