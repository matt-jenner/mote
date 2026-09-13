import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import "../styles/tokens.css";
import "../styles/global.css";
import { WallToolbar } from "./WallToolbar";

const labels = ["Include subfolders", "Oldest first", "Newest first"] as const;

function tooltipStyle(control: Element) {
	return getComputedStyle(control, "::after");
}

describe("desktop wall toolbar", () => {
	beforeEach(async () => {
		await page.viewport(1024, 768);
	});

	afterEach(() => {
		document.documentElement.dataset.theme = "system";
	});

	it("uses accessible icon-only controls with hover and focus tooltips", async () => {
		const screen = await render(
			<WallToolbar
				direction="oldestFirst"
				galleryScope="includeSubfolders"
				onDirectionChange={vi.fn()}
				onGalleryScopeChange={vi.fn()}
				status="42 photos"
			/>,
		);
		const controls = labels.map((label) =>
			screen.getByRole("button", { name: label, exact: true }),
		);

		for (const [index, control] of controls.entries()) {
			const element = control.element();
			expect(element.textContent).toBe("");
			expect(element.children).toHaveLength(1);
			expect(element.firstElementChild?.tagName.toLowerCase()).toBe("svg");
			expect(element).toHaveAttribute("aria-label", labels[index]);
			expect(element).toHaveAttribute("data-tooltip", labels[index]);
			const bounds = element.getBoundingClientRect();
			expect(bounds.width).toBeGreaterThanOrEqual(44);
			expect(bounds.height).toBeGreaterThanOrEqual(44);

			await control.hover();
			expect(tooltipStyle(element).content).toBe(`"${labels[index]}"`);
			expect(tooltipStyle(element).visibility).toBe("visible");
		}

		expect(controls[0]?.element()).toHaveAttribute("aria-pressed", "true");
		expect(controls[1]?.element()).toHaveAttribute("aria-pressed", "true");
		expect(controls[2]?.element()).toHaveAttribute("aria-pressed", "false");

		for (const control of controls) {
			control.element().focus();
			expect(document.activeElement).toBe(control.element());
			expect(control.element().matches(":focus-visible")).toBe(true);
			expect(tooltipStyle(control.element()).visibility).toBe("visible");
		}
	});

	it.each([900, 1024, 1440])(
		"keeps view controls on one row at %ipx",
		async (width) => {
			await page.viewport(width, 768);
			const screen = await render(
				<WallToolbar
					direction="newestFirst"
					galleryScope="includeSubfolders"
					onDirectionChange={vi.fn()}
					onGalleryScopeChange={vi.fn()}
					onRetry={vi.fn()}
					progress={{ busy: true, max: 100, status: "indexing", value: 42 }}
					retryable
					status="Indexing a folder with a deliberately long but concise progress description that must truncate before the view controls wrap"
				/>,
			);
			const controls = labels.map((label) =>
				screen.getByRole("button", { name: label, exact: true }).element(),
			);
			const tops = controls.map((control) =>
				Math.round(control.getBoundingClientRect().top),
			);

			expect(new Set(tops).size).toBe(1);
			if (width === 900)
				expect(
					screen.getByRole("status").element().scrollWidth,
				).toBeGreaterThan(screen.getByRole("status").element().clientWidth);
		},
	);
});
