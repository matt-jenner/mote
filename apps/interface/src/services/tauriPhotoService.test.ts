import { describe, expect, it } from "vitest";
import { PhotoServiceError } from "./photoService";
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

	it("converts a known native command failure to PhotoServiceError", async () => {
		const invoke: InvokeCommand = async () => {
			throw {
				code: "folderNotDirectory",
				message: "Choose a folder, not a file.",
			};
		};
		const service = createTauriPhotoService(invoke);

		const error = await service
			.chooseFolder()
			.catch((reason: unknown) => reason);

		expect(error).toBeInstanceOf(PhotoServiceError);
		expect(error).toMatchObject({
			code: "folderNotDirectory",
			message: "Choose a folder, not a file.",
		});
	});

	it("maps unknown native rejections to a fixed path-free internal error", async () => {
		const nativeDetail = "SQLite failed near /Users/private/Photo Library";
		for (const rejection of [
			new Error(nativeDetail),
			{ code: "unexpectedNativeFailure", message: nativeDetail },
			{ code: "folderNotDirectory", message: nativeDetail },
		]) {
			const invoke: InvokeCommand = async () => {
				throw rejection;
			};
			const service = createTauriPhotoService(invoke);

			const error = await service
				.getBootstrapState()
				.catch((reason: unknown) => reason);

			expect(error).toBeInstanceOf(PhotoServiceError);
			expect(error).toMatchObject({
				code: "internal",
				message: "Photo Viewer could not complete that request.",
			});
			expect((error as Error).message).not.toContain("/Users/private");
		}
	});
});
