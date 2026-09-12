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
import { SourceUnavailableContext } from "../folders/SourceAvailabilityContext";
import {
	activeFolderAccess,
	emptySavedFolders,
	sourceIsUnavailable,
} from "../folders/savedFolders";
import {
	PickListProvider,
	pickOriginFromSavedFolders,
	usePickList,
} from "../picks/PickListContext";
import type { PhotoService } from "../services/photoService";
import styles from "../styles/appShell.module.css";
import picksPanelStyles from "../styles/picksPanel.module.css";
import {
	hydratePickSequence,
	nextPickAfterRemoval,
} from "../viewer/pickSequence";
import { initialViewerState, viewerReducer } from "../viewer/viewerReducer";
import { AppearanceMenu } from "./AppearanceMenu";
import { HostedFolderBrowser } from "./HostedFolderBrowser";
import { NavigationRail } from "./NavigationRail";
import { PhotoViewerOverlay } from "./PhotoViewerOverlay";
import { PicksPanel, PicksToolbarButton } from "./PicksPanel";
import { SourceCanvas } from "./SourceCanvas";
import { WallToolbar } from "./WallToolbar";

function AppShellContents({
	controller,
}: {
	controller: ReturnType<typeof useAppController>;
}) {
	const service = usePhotoService();
	const [drawerOpen, setDrawerOpen] = useState(false);
	const [picksOpen, setPicksOpen] = useState(false);
	const [isMobile, setIsMobile] = useState(
		() => window.matchMedia("(max-width: 899px)").matches,
	);
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
	const closingReturnSurfaceRef = useRef(viewer.returnSurface);
	const pickReviewFocusRef = useRef<HTMLElement | null>(null);
	const drawerRef = useRef<HTMLElement>(null);
	const drawerTriggerRef = useRef<HTMLButtonElement>(null);
	const drawerCloseRef = useRef<HTMLButtonElement>(null);
	const permanentFolderTriggerRef = useRef<HTMLButtonElement>(null);
	const folderRestoreFocusRef = useRef<HTMLElement | null>(null);
	const drawerWasOpen = useRef(false);
	const restoreFolderFocusPendingRef = useRef(false);
	const folderBrowserOpen = folderBrowserBreadcrumbs !== null;
	const source = controller.state?.activeSource ?? null;
	const picks = usePickList();
	const pickAssets = hydratePickSequence(picks.snapshot.items);
	const galleryScope =
		controller.state?.settings.galleryScope ?? "includeSubfolders";
	const wall = usePhotoWall(source?.selectionId ?? null, galleryScope);
	const activeAccess = activeFolderAccess(
		controller.state?.savedFolders ?? emptySavedFolders(),
	);
	const folderUnavailable =
		activeAccess === "available"
			? false
			: sourceIsUnavailable(activeAccess) ||
				(source !== null && source.availability !== "available") ||
				wall.state.items.some(
					(asset) => asset.warning?.code === "sourceUnavailable",
				);
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
				((folderUnavailable || asset.availability !== "available") &&
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
		[wall.state.items, folderUnavailable],
	);
	const handleOpenPickReview = useCallback(
		(assetId: string, launchTarget: HTMLElement) => {
			if (!pickAssets.some((asset) => asset.id === assetId)) return;
			pickReviewFocusRef.current = launchTarget;
			dispatchViewer({
				type: "open",
				assetId,
				anchor: {
					assetId: wall.state.items[0]?.id ?? assetId,
					scrollTop: wallRegionRef.current?.scrollTop ?? 0,
				},
				sequence: "picks",
				returnSurface: "picksPanel",
			});
		},
		[pickAssets, wall.state.items],
	);
	const handleCloseViewer = useCallback(() => {
		const anchor = viewer.returnAnchor;
		const returningToPicks = viewer.returnSurface === "picksPanel";
		closingAnchorRef.current = anchor
			? {
					...anchor,
					assetId: returningToPicks
						? anchor.assetId
						: (viewer.currentAssetId ?? anchor.assetId),
				}
			: null;
		closingReturnSurfaceRef.current = viewer.returnSurface;
		closingFocusFallbackRef.current =
			!returningToPicks &&
			(!viewer.currentAssetId ||
				!wall.state.items.some((item) => item.id === viewer.currentAssetId));
		dispatchViewer({ type: "close" });
	}, [
		viewer.currentAssetId,
		viewer.returnAnchor,
		viewer.returnSurface,
		wall.state.items,
	]);
	const handleSelectViewerAsset = useCallback(
		(assetId: string) => {
			const assets =
				viewer.sequence === "picks" ? pickAssets : wall.state.items;
			if (!assets.some((item) => item.id === assetId)) return;
			dispatchViewer({ type: "select", assetId });
		},
		[pickAssets, viewer.sequence, wall.state.items],
	);
	const handleRemovePick = useCallback(
		(assetId: string) => {
			if (
				viewer.open &&
				viewer.sequence === "picks" &&
				viewer.currentAssetId === assetId
			) {
				const nextAssetId = nextPickAfterRemoval(picks.snapshot.items, assetId);
				if (nextAssetId)
					dispatchViewer({ type: "select", assetId: nextAssetId });
				else handleCloseViewer();
			}
			void picks.remove(assetId).catch(() => {});
		},
		[
			handleCloseViewer,
			picks,
			viewer.currentAssetId,
			viewer.open,
			viewer.sequence,
		],
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
		const returnSurface = closingReturnSurfaceRef.current;
		closingReturnSurfaceRef.current = "wall";
		if (returnSurface === "picksPanel") {
			setPicksOpen(true);
			const focusTarget = pickReviewFocusRef.current;
			window.requestAnimationFrame(() => {
				if (focusTarget?.isConnected)
					focusTarget.focus({ preventScroll: true });
				else
					document
						.querySelector<HTMLElement>(
							focusTarget?.getAttribute("aria-label")
								? `button[aria-label="${CSS.escape(focusTarget.getAttribute("aria-label") ?? "")}"]`
								: "[data-picks-review]",
						)
						?.focus({ preventScroll: true });
			});
			if (wallRegionRef.current)
				wallRegionRef.current.scrollTop = anchor.scrollTop;
			return;
		}
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
			viewer.sequence !== "wall" ||
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
		viewer.sequence,
		wall.state.items,
	]);

	useEffect(() => {
		if (
			!viewer.open ||
			viewer.sequence !== "picks" ||
			!viewer.currentAssetId ||
			pickAssets.some((asset) => asset.id === viewer.currentAssetId)
		)
			return;
		const nextAssetId = nextPickAfterRemoval(
			picks.snapshot.items,
			viewer.currentAssetId,
		);
		if (nextAssetId) dispatchViewer({ type: "select", assetId: nextAssetId });
		else handleCloseViewer();
	}, [
		handleCloseViewer,
		pickAssets,
		picks.snapshot.items,
		viewer.currentAssetId,
		viewer.open,
		viewer.sequence,
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
		const phoneViewport = window.matchMedia("(max-width: 899px)");
		const updateViewport = () => setIsMobile(phoneViewport.matches);
		const closeDrawerAbovePhoneWidth = (event: MediaQueryListEvent) => {
			if (!event.matches) setDrawerOpen(false);
		};

		updateViewport();
		phoneViewport.addEventListener("change", updateViewport);
		phoneViewport.addEventListener("change", closeDrawerAbovePhoneWidth);
		return () => {
			phoneViewport.removeEventListener("change", updateViewport);
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
		<SourceUnavailableContext value={folderUnavailable}>
			<div
				className={`${styles.appShell} ${
					picksOpen && !isMobile ? styles.appShellPicksOpen : ""
				}`}
			>
				<NavigationRail
					savedFolders={controller.state?.savedFolders}
					onActivate={async (id) => {
						if (await controller.activateSavedFolder(id)) setDrawerOpen(false);
					}}
					onRename={controller.renameSavedFolder}
					onRemove={controller.removeSavedFolder}
					chooseFolderAvailable={controller.capabilities.chooseFolder}
					className={styles.permanentRail}
					folderBrowserOpen={folderBrowserOpen}
					folderButtonRef={permanentFolderTriggerRef}
					folderSelection={controller.capabilities.folderSelection}
					inert={
						drawerOpen ||
						viewer.open ||
						folderBrowserOpen ||
						(picksOpen && isMobile)
					}
					onChooseFolder={chooseFolder}
				/>
				<section
					aria-label="Photo workspace"
					className={styles.workspace}
					inert={
						drawerOpen ||
						viewer.open ||
						folderBrowserOpen ||
						(picksOpen && isMobile)
					}
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
								progress={folderUnavailable ? undefined : wall.progress}
								status={
									folderUnavailable
										? "Source unavailable. Showing cached images."
										: wall.status
								}
								retryable={Boolean(wall.state.error)}
							/>
						) : null}
						{!isMobile ? (
							<PicksToolbarButton
								className={`${styles.picksToolbarTrigger} ${picksPanelStyles.toolbarTrigger}`}
								expanded={picksOpen}
								onClick={() => setPicksOpen((open) => !open)}
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
							hasOpenedFolder={controller.state?.savedFolders.hasOpenedFolder}
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
				{!isMobile ? (
					<PicksPanel
						mode="desktop"
						onClose={() => setPicksOpen(false)}
						onOpen={() => setPicksOpen(true)}
						onOpenPick={handleOpenPickReview}
						onRemovePick={handleRemovePick}
						onReview={handleOpenPickReview}
						open={picksOpen}
						reviewOpen={viewer.open && viewer.sequence === "picks"}
					/>
				) : (
					<PicksPanel
						mode="mobile"
						onClose={() => setPicksOpen(false)}
						onOpen={() => setPicksOpen(true)}
						onOpenPick={handleOpenPickReview}
						onRemovePick={handleRemovePick}
						onReview={handleOpenPickReview}
						open={picksOpen}
						reviewOpen={viewer.open && viewer.sequence === "picks"}
					/>
				)}
				{viewer.open ? (
					<PhotoViewerOverlay
						assets={
							viewer.sequence === "picks"
								? pickAssets
								: wall.state.items.filter(
										(asset) =>
											(!folderUnavailable &&
												asset.availability === "available") ||
											asset.wallThumbnail ||
											asset.screenPreview ||
											asset.id === viewer.currentAssetId,
									)
						}
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
						onLoadMore={viewer.sequence === "picks" ? () => {} : wall.loadMore}
						onRequestNearViewportDerivatives={
							viewer.sequence === "picks"
								? (assetIds) =>
										picks.requestDerivatives(assetIds, "wallThumbnail")
								: wall.requestNearViewportDerivatives
						}
						onRequestPreviewDerivatives={
							viewer.sequence === "picks"
								? async (request) => {
										picks.requestDerivatives(
											request.assetIds,
											request.kind,
											request.priority,
										);
									}
								: undefined
						}
						onRemovePick={handleRemovePick}
						onSetWallInteraction={wall.setWallInteraction}
						onSelectAsset={handleSelectViewerAsset}
						loading={wall.loading}
						nextCursor={viewer.sequence === "picks" ? null : wall.state.cursor}
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
						<button
							type="button"
							aria-label="Dismiss sources drawer"
							tabIndex={-1}
							className={styles.drawerDismiss}
							onClick={() => setDrawerOpen(false)}
						/>
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
								savedFolders={controller.state?.savedFolders}
								onActivate={async (id) => {
									if (await controller.activateSavedFolder(id))
										setDrawerOpen(false);
								}}
								onRename={controller.renameSavedFolder}
								onRemove={controller.removeSavedFolder}
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
		</SourceUnavailableContext>
	);
}

export function AppShell() {
	const controller = useAppController();
	const pickOrigin = pickOriginFromSavedFolders(controller.state?.savedFolders);
	return (
		<PickListProvider origin={pickOrigin}>
			<AppShellContents controller={controller} />
		</PickListProvider>
	);
}
