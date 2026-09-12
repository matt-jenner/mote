import { Bookmark, Copy, GripHorizontal, Trash2, X } from "lucide-react";
import {
	type KeyboardEvent,
	type PointerEvent,
	type Ref,
	useEffect,
	useLayoutEffect,
	useRef,
} from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import { usePickList } from "../picks/PickListContext";
import type { PickCopyFailureCode, PickCopyState } from "../picks/usePickList";
import styles from "../styles/picksPanel.module.css";
import { PickRow } from "./PickRow";

export type PicksPanelMode = "desktop" | "mobile";

const copyFailureMessages: Record<PickCopyFailureCode, string> = {
	source_unavailable: "Original unavailable",
	destination_unavailable: "Destination unavailable",
	destination_is_source: "Destination is a source folder",
	copy_failed: "Couldn't copy original",
};

interface PicksPanelProps {
	mode: PicksPanelMode;
	open: boolean;
	onOpen: () => void;
	onClose: () => void;
	onReview?: (assetId: string, launchTarget: HTMLElement) => void;
	onOpenPick?: (assetId: string, launchTarget: HTMLElement) => void;
	onRemovePick?: (assetId: string) => void;
	reviewOpen?: boolean;
}

function pickCountLabel(count: number): string {
	return `${count} ${count === 1 ? "pick" : "picks"}`;
}

export function PicksTrigger({
	count,
	expanded,
	onClick,
	className,
	triggerRef,
	copy,
}: {
	count: number;
	expanded: boolean;
	onClick: () => void;
	className?: string;
	triggerRef?: Ref<HTMLButtonElement>;
	copy?: PickCopyState;
}) {
	const progress =
		copy?.phase === "copying"
			? `Copying ${copy.completed} / ${copy.total}`
			: copy?.phase === "choosing"
				? "Choosing destination…"
				: "";
	return (
		<button
			aria-expanded={expanded}
			aria-label={`Picks, ${pickCountLabel(count)}${progress ? `, ${progress}` : ""}`}
			className={className}
			onClick={(event) => {
				onClick();
				event.currentTarget.focus();
			}}
			ref={triggerRef}
			type="button"
		>
			<Bookmark aria-hidden="true" size={18} strokeWidth={1.8} />
			<span>Picks</span>
			<span className={styles.triggerCount}>{count}</span>
			{progress ? <span>{progress}</span> : null}
		</button>
	);
}

export function PicksToolbarButton({
	expanded,
	onClick,
	className,
}: {
	expanded: boolean;
	onClick: () => void;
	className?: string;
}) {
	const picks = usePickList();
	return (
		<PicksTrigger
			className={className}
			count={picks.count}
			copy={picks.copy}
			expanded={expanded}
			onClick={onClick}
		/>
	);
}

