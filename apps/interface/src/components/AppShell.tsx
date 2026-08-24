import { Menu, X } from "lucide-react";
import { useState } from "react";
import { useAppController } from "../app/useAppController";
import styles from "../styles/appShell.module.css";
import { AppearanceMenu } from "./AppearanceMenu";
import { NavigationRail } from "./NavigationRail";
import { SourceCanvas } from "./SourceCanvas";

export function AppShell() {
	const controller = useAppController();
	const [drawerOpen, setDrawerOpen] = useState(false);
	const source = controller.state?.activeSource ?? null;
	const appearance = controller.state?.settings.appearance ?? "system";
	const chooseFolder = () => controller.chooseFolder();

	return (
		<div className={styles.appShell}>
			<NavigationRail
				chooseFolderAvailable={controller.capabilities.chooseFolder}
				className={styles.permanentRail}
				onChooseFolder={chooseFolder}
			/>
			<section className={styles.workspace}>
				<header className={styles.toolbar}>
					<button
						aria-label="Open sources"
						className={`${styles.iconButton} ${styles.drawerTrigger}`}
						onClick={() => setDrawerOpen(true)}
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
						role="dialog"
					>
						<div className={styles.drawerHeader}>
							<span>Sources</span>
							<button
								aria-label="Close sources"
								className={styles.iconButton}
								onClick={() => setDrawerOpen(false)}
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
