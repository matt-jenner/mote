import assert from "node:assert/strict";
import test from "node:test";
import { BRAND, PLATFORM_SIZES, expectedOutputs } from "./config.mjs";

test("locks the approved Mote identity", () => {
  assert.deepEqual(BRAND.colors, {
    green: "#45A06B",
    graphite: "#171A1F",
    white: "#F7F8FA",
    grey: "#B9C1C9",
    darkTile: "#22272D",
  });
  assert.equal(BRAND.name, "Mote");
  assert.equal(BRAND.tagline, "A simple space for your photos.");
  assert.deepEqual(BRAND.wordmark, { family: "Fredoka", weight: 400, width: 96 });
});

test("declares complete platform size sets", () => {
  assert.deepEqual(PLATFORM_SIZES.windows, [16, 24, 32, 48, 64, 128, 256]);
  assert.deepEqual(PLATFORM_SIZES.linux, [16, 24, 32, 48, 64, 128, 256, 512]);
  assert.deepEqual(PLATFORM_SIZES.web, [16, 32, 48, 180, 192, 512]);
  assert.deepEqual(PLATFORM_SIZES.macos, [16, 32, 128, 256, 512, 1024]);
});

test("does not target shipping application icon paths", () => {
  for (const output of expectedOutputs()) {
    assert.match(output, /^docs\/brand\//);
    assert.doesNotMatch(output, /apps\/desktop\/src-tauri\/icons/);
  }
});
