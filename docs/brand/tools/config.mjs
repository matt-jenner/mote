import path from "node:path";
import { fileURLToPath } from "node:url";

const toolDir = path.dirname(fileURLToPath(import.meta.url));
const brandDir = path.resolve(toolDir, "..");

export const BRAND = Object.freeze({
	name: "Mote",
	tagline: "A simple space for your photos.",
	colors: Object.freeze({
		green: "#45A06B",
		graphite: "#171A1F",
		white: "#F7F8FA",
		grey: "#B9C1C9",
		darkTile: "#22272D",
	}),
	wordmark: Object.freeze({ family: "Fredoka", weight: 400, width: 96 }),
});

export const PLATFORM_SIZES = Object.freeze({
	windows: Object.freeze([16, 24, 32, 48, 64, 128, 256]),
	linux: Object.freeze([16, 24, 32, 48, 64, 128, 256, 512]),
	web: Object.freeze([16, 32, 48, 180, 192, 512]),
	macos: Object.freeze([16, 32, 128, 256, 512, 1024]),
});

export const PATHS = Object.freeze({
	brandDir,
	sourceFont: path.resolve("docs/brand/fonts/Fredoka-Variable.ttf"),
	sourceLicense: path.resolve("docs/brand/fonts/OFL.txt"),
});

export function expectedOutputs() {
	const windows = PLATFORM_SIZES.windows.map(
		(size) => `docs/brand/icons/windows/png/mote-${size}.png`,
	);
	const linux = PLATFORM_SIZES.linux.map(
		(size) => `docs/brand/icons/linux/hicolor/${size}x${size}/apps/mote.png`,
	);
	const iconset = [
		"icon_16x16.png",
		"icon_16x16@2x.png",
		"icon_32x32.png",
		"icon_32x32@2x.png",
		"icon_128x128.png",
		"icon_128x128@2x.png",
		"icon_256x256.png",
		"icon_256x256@2x.png",
		"icon_512x512.png",
		"icon_512x512@2x.png",
	].map((filename) => `docs/brand/icons/macos/Mote.iconset/${filename}`);
	return [
		"docs/brand/source/mote-symbol-master.svg",
		"docs/brand/source/mote-wordmark-outlined.svg",
		"docs/brand/source/mote-lockup-master.svg",
		"docs/brand/fonts/Fredoka-Variable.ttf",
		"docs/brand/fonts/Fredoka-Variable.woff2",
		"docs/brand/fonts/OFL.txt",
		"docs/brand/icons/macos/Mote.icns",
		"docs/brand/icons/macos/icon-1024.png",
		"docs/brand/icons/macos/icon-1024-dark.png",
		"docs/brand/icons/windows/Mote.ico",
		"docs/brand/icons/linux/hicolor/scalable/apps/mote.svg",
		"docs/brand/icons/linux/hicolor/scalable/apps/mote-symbolic.svg",
		"docs/brand/icons/linux/hicolor/scalable/apps/mote-symbolic-dark.svg",
		"docs/brand/icons/web/favicon.svg",
		"docs/brand/icons/web/favicon.ico",
		"docs/brand/icons/web/icon-192.png",
		"docs/brand/icons/web/icon-512.png",
		"docs/brand/icons/web/maskable-192.png",
		"docs/brand/icons/web/maskable-512.png",
		"docs/brand/icons/web/apple-touch-icon-180.png",
		"docs/brand/icons/web/service-monochrome.svg",
		"docs/brand/icons/web/manifest-icons.json",
		"docs/brand/print/mote-lockup-light.svg",
		"docs/brand/print/mote-lockup-dark.svg",
		"docs/brand/print/mote-lockup-light-3000.png",
		"docs/brand/print/mote-lockup-dark-3000.png",
		"docs/brand/print/mote-brand-sheet-a4.pdf",
		"docs/brand/previews/mote-brand-sheet-a4-1.png",
		"docs/brand/previews/mote-asset-contact-sheet.png",
		"docs/brand/previews/mote-small-size-check.png",
		...iconset,
		...windows,
		...linux,
	];
}
