const configuredAccent = /^#[0-9A-Fa-f]{6}$/;

function readableTextColor(accentColor: string): "#000000" | "#FFFFFF" {
	const channels = [1, 3, 5].map((offset) =>
		Number.parseInt(accentColor.slice(offset, offset + 2), 16),
	);
	const luminance = channels
		.map((channel) => {
			const value = channel / 255;
			return value <= 0.04045
				? value / 12.92
				: ((value + 0.055) / 1.055) ** 2.4;
		})
		.reduce(
			(total, channel, index) =>
				total + channel * ([0.2126, 0.7152, 0.0722][index] ?? 0),
			0,
		);
	const whiteContrast = 1.05 / (luminance + 0.05);
	const blackContrast = (luminance + 0.05) / 0.05;
	return whiteContrast >= blackContrast ? "#FFFFFF" : "#000000";
}

export function applyAccentColor(accentColor?: string): void {
	const root = document.documentElement;
	root.style.removeProperty("--accent-color");
	root.style.removeProperty("--accent-color-text");

	if (accentColor === "system") {
		root.dataset.accent = "system";
		return;
	}
	if (accentColor && configuredAccent.test(accentColor)) {
		root.dataset.accent = "custom";
		root.style.setProperty("--accent-color", accentColor);
		root.style.setProperty(
			"--accent-color-text",
			readableTextColor(accentColor),
		);
		return;
	}
	delete root.dataset.accent;
}
