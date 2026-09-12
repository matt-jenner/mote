import { describe, expect, it } from "vitest";
import {
	addPickReference,
	clearPickReferences,
	removePickReference,
	restoreClearedPickReferences,
	type PickReference,
} from "./pickList";

const pick = (
	assetId: string,
	sourceFolderId = "folder-a",
	sourceLabel = "Family",
): PickReference => ({ assetId, sourceFolderId, sourceLabel });

describe("pick-list reference helpers", () => {
	it("keeps the first insertion order and ignores a duplicate asset ID", () => {
		const first = pick("asset-1", "folder-a", "Family");
		const result = addPickReference(
			addPickReference([], first),
			pick("asset-1", "folder-b", "Trips"),
		);

		expect(result).toEqual([first]);
	});

	it("removes only the matching asset ID", () => {
		expect(removePickReference([pick("asset-1"), pick("asset-2")], "asset-1")).toEqual([
			pick("asset-2"),
		]);
	});

	it("clears every reference", () => {
		expect(clearPickReferences([pick("asset-1")])).toEqual([]);
	});

	it("restores cleared unique references before selections made after clear", () => {
		expect(
			restoreClearedPickReferences(
				[pick("asset-1"), pick("asset-2"), pick("asset-1", "old-folder", "Old")],
				[pick("asset-2", "new-folder", "New"), pick("asset-3")],
			),
		).toEqual([pick("asset-1"), pick("asset-2"), pick("asset-3")]);
	});
});
