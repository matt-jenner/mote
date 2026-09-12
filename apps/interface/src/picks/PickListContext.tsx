import {
	createContext,
	type PropsWithChildren,
	useContext,
	useMemo,
} from "react";
import { usePhotoService } from "../app/PhotoServiceContext";
import { PickToast } from "../components/PickToast";
import { folderLabel, type SavedFolderSnapshot } from "../folders/savedFolders";
import styles from "../styles/appShell.module.css";
import {
	type PickListController,
	type PickOrigin,
	usePickListController,
} from "./usePickList";

interface PickListContextValue {
	controller: PickListController;
	origin: PickOrigin | null;
}

const PickListContext = createContext<PickListContextValue | null>(null);

export function pickOriginFromSavedFolders(
	snapshot: SavedFolderSnapshot | undefined,
): PickOrigin | null {
	if (!snapshot?.activeEntryId) return null;
	const entry = snapshot.entries.find(
		(candidate) => candidate.id === snapshot.activeEntryId,
	);
	return entry
		? { sourceFolderId: entry.folderId, sourceLabel: folderLabel(entry) }
		: null;
}

export function PickListProvider({
	children,
	origin,
	onViewPicks,
}: PropsWithChildren<{
	origin: PickOrigin | null;
	onViewPicks?: () => void;
}>) {
	const service = usePhotoService();
	const controller = usePickListController(service);
	const value = useMemo(() => ({ controller, origin }), [controller, origin]);

	return (
		<PickListContext value={value}>
			{children}
			{controller.snapshot.persistenceError ? (
				<aside
					aria-label="Pick storage warning"
					className={styles.pickPersistenceWarning}
				>
					{controller.snapshot.persistenceError}
				</aside>
			) : null}
			<PickToast onViewPicks={onViewPicks} toast={controller.toast} />
			<div
				aria-atomic="true"
				aria-live="polite"
				className={styles.visuallyHidden}
			>
				<span key={controller.announcementId}>{controller.announcement}</span>
			</div>
		</PickListContext>
	);
}

function usePickListValue(): PickListContextValue {
	const value = useContext(PickListContext);
	if (!value) throw new Error("PickListProvider is missing");
	return value;
}

export function usePickList(): PickListController {
	return usePickListValue().controller;
}

export function usePickListOrigin(): PickOrigin | null {
	return usePickListValue().origin;
}

export type { PickListController, PickOrigin } from "./usePickList";
