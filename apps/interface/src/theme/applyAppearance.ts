import type { Appearance } from "../services/photoService";

export function applyAppearance(appearance: Appearance): void {
	document.documentElement.dataset.theme = appearance;
	document.documentElement.style.colorScheme =
		appearance === "system" ? "light dark" : appearance;
}
