import { describe, expect, it } from "vitest";
import { createInMemoryPhotoService } from "./inMemoryPhotoService";

describe("PhotoService contract", () => {
	it("persists a selected source and appearance for the adapter lifetime", async () => {
		const service = createInMemoryPhotoService({
			selectedFolderName: "Iceland 2025",
		});
		expect((await service.getBootstrapState()).activeSource).toBeNull();

		const chosen = await service.chooseFolder();
		expect(chosen.kind).toBe("selected");
		if (chosen.kind !== "selected") throw new Error("expected selected folder");
		expect(chosen.state.activeSource?.displayName).toBe("Iceland 2025");

		const updated = await service.updateAppearance("dark");
		expect(updated.settings.appearance).toBe("dark");
		expect((await service.getBootstrapState()).activeSource?.displayName).toBe(
			"Iceland 2025",
		);
	});

	it("represents picker cancellation without throwing", async () => {
		const service = createInMemoryPhotoService({ cancelFolderPicker: true });
		await expect(service.chooseFolder()).resolves.toEqual({
			kind: "cancelled",
		});
	});
});
