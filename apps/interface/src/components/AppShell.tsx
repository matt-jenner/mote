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
import type { PhotoService } from "../services/photoService";
import styles from "../styles/appShell.module.css";
import { initialViewerState, viewerReducer } from "../viewer/viewerReducer";
import { AppearanceMenu } from "./AppearanceMenu";
import { HostedFolderBrowser } from "./HostedFolderBrowser";
import { NavigationRail } from "./NavigationRail";
import { PhotoViewerOverlay } from "./PhotoViewerOverlay";
import { SourceCanvas } from "./SourceCanvas";
import { WallToolbar } from "./WallToolbar";

export function AppShell() {
	const controller = useAppController();
	const service = usePhotoService();
	const [drawerOpen, setDrawerOpen] = useState(false);
	const [folderBrowserBreadcrumbs, setFolderBrowserBreadcrumbs] = useState<
		ReturnType<PhotoService["folderBrowserState"]>["breadcrumbs"] | null
	>(null);
	const [viewer, dispatchViewer] = useReducer(
		viewerReducer,
		initialViewerState,
	);
	const [highlightedAssetId, setHighlightedAssetId] = useState<string | null>(
		null,
	);
	const wallRegionRef = useRef<HTMLElement>(null);
	const closingAnchorRef = useRef(viewer.returnAnchor);
	const closingFocusFallbackRef = useRef(false);
	const drawerRef = useRef<HTMLElement>(null);
	const drawerTriggerRef = useRef<HTMLButtonElement>(null);
	const drawerCloseRef = useRef<HTMLButtonElement>(null);
	const permanentFolderTriggerRef = useRef<HTMLButtonElement>(null);
	const folderRestoreFocusRef = useRef<HTMLElement | null>(null);
	const drawerWasOpen = useRef(false);
	const restoreFolderFocusPendingRef = useRef(false);
	const folderBrowserOpen = folderBrowserBreadcrumbs !== null;
	const source = controller.state?.activeSource ?? null;
	const galleryScope =
		controller.state?.settings.galleryScope ?? "includeSubfolders";
	const wall = usePhotoWall(source?.selectionId ?? null, galleryScope);
	const appearance = controller.state?.settings.appearance ?? "system";
	const chooseFolder = () => {
		if (controller.capabilities.folderSelection === "native") {
			controller.chooseFolder();
			return;
		}
		const activeElement = document.activeElement;
		folderRestoreFocusRef.current =
			activeElement instanceof HTMLElement && activeElement !== document.body
				? activeElement
				: drawerOpen
					? drawerTriggerRef.current
					: permanentFolderTriggerRef.current;
		setFolderBrowserBreadcrumbs(service.folderBrowserState().breadcrumbs);
	};
	const closeFolderBrowser = () => {
		restoreFolderFocusPendingRef.current = true;
		setFolderBrowserBreadcrumbs(null);
	};
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
		const anchor = viewer.returnAnchor;
		closingAnchorRef.current = anchor
			? {
					...anchor,
					assetId: viewer.currentAssetId ?? anchor.assetId,
				}
			: null;
		closingFocusFallbackRef.current =
			!viewer.currentAssetId ||
			!wall.state.items.some((item) => item.id === viewer.currentAssetId);
		dispatchViewer({ type: "close" });
	}, [viewer.currentAssetId, viewer.returnAnchor, wall.state.items]);
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
		const focusWall = closingFocusFallbackRef.current;
		closingFocusFallbackRef.current = false;
		if (wallRegionRef.current)
			wallRegionRef.current.scrollTop = anchor.scrollTop;
		const tile = focusWall
			? null
			: [
					...(wallRegionRef.current?.querySelectorAll<HTMLElement>(
						"[data-asset-id]",
					) ?? []),
				].find((candidate) => candidate.dataset.assetId === anchor.assetId);
		const focusTarget =
			tile?.querySelector<HTMLElement>("button[aria-label^='Open ']") ?? tile;
		if (focusTarget && typeof focusTarget.focus === "function")
			focusTarget.focus({ preventScroll: true });
		else wallRegionRef.current?.focus({ preventScroll: true });
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
		const anchor = viewer.returnAnchor;
		closingAnchorRef.current = anchor
			? {
					...anchor,
					assetId: viewer.currentAssetId,
				}
			: null;
		closingFocusFallbackRef.current = true;
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
			if (!folderBrowserOpen) drawerTriggerRef.current?.focus();
		}
	}, [drawerOpen, folderBrowserOpen]);

	useLayoutEffect(() => {
		if (folderBrowserOpen || !restoreFolderFocusPendingRef.current) return;
		restoreFolderFocusPendingRef.current = false;
		const restoreFocus = folderRestoreFocusRef.current;
		if (restoreFocus?.isConnected) restoreFocus.focus();
		else if (drawerTriggerRef.current?.isConnected)
			drawerTriggerRef.current.focus();
		else permanentFolderTriggerRef.current?.focus();
	}, [folderBrowserOpen]);

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
				folderBrowserOpen={folderBrowserOpen}
				folderButtonRef={permanentFolderTriggerRef}
				folderSelection={controller.capabilities.folderSelection}
				inert={drawerOpen || viewer.open || folderBrowserOpen}
				onChooseFolder={chooseFolder}
			/>
			<section
				aria-label="Photo workspace"
				className={styles.workspace}
				inert={drawerOpen || viewer.open || folderBrowserOpen}
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
						<span className={styles.eyebrow}>Library</span>
						<span className={styles.sourceTitle}>
							{source?.displayName ?? "All photos"}
						</span>
					</div>
					{source ? (
						<WallToolbar
							direction={wall.state.direction}
							galleryScope={galleryScope}
							onGalleryScopeChange={controller.updateGalleryScope}
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
					onSetInfoOpen={(open) =>
						dispatchViewer({ type: "setInfoOpen", open })
					}
					onShowControls={() => dispatchViewer({ type: "showControls" })}
					onHideControls={() => dispatchViewer({ type: "hideControls" })}
					onToggleTouchControls={() =>
						dispatchViewer({ type: "toggleTouchControls" })
					}
					service={service}
					state={viewer}
					onLoadMore={wall.loadMore}
					onRequestNearViewportDerivatives={wall.requestNearViewportDerivatives}
					onSetWallInteraction={wall.setWallInteraction}
					onSelectAsset={handleSelectViewerAsset}
					loading={wall.loading}
					nextCursor={wall.state.cursor}
				/>
			) : null}
			{folderBrowserBreadcrumbs ? (
				<HostedFolderBrowser
					initialBreadcrumbs={folderBrowserBreadcrumbs}
					onClose={closeFolderBrowser}
					onSelected={(result) => {
						controller.acceptFolderSelection(result);
						closeFolderBrowser();
					}}
					service={service}
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
							folderBrowserOpen={folderBrowserOpen}
							folderSelection={controller.capabilities.folderSelection}
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