export function PicksPanel({
	mode,
	open,
	onOpen,
	onClose,
	onReview,
	onOpenPick,
	onRemovePick,
	reviewOpen = false,
}: PicksPanelProps) {
	const picks = usePickList();
	const service = usePhotoService();
	const dialogRef = useRef<HTMLElement>(null);
	const closeRef = useRef<HTMLButtonElement>(null);
	const mobileTriggerRef = useRef<HTMLButtonElement>(null);
	const mobileWasOpen = useRef(false);
	const historyEntry = useRef(false);
	const dragStartY = useRef<number | null>(null);
	const isMobile = mode === "mobile";
	const items = picks.snapshot.items;
	const hasItems = items.length > 0;
	const copying =
		picks.copy.phase === "choosing" || picks.copy.phase === "copying";
	const showCopyActions = hasItems || picks.copy.phase !== "idle";
	const copyLabel =
		picks.copy.phase === "choosing"
			? "Choosing destination…"
			: picks.copy.phase === "copying"
				? `Copying ${picks.copy.completed} / ${picks.copy.total}`
				: picks.copy.phase === "partial"
					? `Retry ${picks.copy.failedAssetIds.length} originals…`
					: `Copy ${picks.count} ${picks.count === 1 ? "original" : "originals"}…`;
	const firstReviewableItem = items.find((item) => item.asset !== null);

	useEffect(() => {
		if (!hasItems) return;
		const missingThumbnails = items
			.filter((item) => item.asset && !item.asset.wallThumbnail)
			.map((item) => item.assetId);
		if (missingThumbnails.length > 0)
			picks.requestDerivatives(missingThumbnails, "wallThumbnail");
	}, [hasItems, items, picks]);

	useLayoutEffect(() => {
		if (isMobile && open) closeRef.current?.focus();
	}, [isMobile, open]);

	useLayoutEffect(() => {
		if (!isMobile || !open || !dialogRef.current) return;
		if (
			items.length === 0 ||
			!dialogRef.current.contains(document.activeElement)
		)
			closeRef.current?.focus();
	}, [isMobile, items, open]);

	useLayoutEffect(() => {
		if (!isMobile) return;
		if (open) {
			mobileWasOpen.current = true;
			return;
		}
		if (!mobileWasOpen.current) return;
		mobileWasOpen.current = false;
		mobileTriggerRef.current?.focus();
	}, [isMobile, open]);

	useEffect(() => {
		if (!isMobile || !open) return;
		const closeOnPlatformBack = () => {
			historyEntry.current = false;
			onClose();
		};
		window.addEventListener("popstate", closeOnPlatformBack);
		return () => window.removeEventListener("popstate", closeOnPlatformBack);
	}, [isMobile, onClose, open]);

	const openMobileSheet = () => {
		window.history.pushState({ picksSheet: true }, "");
		historyEntry.current = true;
		onOpen();
	};

	useEffect(() => {
		if ((open && isMobile) || !historyEntry.current) return;
		historyEntry.current = false;
		window.history.back();
	}, [isMobile, open]);

	useEffect(
		() => () => {
			if (!historyEntry.current) return;
			historyEntry.current = false;
			window.history.back();
		},
		[],
	);

	useEffect(() => {
		if (isMobile || !open) return;
		const closeOnEscape = (event: globalThis.KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.preventDefault();
			onClose();
		};
		window.addEventListener("keydown", closeOnEscape);
		return () => window.removeEventListener("keydown", closeOnEscape);
	}, [isMobile, onClose, open]);

	const trapFocus = (event: KeyboardEvent<HTMLElement>) => {
		if (event.key === "Escape") {
			event.preventDefault();
			onClose();
			return;
		}
		if (!isMobile || event.key !== "Tab") return;
		const focusable = Array.from(
			dialogRef.current?.querySelectorAll<HTMLElement>(
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

	const startDrag = (event: PointerEvent<HTMLElement>) => {
		dragStartY.current = event.clientY;
	};
	const endDrag = (event: PointerEvent<HTMLElement>) => {
		const start = dragStartY.current;
		dragStartY.current = null;
		if (start !== null && event.clientY - start >= 80) onClose();
	};

	const contents = (
		<>
			<div className={styles.panelHeader}>
				<div>
					<h2 className={styles.panelTitle}>Picks</h2>
					<p className={styles.panelCount}>{pickCountLabel(picks.count)}</p>
				</div>
				<button
					aria-label="Close picks"
					className={styles.closeButton}
					onClick={onClose}
					ref={closeRef}
					type="button"
				>
					<X aria-hidden="true" size={22} strokeWidth={1.8} />
				</button>
			</div>
			{showCopyActions ? (
				<>
					<ul className={styles.pickList}>
						{items.map((item) => (
							<PickRow
								item={item}
								key={item.assetId}
								onRemove={(assetId) =>
									onRemovePick
										? onRemovePick(assetId)
										: void picks.remove(assetId).catch(() => {})
								}
								action={
									<div>
										{picks.copy.failures
											.filter((failure) => failure.assetId === item.assetId)
											.map((failure) => (
												<span
													key={failure.assetId}
													className={styles.sourceWarning}
												>
													{copyFailureMessages[failure.code]}
												</span>
											))}
										{item.asset && onOpenPick ? (
											<button
												aria-label={`Review ${item.asset.displayName}`}
												onClick={(event) =>
													onOpenPick(item.assetId, event.currentTarget)
												}
												type="button"
											>
												Review
											</button>
										) : null}
									</div>
								}
							/>
						))}
					</ul>
					{!hasItems ? (
						<p className={styles.emptyState}>
							Add photos to picks as you browse.
						</p>
					) : null}
					<div className={styles.actions} data-testid="picks-actions">
						{picks.copy.phase === "copying" ? (
							<progress
								aria-label="Copy originals"
								value={picks.copy.completed}
								max={picks.copy.total}
							/>
						) : null}
						{!copying && picks.copy.message ? (
							<p>{picks.copy.message}</p>
						) : null}
						{hasItems ? (
							<button
								className={styles.reviewButton}
								disabled={!onReview || !firstReviewableItem}
								onClick={(event) => {
									if (!firstReviewableItem || !onReview) return;
									onReview(firstReviewableItem.assetId, event.currentTarget);
								}}
								type="button"
							>
								Review picks
							</button>
						) : null}
						{hasItems || copying || picks.copy.phase === "partial" ? (
							<button
								className={styles.copyButton}
								disabled={
									copying || service.capabilities.originalAction !== "copy"
								}
								onClick={() => void picks.copyOriginals()}
								type="button"
							>
								<Copy aria-hidden="true" size={18} strokeWidth={1.8} />
								{copyLabel}
							</button>
						) : null}
						{picks.copy.copiedCount > 0 && !copying ? (
							<button
								type="button"
								className={styles.reviewButton}
								onClick={() => void picks.showCopyFolder()}
							>
								Show folder
							</button>
						) : null}
						{hasItems ? (
							<button
								className={styles.clearButton}
								onClick={() => void picks.clear().catch(() => {})}
								type="button"
							>
								<Trash2 aria-hidden="true" size={18} strokeWidth={1.8} />
								Clear picks
							</button>
						) : null}
					</div>
				</>
			) : (
				<p className={styles.emptyState}>Add photos to picks as you browse.</p>
			)}
		</>
	);

	if (!isMobile) {
		if (!open) return null;
		return (
			<aside
				aria-label="Picks"
				className={styles.desktopPanel}
				onKeyDown={trapFocus}
				ref={dialogRef}
			>
				{contents}
			</aside>
		);
	}

	return (
		<>
			<PicksTrigger
				className={styles.mobileBar}
				count={picks.count}
				copy={picks.copy}
				expanded={open}
				onClick={open ? onClose : openMobileSheet}
				triggerRef={mobileTriggerRef}
			/>
			{open && !reviewOpen ? (
				<div className={styles.mobileBackdrop}>
					<button
						aria-label="Dismiss picks"
						className={styles.dismissButton}
						onClick={onClose}
						tabIndex={-1}
						type="button"
					/>
					<section
						aria-label="Picks"
						aria-modal="true"
						className={styles.mobileSheet}
						onKeyDown={trapFocus}
						ref={dialogRef}
						role="dialog"
					>
						<div
							aria-hidden="true"
							className={styles.dragHandle}
							onPointerDown={startDrag}
							onPointerUp={endDrag}
						>
							<GripHorizontal size={24} strokeWidth={1.8} />
						</div>
						{contents}
					</section>
				</div>
			) : null}
		</>
	);
}
