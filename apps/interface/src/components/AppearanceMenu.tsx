import { SunMoon } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import type { Appearance } from "../services/photoService";
import styles from "../styles/appShell.module.css";

const appearances: ReadonlyArray<{ value: Appearance; label: string }> = [
	{ value: "system", label: "System" },
	{ value: "light", label: "Light" },
	{ value: "dark", label: "Dark" },
];

interface AppearanceMenuProps {
	value: Appearance;
	onChange: (appearance: Appearance) => void;
}

export function AppearanceMenu({ value, onChange }: AppearanceMenuProps) {
	const [open, setOpen] = useState(false);
	const menuId = useId();
	const containerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (!open) return;
		const closeOnEscape = (event: KeyboardEvent) => {
			if (event.key === "Escape") setOpen(false);
		};
		const closeOutside = (event: PointerEvent) => {
			if (!containerRef.current?.contains(event.target as Node)) setOpen(false);
		};
		document.addEventListener("keydown", closeOnEscape);
		document.addEventListener("pointerdown", closeOutside);
		return () => {
			document.removeEventListener("keydown", closeOnEscape);
			document.removeEventListener("pointerdown", closeOutside);
		};
	}, [open]);

	return (
		<div className={styles.appearance} ref={containerRef}>
			<button
				aria-controls={menuId}
				aria-expanded={open}
				aria-haspopup="dialog"
				className={styles.iconButton}
				onClick={() => setOpen((current) => !current)}
				type="button"
			>
				<SunMoon aria-hidden="true" size={19} strokeWidth={1.7} />
				<span className={styles.visuallyHidden}>Appearance</span>
			</button>
			{open ? (
				<div
					aria-label="Appearance settings"
					className={styles.appearanceMenu}
					id={menuId}
					role="dialog"
				>
					<div className={styles.menuTitle}>Appearance</div>
					<div
						aria-label="Appearance"
						className={styles.radioGroup}
						role="radiogroup"
					>
						{appearances.map((appearance) => (
							<label className={styles.radioOption} key={appearance.value}>
								<span>{appearance.label}</span>
								<input
									checked={value === appearance.value}
									name="appearance"
									onChange={() => {
										onChange(appearance.value);
										setOpen(false);
									}}
									type="radio"
									value={appearance.value}
								/>
							</label>
						))}
					</div>
				</div>
			) : null}
		</div>
	);
}
