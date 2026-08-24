import { describe, expect, it } from "vitest";
import {
	createTauriPhotoService,
	type InvokeCommand,
} from "./tauriPhotoService";

describe("Tauri PhotoService", () => {
	it("uses only the three checkpoint commands", async () => {
		const responses: unknown[] = [
			{ settings: { appearance: "system" }, activeSource: null },
			{ kind: "cancelled" },
			{ settings: { appearance: "dark" }, activeSource: null },
		];
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			calls.push([command, args]);
			return responses.shift() as T;
		};
		const service = createTauriPhotoService(invoke);

		await service.getBootstrapState();
		await service.chooseFolder();
		await service.updateAppearance("dark");

		expect(calls).toEqual([
			["get_bootstrap_state", undefined],
			["choose_folder", undefined],
			["update_appearance", { appearance: "dark" }],
		]);
	});
});
