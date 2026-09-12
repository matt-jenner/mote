import type { PickToastState } from "../picks/usePickList";
import styles from "../styles/appShell.module.css";

export function PickToast({ toast }: { toast: PickToastState | null }) {
	if (!toast) return null;
	return (
		<div className={styles.pickToast} data-toast-id={toast.id}>
			<span>{toast.message}</span>
			{toast.action ? (
				<button onClick={toast.action.run} type="button">
					{toast.action.label}
				</button>
			) : null}
		</div>
	);
}
