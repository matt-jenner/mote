import { Menu, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { useAppController } from "../app/useAppController";
import styles from "../styles/appShell.module.css";
import { AppearanceMenu } from "./AppearanceMenu";
import { NavigationRail } from "./NavigationRail";
import { SourceCanvas } from "./SourceCanvas";

export function AppShell() {
	const controller = useAppController();
	const [drawerOpen, setDrawerOpen] = useState(false);
	const drawerRef = useRef<HTMLElement>(null);
	const drawerTriggerRef = useRef<HTMLButtonElement>(null);
	const drawerCloseRef = useRef<HTMLButtonElement>(null);
	const drawerWasOpen = useRef(false);
	const source = controller.state?.activeSource ?? null;
	const appearance = controller.state?.settings.appearance ?? "system";
	const chooseFolder = () => controller.chooseFolder();

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
				inert={drawerOpen}
				onChooseFolder={chooseFolder}
			/>
			<section
				aria-label="Photo workspace"
				className={styles.workspace}
				inert={drawerOpen}
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
					/>
				)}
			</section>
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
