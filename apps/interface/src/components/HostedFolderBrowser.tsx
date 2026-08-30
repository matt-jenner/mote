import { ArrowLeft, ChevronRight, Folder, X } from "lucide-react";
import {
	type KeyboardEvent,
	useCallback,
	useEffect,
	useId,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import type {
	ChooseFolderResult,
	FolderBreadcrumb,
	FolderListing,
	PhotoService,
} from "../services/photoService";
import styles from "../styles/appShell.module.css";

export interface HostedFolderBrowserProps {
	initialBreadcrumbs: FolderBreadcrumb[];
	onClose(): void;
	onSelected(result: ChooseFolderResult): void;
	service: PhotoService;
}

function errorMessage(reason: unknown): string {
	return reason instanceof Error
		? reason.message
		: "That folder is unavailable.";
}

function useMediaQuery(query: string): boolean {
	const [matches, setMatches] = useState(
		() => window.matchMedia(query).matches,
	);

	useEffect(() => {
		const media = window.matchMedia(query);
		const update = () => setMatches(media.matches);
		update();
		media.addEventListener("change", update);
		return () => media.removeEventListener("change", update);
	}, [query]);

	return matches;
}

function uniqueRecoveryPaths(breadcrumbs: FolderBreadcrumb[]): string[] {
	const paths: string[] = [];
	for (const breadcrumb of [...breadcrumbs].reverse()) {
		if (!paths.includes(breadcrumb.path)) paths.push(breadcrumb.path);
	}
	if (!paths.includes("")) paths.push("");
	return paths;
}

export function HostedFolderBrowser({
	initialBreadcrumbs,
	onClose,
	onSelected,
	service,
}: HostedFolderBrowserProps) {
	const titleId = useId();
	const dialogRef = useRef<HTMLElement>(null);
	const closeRef = useRef<HTMLButtonElement>(null);
	const restoreFocusRef = useRef<HTMLElement | null>(
		document.activeElement instanceof HTMLElement
			? document.activeElement
			: null,
	);
	const requestIdRef = useRef(0);
	const mountedRef = useRef(true);
	const [listing, setListing] = useState<FolderListing | null>(null);
	const [loadingPath, setLoadingPath] = useState<string | null>("");
	const [selecting, setSelecting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [retryPath, setRetryPath] = useState<string | null>(null);
	const coarsePointer = useMediaQuery("(pointer: coarse)");
	const reducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)");
	const busy = loadingPath !== null || selecting;

	const loadPath = useCallback(
		async (path: string) => {
			const requestId = ++requestIdRef.current;
			setLoadingPath(path);
			setError(null);
			setRetryPath(path);
			try {
				const next = await service.listFolders(path);
				if (!mountedRef.current || requestId !== requestIdRef.current) return;
				setListing(next);
				setError(null);
				setRetryPath(null);
			} catch (reason) {
				if (!mountedRef.current || requestId !== requestIdRef.current) return;
				setError(errorMessage(reason));
			} finally {
				if (mountedRef.current && requestId === requestIdRef.current)
					setLoadingPath(null);
			}
		},
		[service],
	);

	useLayoutEffect(() => {
		mountedRef.current = true;
		closeRef.current?.focus();
		return () => {
			mountedRef.current = false;
			requestIdRef.current += 1;
			const restoreFocus = restoreFocusRef.current;
			if (restoreFocus?.isConnected) restoreFocus.focus();
		};
	}, []);

	useEffect(() => {
		const requestId = ++requestIdRef.current;
		setLoadingPath(initialBreadcrumbs.at(-1)?.path ?? "");
		setError(null);
		setRetryPath(null);

		void (async () => {
			let lastError: unknown = null;
			for (const path of uniqueRecoveryPaths(initialBreadcrumbs)) {
				try {
					const recovered = await service.listFolders(path);
					if (!mountedRef.current || requestId !== requestIdRef.current) return;
					setListing(recovered);
					setLoadingPath(null);
					return;
				} catch (reason) {
					lastError = reason;
				}
			}
			if (!mountedRef.current || requestId !== requestIdRef.current) return;
			setLoadingPath(null);
			setRetryPath("");
			setError(errorMessage(lastError));
		})();
	}, [initialBreadcrumbs, service]);

	const parentPath = (() => {
		if (!listing || listing.breadcrumbs.length === 0) return null;
		return listing.breadcrumbs.at(-2)?.path ?? "";
	})();

	const openCurrentFolder = async () => {
		if (!listing || busy) return;
		setSelecting(true);
		setError(null);
		setRetryPath(null);
		try {
			const result = await service.selectFolder(listing.path);
			if (!mountedRef.current) return;
			if (result.kind === "selected") onSelected(result);
		} catch (reason) {
			if (mountedRef.current) setError(errorMessage(reason));
		} finally {
			if (mountedRef.current) setSelecting(false);
		}
	};

	const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
		if (event.key === "Escape") {
			event.preventDefault();
			onClose();
			return;
		}
		if (event.key !== "Tab") return;

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

	return (
		<div
			className={`${styles.hostedFolderBackdrop} ${coarsePointer ? styles.hostedFolderBackdropSheet : ""}`}
		>
			<section
				aria-busy={busy}
				aria-labelledby={titleId}
				aria-modal="true"
				className={`${styles.hostedFolderSurface} ${coarsePointer ? styles.hostedFolderSheet : ""} ${reducedMotion ? styles.hostedFolderReducedMotion : ""}`}
				id="hosted-folder-browser"
				onKeyDown={handleKeyDown}
				ref={dialogRef}
				role="dialog"
			>
				<header className={styles.hostedFolderHeader}>
					<div>
						<span className={styles.hostedFolderEyebrow}>Photo source</span>
						<h2 className={styles.hostedFolderTitle} id={titleId}>
							Choose a folder
						</h2>
					</div>
					<button
						aria-label="Close folder browser"
						className={styles.iconButton}
						onClick={onClose}
						ref={closeRef}
						type="button"
					>
						<X aria-hidden="true" size={20} strokeWidth={1.7} />
					</button>
				</header>

				<div className={styles.hostedFolderNavigationRow}>
					<button
						className={styles.hostedFolderBack}
						disabled={busy || parentPath === null}
						onClick={() => {
							if (parentPath !== null) void loadPath(parentPath);
						}}
						type="button"
					>
						<ArrowLeft aria-hidden="true" size={18} strokeWidth={1.7} />
						Back
					</button>
					<nav aria-label="Folder path" className={styles.hostedBreadcrumbs}>
						<ol>
							<li>
								<button
									disabled={busy || listing?.path === ""}
									onClick={() => void loadPath("")}
									type="button"
								>
									Photos
								</button>
							</li>
							{listing?.breadcrumbs.map((breadcrumb, index) => {
								const current = index === listing.breadcrumbs.length - 1;
								return (
									<li key={breadcrumb.path}>
										<ChevronRight
											aria-hidden="true"
											size={14}
											strokeWidth={1.7}
										/>
										{current ? (
											<span aria-current="page">{breadcrumb.name}</span>
										) : (
											<button
												disabled={busy}
												onClick={() => void loadPath(breadcrumb.path)}
												type="button"
											>
												{breadcrumb.name}
											</button>
										)}
									</li>
								);
							})}
						</ol>
					</nav>
				</div>

				<div className={styles.hostedFolderBody}>
					{error ? (
						<div className={styles.hostedFolderError} role="alert">
							<span>{error}</span>
							{retryPath !== null ? (
								<button
									disabled={busy}
									onClick={() => void loadPath(retryPath)}
									type="button"
								>
									Retry
								</button>
							) : null}
						</div>
					) : null}
					{loadingPath !== null && !listing ? (
						<p aria-live="polite" className={styles.hostedFolderStatus}>
							Loading folders…
						</p>
					) : null}
					{listing ? (
						listing.children.length > 0 ? (
							<ul aria-label="Folders" className={styles.hostedFolderList}>
								{listing.children.map((child) => (
									<li key={child.path}>
										<button
											disabled={busy}
											onClick={() => void loadPath(child.path)}
											type="button"
										>
											<Folder aria-hidden="true" size={20} strokeWidth={1.5} />
											<span>{child.name}</span>
											<ChevronRight
												aria-hidden="true"
												size={17}
												strokeWidth={1.7}
											/>
										</button>
									</li>
								))}
							</ul>
						) : (
							<p className={styles.hostedFolderStatus}>
								No folders inside this folder.
							</p>
						)
					) : null}
				</div>

				<footer className={styles.hostedFolderFooter}>
					<span aria-live="polite" className={styles.hostedFolderBusyText}>
						{selecting
							? "Opening folder…"
							: loadingPath !== null
								? "Loading…"
								: ""}
					</span>
					<button
						className={styles.primaryButton}
						disabled={busy || !listing}
						onClick={() => void openCurrentFolder()}
						type="button"
					>
						Open this folder
					</button>
				</footer>
			</section>
		</div>
	);
}
