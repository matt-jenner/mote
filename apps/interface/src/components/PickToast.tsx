import type { PickToastAction, PickToastState } from "../picks/usePickList";
import styles from "../styles/appShell.module.css";

export async function consumePickToastAction(
	action: PickToastAction,
): Promise<void> {
	try {
		await action.run();
	} catch {}
}

export function PickToast({
	toast,
	onViewPicks,
}: {
	toast: PickToastState | null;
	onViewPicks?: () => void;
}) {
	if (!toast) return null;
	const action =
		toast.action ??
		(toast.message === "Added to picks" && onViewPicks
			? { label: "View", run: onViewPicks }
			: undefined);
	return (
		<div className={styles.pickToast} data-toast-id={toast.id}>
			<span>{toast.message}</span>
			{action ? (
				<button
					onClick={() => void consumePickToastAction(action)}
					type="button"
				>
					{action.label}
				</button>
			) : null}
		</div>
	);
}
