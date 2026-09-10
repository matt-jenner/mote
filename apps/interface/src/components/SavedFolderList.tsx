import { Ellipsis, Folder, LoaderCircle, TriangleAlert } from "lucide-react";
import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import {
	folderLabel,
	type SavedFolderSnapshot,
	sortSavedFolders,
	sourceIsUnavailable,
} from "../folders/savedFolders";
import styles from "../styles/appShell.module.css";

interface Props {
	snapshot: SavedFolderSnapshot;
	onActivate: (id: string) => Promise<void>;
	onRename: (id: string, label: string) => Promise<void>;
	onRemove: (id: string) => Promise<void>;
}
export function SavedFolderList({
	snapshot,
	onActivate,
	onRename,
	onRemove,
}: Props) {
	const [menu, setMenu] = useState<string | null>(null);
	const [editing, setEditing] = useState<string | null>(null);
	const [label, setLabel] = useState("");
	const [message, setMessage] = useState("");
	const root = useRef<HTMLUListElement>(null);
	const menuRef = useRef<HTMLDivElement>(null);
	const input = useRef<HTMLInputElement>(null);
	const entries = sortSavedFolders(snapshot.entries);
	const focusMenu = useCallback(
		(id: string) =>
			requestAnimationFrame(() => {
				[
					...(root.current?.querySelectorAll<HTMLButtonElement>(
						"button[data-folder-menu]",
					) ?? []),
				]
					.find((button) => button.dataset.folderMenu === id)
					?.focus();
			}),
		[],
	);
	const focusRow = (id: string | null) =>
		requestAnimationFrame(() => {
			const target = [
				...(root.current?.querySelectorAll<HTMLButtonElement>(
					"button[data-folder-id]",
				) ?? []),
			].find((button) => button.dataset.folderId === id);
			(
				target ??
				root.current
					?.closest("nav")
					?.querySelector<HTMLButtonElement>("[data-add-folder]")
			)?.focus();
		});
	useEffect(() => {
		if (editing) {
			input.current?.focus();
			input.current?.select();
		}
	}, [editing]);
	useLayoutEffect(() => {
		const popup = menuRef.current;
		if (!menu || !popup) return;
		popup.style.top = "48px";
		popup.style.bottom = "auto";
		const rail = root.current?.closest("nav")?.getBoundingClientRect();
		if (
			popup.getBoundingClientRect().bottom >
			Math.min(window.innerHeight - 8, (rail?.bottom ?? window.innerHeight) - 8)
		) {
			popup.style.top = "auto";
			popup.style.bottom = "48px";
		}
	}, [menu]);
	useEffect(() => {
		if (!menu) return;
		menuRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
		const close = (event: PointerEvent) => {
			if (!menuRef.current?.contains(event.target as Node)) {
				setMenu(null);
				focusMenu(menu);
			}
		};
		document.addEventListener("pointerdown", close);
		return () => document.removeEventListener("pointerdown", close);
	}, [menu, focusMenu]);
	const run = async (work: () => Promise<void>) => {
		try {
			await work();
		} catch (error) {
			setMessage(
				error instanceof Error
					? error.message
					: "The folder could not be updated.",
			);
		}
	};
	return (
		<>
			<ul className={styles.savedFolders} ref={root}>
				{entries.map((entry, index) => {
					const state = snapshot.access[entry.folderId]?.state ?? "unknown";
					const unavailable = sourceIsUnavailable(state);
					const checking = state === "checking" || state === "unknown";
					const active = snapshot.activeEntryId === entry.id;
					const title = `${folderLabel(entry)}\n${entry.displayPath || "/"}${unavailable ? "\nFolder unavailable. Select to check again." : checking ? "\nChecking folder…" : ""}`;
					return (
						<li
							key={entry.id}
							className={`${styles.savedFolder} ${active ? styles.savedFolderActive : ""} ${unavailable || checking ? styles.savedFolderUnavailable : ""}`}
						>
							{editing === entry.id ? (
								<input
									ref={input}
									aria-label="Folder label"
									value={label}
									className={styles.folderLabelInput}
									maxLength={256}
									onChange={(e) => setLabel(e.target.value)}
									onBlur={() => setEditing(null)}
									onKeyDown={(event) => {
										if (event.key === "Escape") {
											event.preventDefault();
											setEditing(null);
											focusRow(entry.id);
										}
										if (event.key === "Enter") {
											event.preventDefault();
											setEditing(null);
											void run(async () => {
												await onRename(entry.id, label);
												focusRow(entry.id);
											});
										}
									}}
								/>
							) : (
								<button
									type="button"
									data-folder-id={entry.id}
									aria-current={active ? "page" : undefined}
									aria-busy={checking}
									aria-description={checking ? "Checking folder" : undefined}
									title={title}
									className={styles.savedFolderButton}
									onClick={() => void run(() => onActivate(entry.id))}
								>
									{checking ? (
										<LoaderCircle
											aria-hidden="true"
											size={18}
											className={styles.folderSpinner}
										/>
									) : unavailable ? (
										<TriangleAlert
											aria-hidden="true"
											size={18}
											className={styles.folderWarningIcon}
										/>
									) : (
										<Folder aria-hidden="true" size={18} strokeWidth={1.6} />
									)}
									<span>{folderLabel(entry)}</span>
									{unavailable ? (
										<span className={styles.visuallyHidden}>
											{" "}
											unavailable; select to recheck
										</span>
									) : null}
								</button>
							)}
							<button
								type="button"
								data-folder-menu={entry.id}
								className={styles.folderMore}
								aria-label={`Options for ${folderLabel(entry)}`}
								aria-haspopup="menu"
								aria-expanded={menu === entry.id}
								onClick={() => setMenu(menu === entry.id ? null : entry.id)}
							>
								<Ellipsis aria-hidden="true" size={20} />
							</button>
							{menu === entry.id ? (
								<div
									ref={menuRef}
									role="menu"
									aria-label={`Options for ${folderLabel(entry)}`}
									className={styles.folderMenu}
									onKeyDown={(event) => {
										if (event.key === "Escape") {
											event.preventDefault();
											event.stopPropagation();
											setMenu(null);
											focusMenu(entry.id);
										}
										if (event.key === "Tab") setMenu(null);
										if (
											["ArrowDown", "ArrowUp", "Home", "End"].includes(
												event.key,
											)
										) {
											event.preventDefault();
											const buttons = [
												...event.currentTarget.querySelectorAll<HTMLButtonElement>(
													"button",
												),
											];
											const current = buttons.indexOf(
												document.activeElement as HTMLButtonElement,
											);
											buttons[
												event.key === "Home"
													? 0
													: event.key === "End"
														? buttons.length - 1
														: (current +
																(event.key === "ArrowUp" ? -1 : 1) +
																buttons.length) %
															buttons.length
											]?.focus();
										}
									}}
								>
									{!unavailable && !checking ? (
										<button
											type="button"
											role="menuitem"
											onClick={() => {
												setMenu(null);
												setLabel(folderLabel(entry));
												setEditing(entry.id);
											}}
										>
											Rename
										</button>
									) : null}
									<button
										type="button"
										role="menuitem"
										onClick={() => {
											setMenu(null);
											void run(async () => {
												await onRemove(entry.id);
												focusRow(
													entries[index + 1]?.id ??
														entries[index - 1]?.id ??
														null,
												);
											});
										}}
									>
										Remove
									</button>
								</div>
							) : null}
						</li>
					);
				})}
			</ul>
			{message ? (
				<p role="alert" className={styles.folderNotice}>
					{message}
				</p>
			) : null}
			{snapshot.persistenceError ? (
				<p role="status" className={styles.folderNotice}>
					{snapshot.persistenceError}
				</p>
			) : null}
		</>
	);
}
