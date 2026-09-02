import assert from "node:assert/strict";
import test from "node:test";
import { horizontalLockupSvg, symbolSvg, wordmarkSvg } from "./svg.mjs";

test("full-colour symbol retains every frame and the centre square", () => {
  const svg = symbolSvg({ mode: "light", tile: false, size: 1024 });
  assert.match(svg, /data-layer="rear"/);
  assert.match(svg, /data-layer="middle"/);
  assert.match(svg, /data-layer="front"/);
  assert.match(svg, /data-layer="centre"/);
  assert.match(svg, /#45A06B/i);
  assert.match(svg, /#171A1F/i);
  assert.match(svg, /#F7F8FA/i);
});

test("dark symbol reverses the rear frame and retains Mote green", () => {
  const svg = symbolSvg({ mode: "dark", tile: false, size: 1024 });
  assert.match(svg, /data-layer="rear"[^>]+stroke="#F7F8FA"/i);
  assert.match(svg, /data-layer="front"[^>]+stroke="#45A06B"/i);
});

test("maskable app icon keeps essential artwork inside the safe zone", () => {
  const svg = symbolSvg({ mode: "light", tile: true, maskable: true, size: 1024 });
  assert.match(svg, /data-safe-zone="205 205 614 614"/);
  assert.match(svg, /data-artwork-bounds="205 205 614 614"/);
});

test("portable wordmarks and lockups use outlines instead of live text", async () => {
  const wordmark = await wordmarkSvg({ mode: "light" });
  const lockup = await horizontalLockupSvg({ mode: "dark" });
  for (const svg of [wordmark, lockup]) {
    assert.match(svg, /<path\b/);
    assert.doesNotMatch(svg, /<text\b/i);
  }
});

test("monochrome symbol keeps all layers in one requested colour", () => {
  const svg = symbolSvg({ mode: "light", tile: false, monochrome: true, size: 1024 });
  const fillsAndStrokes = [...svg.matchAll(/(?:fill|stroke)="(#[0-9A-F]{6})"/gi)].map(
    ([, colour]) => colour.toUpperCase(),
  );
  assert.ok(fillsAndStrokes.length >= 4);
  assert.deepEqual(new Set(fillsAndStrokes), new Set(["#171A1F"]));
});
