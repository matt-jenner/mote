# Mote Brand Asset Pack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a documented, reproducible Mote brand asset pack containing source vectors, light and dark variants, desktop and PWA icon packages, licensed Fredoka files, print exports, and visual verification sheets.

**Architecture:** Human-readable source SVGs and one configuration module define the mark, palette, wordmark settings, output names, and platform sizes. A deterministic Node generator uses Fontkit, Sharp, and png-to-ico to outline the wordmark and render vector sources into platform packages through a temporary staging directory. A small Python program creates and validates the printable PDF using the generated assets.

**Tech Stack:** SVG, Node.js 24, Node test runner, Fontkit, Sharp, png-to-ico, fast-xml-parser, Python 3, FontTools with Brotli, Pillow, ReportLab, pypdf, macOS `iconutil`, and Poppler.

**Spec:** `docs/superpowers/specs/2026-09-02-mote-brand-asset-pack-design.md`

## Global Constraints

- Product name is `Mote`.
- Highlight is `#45A06B`, graphite is `#171A1F`, cool white is `#F7F8FA`, and mineral grey is `#B9C1C9`.
- The wordmark uses Fredoka at weight 400 and width axis 96.
- The brand line is `A simple space for your photos.`
- The same frame-stack symbol geometry is used on macOS, Windows, Linux, and the hosted PWA.
- Platform changes are limited to canvas, padding, masking, packaging, and tiny-size optical correction.
- Light and dark exports are first-class outputs.
- App icons contain no text.
- Portable wordmark and lockup SVGs contain outlined paths and no `<text>` elements.
- The original Fredoka variable font name and axes remain unchanged.
- `OFL.txt` ships beside both font formats.
- Generation writes only inside `docs/brand/` and must not replace `apps/desktop/src-tauri/icons/`.
- Generated assets are staged before they replace the final generated directories.
- Existing `docs/brand/design-qa.md`, `fredoka-source-comparison.jpg`, and `fredoka-test-page-viewport.jpg` remain as design-history files.

## File map

- `package.json`: add repeatable brand generation and validation commands.
- `package-lock.json`: pin asset-tool dependencies.
- `docs/brand/tools/config.mjs`: palette, wordmark axes, source paths, platform sizes, and expected output paths.
- `docs/brand/tools/config.test.mjs`: validate immutable brand constants and size sets.
- `docs/brand/tools/svg.mjs`: create symbol, app-tile, wordmark, and lockup SVG strings.
- `docs/brand/tools/svg.test.mjs`: verify SVG structure, variants, path-only wordmarks, and safe-zone geometry.
- `docs/brand/tools/generate-assets.mjs`: stage and generate SVG, PNG, ICO, ICNS, Linux, web, contact-sheet, and print-raster outputs.
- `docs/brand/tools/generate-assets.test.mjs`: validate every generated file, raster dimension, ICO entry, manifest declaration, and font package.
- `docs/brand/tools/requirements.txt`: pin PDF and font-conversion dependencies.
- `docs/brand/tools/convert_font.py`: convert the original variable TTF to WOFF2 without changing names or axes.
- `docs/brand/tools/generate_brand_sheet.py`: create the A4 PDF from generated brand assets.
- `docs/brand/tools/validate_brand_sheet.py`: inspect PDF page size, page count, metadata, and extracted text.
- `docs/brand/source/*.svg`: editable master symbol, app tile, outlined wordmark, and master lockup.
- `docs/brand/svg/*.svg`: ready-to-use light, dark, monochrome, horizontal, and stacked exports.
- `docs/brand/fonts/*`: original Fredoka TTF, converted WOFF2, licence, and font usage notes.
- `docs/brand/icons/macos/*`: complete iconset, appearance sources, and packaged ICNS.
- `docs/brand/icons/windows/*`: individual PNG sizes and packaged ICO.
- `docs/brand/icons/linux/hicolor/*`: freedesktop-sized PNGs, scalable SVG, and symbolic variants.
- `docs/brand/icons/web/*`: favicon, touch, standard, maskable, monochrome, and manifest assets.
- `docs/brand/print/*`: vector lockups, 3000-pixel PNG lockups, and the A4 brand sheet.
- `docs/brand/previews/*`: contact sheet, small-size sheet, and rendered PDF preview.
- `docs/brand/README.md`: inventory, usage rules, integration examples, and source provenance.

---

### Task 1: Establish the generation contract

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Create: `docs/brand/tools/config.mjs`
- Create: `docs/brand/tools/config.test.mjs`
- Create: `docs/brand/tools/requirements.txt`

**Interfaces:**
- Consumes: approved constants from the design specification.
- Produces: `BRAND`, `PLATFORM_SIZES`, `PATHS`, and `expectedOutputs()` for every later task.

- [ ] **Step 1: Install the Node generation dependencies**

Run:

```bash
npm install --save-dev fast-xml-parser fontkit png-to-ico sharp
```

Expected: `package.json` and `package-lock.json` contain the four pinned development dependencies.

- [ ] **Step 2: Add repository commands**

Add these scripts to the root `package.json`:

```json
"brand:generate": "node docs/brand/tools/generate-assets.mjs",
"brand:test": "node --test docs/brand/tools/*.test.mjs"
```

- [ ] **Step 3: Write the failing configuration test**

Create `docs/brand/tools/config.test.mjs`:

```js
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
```

- [ ] **Step 4: Run the test and confirm the missing-module failure**

Run:

```bash
node --test docs/brand/tools/config.test.mjs
```

Expected: FAIL because `config.mjs` does not exist.

- [ ] **Step 5: Implement the configuration module**

Create `docs/brand/tools/config.mjs` with this public shape:

```js
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
  sourceFont: path.resolve("apps/interface/public/fonts/Fredoka-Variable.ttf"),
  sourceLicense: path.resolve("apps/interface/public/fonts/Fredoka-OFL.txt"),
});

export function expectedOutputs() {
  const windows = PLATFORM_SIZES.windows.map((size) => `docs/brand/icons/windows/png/mote-${size}.png`);
  const linux = PLATFORM_SIZES.linux.map((size) => `docs/brand/icons/linux/hicolor/${size}x${size}/apps/mote.png`);
  return [
    "docs/brand/source/mote-symbol-master.svg",
    "docs/brand/source/mote-wordmark-outlined.svg",
    "docs/brand/source/mote-lockup-master.svg",
    "docs/brand/icons/macos/Mote.icns",
    "docs/brand/icons/windows/Mote.ico",
    "docs/brand/icons/web/favicon.svg",
    "docs/brand/icons/web/favicon.ico",
    "docs/brand/icons/web/icon-192.png",
    "docs/brand/icons/web/icon-512.png",
    "docs/brand/icons/web/maskable-192.png",
    "docs/brand/icons/web/maskable-512.png",
    "docs/brand/print/mote-brand-sheet-a4.pdf",
    ...windows,
    ...linux,
  ];
}
```

- [ ] **Step 6: Pin Python dependencies**

Create `docs/brand/tools/requirements.txt`:

```text
fonttools[woff]>=4.59,<5
Pillow>=11,<13
reportlab>=4.4,<5
pypdf>=6,<7
```

- [ ] **Step 7: Run the configuration test**

Run:

```bash
npm run brand:test -- --test-name-pattern="approved Mote identity|platform size sets|shipping application"
```

Expected: 3 passing tests.

- [ ] **Step 8: Commit the generation contract**

```bash
git add package.json package-lock.json docs/brand/tools/config.mjs docs/brand/tools/config.test.mjs docs/brand/tools/requirements.txt
git commit -m "build: define Mote asset generation contract"
```

---

### Task 2: Create the vector sources and outlined wordmark

**Files:**
- Create: `docs/brand/tools/svg.mjs`
- Create: `docs/brand/tools/svg.test.mjs`
- Create: `docs/brand/source/mote-symbol-master.svg`
- Create: `docs/brand/source/mote-wordmark-outlined.svg`
- Create: `docs/brand/source/mote-lockup-master.svg`
- Create: `docs/brand/svg/mote-symbol-light.svg`
- Create: `docs/brand/svg/mote-symbol-dark.svg`
- Create: `docs/brand/svg/mote-symbol-monochrome-dark.svg`
- Create: `docs/brand/svg/mote-symbol-monochrome-light.svg`
- Create: `docs/brand/svg/mote-wordmark-dark.svg`
- Create: `docs/brand/svg/mote-wordmark-light.svg`
- Create: `docs/brand/svg/mote-lockup-horizontal-light.svg`
- Create: `docs/brand/svg/mote-lockup-horizontal-dark.svg`
- Create: `docs/brand/svg/mote-lockup-stacked-light.svg`
- Create: `docs/brand/svg/mote-lockup-stacked-dark.svg`

**Interfaces:**
- Consumes: `BRAND` and `PATHS` from `config.mjs`, plus the original Fredoka variable TTF.
- Produces: `symbolSvg(options)`, `wordmarkSvg(options)`, `horizontalLockupSvg(options)`, and `stackedLockupSvg(options)`.

- [ ] **Step 1: Write the failing SVG contract tests**

Create `docs/brand/tools/svg.test.mjs`:

```js
import assert from "node:assert/strict";
import test from "node:test";
import { horizontalLockupSvg, symbolSvg, wordmarkSvg } from "./svg.mjs";

test("full-colour symbol retains all four layers", () => {
  const svg = symbolSvg({ mode: "light", tile: false, size: 1024 });
  assert.match(svg, /data-layer="rear"/);
  assert.match(svg, /data-layer="middle"/);
  assert.match(svg, /data-layer="front"/);
  assert.match(svg, /data-layer="centre"/);
  assert.match(svg, /#45A06B/i);
  assert.match(svg, /#171A1F/i);
  assert.match(svg, /#F7F8FA/i);
});

test("dark symbol changes the rear frame but keeps Mote green", () => {
  const svg = symbolSvg({ mode: "dark", tile: false, size: 1024 });
  assert.match(svg, /data-layer="rear"[^>]+#F7F8FA/i);
  assert.match(svg, /data-layer="front"[^>]+#45A06B/i);
});

test("maskable app icon keeps essential artwork inside the 80 percent safe zone", () => {
  const svg = symbolSvg({ mode: "light", tile: true, maskable: true, size: 1024 });
  assert.match(svg, /data-safe-zone="205 205 614 614"/);
});

test("portable wordmarks and lockups use paths instead of live text", async () => {
  const wordmark = await wordmarkSvg({ mode: "light" });
  const lockup = await horizontalLockupSvg({ mode: "dark" });
  for (const svg of [wordmark, lockup]) {
    assert.match(svg, /<path\b/);
    assert.doesNotMatch(svg, /<text\b/i);
  }
});
```

- [ ] **Step 2: Run the SVG tests and confirm they fail**

Run:

```bash
node --test docs/brand/tools/svg.test.mjs
```

Expected: FAIL because `svg.mjs` does not exist.

- [ ] **Step 3: Implement the symbol geometry**

Create `docs/brand/tools/svg.mjs`. Use one 1024-unit coordinate system for every symbol:

```js
import fs from "node:fs/promises";
import fontkit from "fontkit";
import { BRAND, PATHS } from "./config.mjs";

const escapeXml = (value) => value.replaceAll("&", "&amp;").replaceAll('"', "&quot;");

function symbolLayers({ mode, small = false }) {
  const stroke = small ? 112 : 96;
  const centre = small ? 104 : 82;
  const rear = mode === "dark" ? BRAND.colors.white : BRAND.colors.graphite;
  return `
    <g fill="none" stroke-linejoin="round">
      <rect data-layer="rear" x="372" y="160" width="360" height="520" rx="42" stroke="${rear}" stroke-width="${stroke}"/>
      <rect data-layer="middle" x="292" y="280" width="416" height="424" rx="42" stroke="${BRAND.colors.white}" stroke-width="${stroke}"/>
      <rect data-layer="front" x="180" y="344" width="448" height="448" rx="42" stroke="${BRAND.colors.green}" stroke-width="${stroke}"/>
    </g>
    <rect data-layer="centre" x="${512 - centre / 2}" y="${512 - centre / 2}" width="${centre}" height="${centre}" rx="12" fill="${BRAND.colors.green}"/>`;
}

export function symbolSvg({ mode, tile = false, maskable = false, size = 1024 }) {
  const small = size <= 24;
  const tileFill = mode === "dark" ? BRAND.colors.darkTile : BRAND.colors.graphite;
  const tileNode = tile
    ? `<rect data-layer="tile" x="64" y="64" width="896" height="896" rx="214" fill="${tileFill}"/>`
    : "";
  const safeZone = maskable ? '<rect data-safe-zone="205 205 614 614" x="205" y="205" width="614" height="614" fill="none"/>' : "";
  const scale = maskable ? 0.72 : 0.82;
  const offset = (1024 - 1024 * scale) / 2;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="${size}" height="${size}">
    ${tileNode}
    <g transform="translate(${offset} ${offset}) scale(${scale})">${symbolLayers({ mode, small })}</g>
    ${safeZone}
  </svg>`;
}
```

Use the same module to provide monochrome output by replacing every visible layer colour with graphite or white. Do not remove frame layers.

- [ ] **Step 4: Implement Fredoka outline extraction**

In `svg.mjs`, open the original font and use its variation and layout APIs:

```js
let fredoka;

async function fredokaFont() {
  if (!fredoka) {
    const bytes = await fs.readFile(PATHS.sourceFont);
    const collection = fontkit.create(bytes);
    const base = "fonts" in collection ? collection.fonts[0] : collection;
    fredoka = base.getVariation({ wght: BRAND.wordmark.weight, wdth: BRAND.wordmark.width });
  }
  return fredoka;
}

async function outlinedRun(text) {
  const font = await fredokaFont();
  const run = font.layout(text);
  let x = 0;
  const paths = run.glyphs.map((glyph, index) => {
    const path = glyph.path.toSVG();
    const node = path.replace("<path", `<path transform="translate(${x} 0) scale(1 -1)"`);
    x += run.positions[index].xAdvance;
    return node;
  });
  return { paths: paths.join(""), width: x, ascent: font.ascent, descent: font.descent };
}

export async function wordmarkSvg({ mode }) {
  const run = await outlinedRun(BRAND.name);
  const fill = mode === "dark" ? BRAND.colors.white : BRAND.colors.graphite;
  const height = run.ascent - run.descent;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 ${-run.ascent} ${run.width} ${height}" role="img" aria-label="${escapeXml(BRAND.name)}"><g fill="${fill}">${run.paths}</g></svg>`;
}
```

Implement horizontal and stacked lockups by embedding the symbol group and the same outlined wordmark paths into a shared view box. Horizontal clear space equals the central-square width. Stacked clear space equals twice that width.

- [ ] **Step 5: Run the SVG tests**

Run:

```bash
node --test docs/brand/tools/svg.test.mjs
```

Expected: all four tests pass.

- [ ] **Step 6: Export the source and ready-to-use SVG files**

Add an `exportSvgSources(stageDir)` function to `svg.mjs` that writes every file listed for this task. Call it through a temporary one-off Node expression, then inspect the output names:

```bash
node -e 'import("./docs/brand/tools/svg.mjs").then((m) => m.exportSvgSources("docs/brand"))'
find docs/brand/source docs/brand/svg -type f -name '*.svg' | sort
```

Expected: 3 source SVGs and 10 distribution SVGs.

- [ ] **Step 7: Commit the vector system**

```bash
git add docs/brand/source docs/brand/svg docs/brand/tools/svg.mjs docs/brand/tools/svg.test.mjs
git commit -m "feat: add Mote vector identity system"
```

---

### Task 3: Package fonts and generate platform icons

**Files:**
- Create: `docs/brand/tools/convert_font.py`
- Create: `docs/brand/tools/generate-assets.mjs`
- Create: `docs/brand/tools/generate-assets.test.mjs`
- Create: generated files under `docs/brand/fonts/` and `docs/brand/icons/`

**Interfaces:**
- Consumes: `symbolSvg`, `BRAND`, `PLATFORM_SIZES`, `PATHS`, the Fredoka TTF, and OFL text.
- Produces: `generateAll({ destination, keepStage })`, font package, ICNS, ICO, Linux tree, and PWA files.

- [ ] **Step 1: Write the failing generated-asset tests**

Create `docs/brand/tools/generate-assets.test.mjs`:

```js
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { XMLValidator } from "fast-xml-parser";
import sharp from "sharp";
import { PLATFORM_SIZES, expectedOutputs } from "./config.mjs";

const root = path.resolve("docs/brand");

test("every declared output exists", async () => {
  for (const output of expectedOutputs()) await fs.access(path.resolve(output));
});

test("Windows and Linux PNG dimensions match their filenames", async () => {
  const files = [
    ...PLATFORM_SIZES.windows.map((size) => [`icons/windows/png/mote-${size}.png`, size]),
    ...PLATFORM_SIZES.linux.map((size) => [`icons/linux/hicolor/${size}x${size}/apps/mote.png`, size]),
  ];
  for (const [relative, size] of files) {
    const metadata = await sharp(path.join(root, relative)).metadata();
    assert.equal(metadata.width, size);
    assert.equal(metadata.height, size);
    assert.equal(metadata.hasAlpha, true);
  }
});

test("every SVG is well-formed XML and portable wordmarks contain no live text", async () => {
  const queue = [root];
  const svgFiles = [];
  while (queue.length) {
    const directory = queue.pop();
    for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) queue.push(filename);
      else if (entry.name.endsWith(".svg")) svgFiles.push(filename);
    }
  }
  assert.ok(svgFiles.length >= 15);
  for (const filename of svgFiles) {
    const svg = await fs.readFile(filename, "utf8");
    assert.equal(XMLValidator.validate(svg), true, filename);
    if (/wordmark|lockup/.test(filename)) {
      assert.match(svg, /<path\b/);
      assert.doesNotMatch(svg, /<text\b/i);
    }
  }
});

test("Windows ICO contains a readable PNG entry for every required size", async () => {
  const ico = await fs.readFile(path.join(root, "icons/windows/Mote.ico"));
  assert.equal(ico.readUInt16LE(0), 0);
  assert.equal(ico.readUInt16LE(2), 1);
  const count = ico.readUInt16LE(4);
  const sizes = [];
  for (let index = 0; index < count; index += 1) {
    const entry = 6 + index * 16;
    const width = ico[entry] || 256;
    const height = ico[entry + 1] || 256;
    assert.equal(width, height);
    assert.equal(ico.readUInt16LE(entry + 6), 32);
    const length = ico.readUInt32LE(entry + 8);
    const offset = ico.readUInt32LE(entry + 12);
    const metadata = await sharp(ico.subarray(offset, offset + length)).metadata();
    assert.equal(metadata.width, width);
    assert.equal(metadata.height, height);
    sizes.push(width);
  }
  assert.deepEqual(sizes.sort((a, b) => a - b), PLATFORM_SIZES.windows);
});

test("PWA manifest declares existing standard, maskable, and monochrome assets", async () => {
  const manifestPath = path.join(root, "icons/web/manifest-icons.json");
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  assert.deepEqual(new Set(manifest.icons.map((icon) => icon.purpose)), new Set(["any", "maskable", "monochrome"]));
  for (const icon of manifest.icons) {
    const filename = path.join(root, "icons/web", icon.src);
    await fs.access(filename);
    if (icon.type === "image/png") {
      const [declared] = icon.sizes.split("x").map(Number);
      const metadata = await sharp(filename).metadata();
      assert.equal(metadata.width, declared);
      assert.equal(metadata.height, declared);
    } else {
      assert.equal(icon.type, "image/svg+xml");
      assert.equal(icon.sizes, "any");
    }
  }
});

test("font package includes original names, both axes, and the OFL", async () => {
  const fontkit = (await import("fontkit")).default;
  for (const filename of ["Fredoka-Variable.ttf", "Fredoka-Variable.woff2"]) {
    const font = fontkit.openSync(path.join(root, "fonts", filename));
    assert.equal(font.familyName, "Fredoka");
    assert.ok(font.variationAxes.wght);
    assert.ok(font.variationAxes.wdth);
  }
  const license = await fs.readFile(path.join(root, "fonts/OFL.txt"), "utf8");
  assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/);
});
```

- [ ] **Step 2: Run the test and confirm missing-output failures**

Run:

```bash
npm run brand:test -- --test-name-pattern="declared output|PNG dimensions|PWA manifest|font package"
```

Expected: FAIL because the generated platform packages do not exist.

- [ ] **Step 3: Implement lossless variable-font conversion**

Create `docs/brand/tools/convert_font.py`:

```python
from pathlib import Path
import sys
from fontTools.ttLib import TTFont

source = Path(sys.argv[1])
destination = Path(sys.argv[2])
font = TTFont(source)
font.flavor = "woff2"
destination.parent.mkdir(parents=True, exist_ok=True)
font.save(destination)
```

The generator must run this with a Python environment containing `fonttools[woff]`. It then reopens both fonts through Fontkit before accepting the staged package.

- [ ] **Step 4: Implement staged raster generation**

Create `docs/brand/tools/generate-assets.mjs` with these helpers:

```js
import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import pngToIco from "png-to-ico";
import sharp from "sharp";
import { BRAND, PATHS, PLATFORM_SIZES } from "./config.mjs";
import { exportSvgSources, symbolSvg } from "./svg.mjs";

const execFileAsync = promisify(execFile);

async function renderPng(svg, size, output) {
  await fs.mkdir(path.dirname(output), { recursive: true });
  await sharp(Buffer.from(svg)).resize(size, size).png({ compressionLevel: 9 }).toFile(output);
}

async function writeIco(pngPaths, output) {
  const buffers = await Promise.all(pngPaths.map((filename) => fs.readFile(filename)));
  await fs.writeFile(output, await pngToIco(buffers));
}

async function copyFontPackage(stage) {
  const fontDir = path.join(stage, "fonts");
  await fs.mkdir(fontDir, { recursive: true });
  await fs.copyFile(PATHS.sourceFont, path.join(fontDir, "Fredoka-Variable.ttf"));
  await fs.copyFile(PATHS.sourceLicense, path.join(fontDir, "OFL.txt"));
  await execFileAsync(process.env.BRAND_PYTHON ?? "python3", [
    path.resolve("docs/brand/tools/convert_font.py"),
    path.join(fontDir, "Fredoka-Variable.ttf"),
    path.join(fontDir, "Fredoka-Variable.woff2"),
  ]);
}
```

`generateAll()` creates a temporary directory with `fs.mkdtemp(path.join(os.tmpdir(), "mote-brand-"))`, calls `exportSvgSources(stage)`, writes every output, validates the stage, and copies completed generated directories into `docs/brand/`. On failure it removes the stage and leaves current outputs unchanged.

- [ ] **Step 5: Generate the macOS package**

Render `icon-1024.png` from the light full-colour tile and `icon-1024-dark.png` from the dark tile. Create `Mote.iconset` with these exact mappings:

```js
const iconsetEntries = [
  [16, "icon_16x16.png"],
  [32, "icon_16x16@2x.png"],
  [32, "icon_32x32.png"],
  [64, "icon_32x32@2x.png"],
  [128, "icon_128x128.png"],
  [256, "icon_128x128@2x.png"],
  [256, "icon_256x256.png"],
  [512, "icon_256x256@2x.png"],
  [512, "icon_512x512.png"],
  [1024, "icon_512x512@2x.png"],
];
```

Run `iconutil -c icns Mote.iconset -o Mote.icns`. After packaging, run `iconutil -c iconset Mote.icns -o verification.iconset` and confirm that all ten names can be reopened.

- [ ] **Step 6: Generate Windows and Linux packages**

For Windows, render the graphite-tile symbol at 16, 24, 32, 48, 64, 128, and 256 pixels. Use the small optical geometry at 16 and 24 pixels. Pass every PNG buffer to `writeIco()` in ascending size order.

For Linux, render the same artwork into `icons/linux/hicolor/{size}x{size}/apps/mote.png` at 16, 24, 32, 48, 64, 128, 256, and 512 pixels. Write the full-colour vector to `hicolor/scalable/apps/mote.svg`, graphite monochrome to `mote-symbolic.svg`, and white monochrome to `mote-symbolic-dark.svg`.

- [ ] **Step 7: Generate web and PWA assets**

Generate:

```json
{
  "icons": [
    { "src": "icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "maskable-192.png", "sizes": "192x192", "type": "image/png", "purpose": "maskable" },
    { "src": "maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" },
    { "src": "service-monochrome.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "monochrome" }
  ]
}
```

Render `apple-touch-icon-180.png` with an opaque graphite tile. Build `favicon.ico` from 16, 32, and 48 pixel PNG buffers. Write `favicon.svg` with an embedded `prefers-color-scheme: dark` rule that swaps only the theme-dependent rear frame and canvas values.

- [ ] **Step 8: Run the generator and asset tests**

Run:

```bash
python3 -m pip install -r docs/brand/tools/requirements.txt
BRAND_PYTHON=python3 npm run brand:generate
npm run brand:test
```

Expected: all configuration, SVG, font, platform-size, manifest, and file-existence tests pass.

- [ ] **Step 9: Commit platform and font assets**

```bash
git add docs/brand/fonts docs/brand/icons docs/brand/tools/convert_font.py docs/brand/tools/generate-assets.mjs docs/brand/tools/generate-assets.test.mjs
git commit -m "feat: package Mote fonts and platform icons"
```

---

### Task 4: Create printable and preview assets

**Files:**
- Create: `docs/brand/tools/generate_brand_sheet.py`
- Create: `docs/brand/tools/validate_brand_sheet.py`
- Create: `docs/brand/print/mote-lockup-light.svg`
- Create: `docs/brand/print/mote-lockup-dark.svg`
- Create: `docs/brand/print/mote-lockup-light-3000.png`
- Create: `docs/brand/print/mote-lockup-dark-3000.png`
- Create: `docs/brand/print/mote-brand-sheet-a4.pdf`
- Create: `docs/brand/previews/mote-brand-sheet-a4-1.png`
- Create: `docs/brand/previews/mote-asset-contact-sheet.png`
- Create: `docs/brand/previews/mote-small-size-check.png`
- Modify: `docs/brand/tools/generate-assets.mjs`

**Interfaces:**
- Consumes: approved source vectors and generated platform PNGs.
- Produces: print lockups, one A4 PDF, one rendered PDF preview, and two contact sheets.

- [ ] **Step 1: Mark the PDF authoring operation**

Immediately before creating the PDF for the first time, run exactly once:

```bash
node /Users/jennerm/.codex/plugins/cache/openai-primary-runtime/pdf/26.826.12353/skills/pdf/container_tools/mark_artifact_operation_started.mjs --operation-kind create --expected-output-count 1 --output-format pdf
```

Expected: exit code 0.

- [ ] **Step 2: Write the failing PDF validator**

Create `docs/brand/tools/validate_brand_sheet.py`:

```python
from pathlib import Path
from pypdf import PdfReader

pdf = Path("docs/brand/print/mote-brand-sheet-a4.pdf")
reader = PdfReader(pdf)
assert len(reader.pages) == 1
page = reader.pages[0]
width = float(page.mediabox.width)
height = float(page.mediabox.height)
assert abs(width - 595.276) < 1
assert abs(height - 841.89) < 1
text = page.extract_text()
for expected in ["Mote", "#45A06B", "Fredoka Regular 400", "96% width", "A simple space for your photos."]:
    assert expected in text
print("brand sheet validated")
```

- [ ] **Step 3: Run the validator and confirm the missing-file failure**

Run:

```bash
python3 docs/brand/tools/validate_brand_sheet.py
```

Expected: FAIL because `mote-brand-sheet-a4.pdf` does not exist.

- [ ] **Step 4: Generate the print lockups and contact sheets**

Extend `generate-assets.mjs` to copy the outlined horizontal lockups into `print/`, then render both to 3000-pixel transparent PNGs with Sharp.

Create the main contact sheet on a `2400 x 1600` cool-white canvas. Include labelled light and dark lockups, full-colour app icons, monochrome symbols, web maskable icon, and the four palette swatches. Create the small-size sheet on an `1800 x 600` canvas using nearest-neighbour display cells for 16, 24, 32, 48, and 64 pixel Windows assets so edge quality can be judged without browser scaling.

- [ ] **Step 5: Implement the A4 brand sheet**

Create `docs/brand/tools/generate_brand_sheet.py` using ReportLab. It must set `A4`, add document title metadata, draw the light lockup at the top, draw a graphite panel with the dark lockup beneath it, add the symbol, palette, font specification, tagline, clear-space rule, minimum sizes, and licence note. Use the 3000-pixel transparent lockups at an effective resolution above 300 dpi.

The command-line contract is:

```bash
python3 docs/brand/tools/generate_brand_sheet.py \
  docs/brand/print/mote-lockup-light-3000.png \
  docs/brand/print/mote-lockup-dark-3000.png \
  docs/brand/print/mote-brand-sheet-a4.pdf
```

- [ ] **Step 6: Generate, validate, and render the PDF**

Run:

```bash
python3 docs/brand/tools/generate_brand_sheet.py \
  docs/brand/print/mote-lockup-light-3000.png \
  docs/brand/print/mote-lockup-dark-3000.png \
  docs/brand/print/mote-brand-sheet-a4.pdf
python3 docs/brand/tools/validate_brand_sheet.py
/Users/jennerm/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/override/pdftoppm \
  -png -r 150 docs/brand/print/mote-brand-sheet-a4.pdf \
  docs/brand/previews/mote-brand-sheet-a4
```

Expected: the validator prints `brand sheet validated`, and Poppler writes `mote-brand-sheet-a4-1.png`.

- [ ] **Step 7: Inspect all three preview images**

Open and inspect:

```text
docs/brand/previews/mote-brand-sheet-a4-1.png
docs/brand/previews/mote-asset-contact-sheet.png
docs/brand/previews/mote-small-size-check.png
```

Pass only when no mark is clipped, light and dark assets retain contrast, the 16-pixel mark keeps all four layers, wordmark outlines are sharp, labels are legible, and the A4 page has balanced margins.

- [ ] **Step 8: Commit print and preview assets**

```bash
git add docs/brand/print docs/brand/previews docs/brand/tools/generate_brand_sheet.py docs/brand/tools/validate_brand_sheet.py docs/brand/tools/generate-assets.mjs
git commit -m "feat: add Mote print and preview assets"
```

---

### Task 5: Document usage and complete pack validation

**Files:**
- Create: `docs/brand/README.md`
- Create: `docs/brand/fonts/README.md`
- Modify: `docs/brand/design-qa.md`
- Modify: `docs/brand/tools/generate-assets.test.mjs`

**Interfaces:**
- Consumes: the complete generated pack and design specification.
- Produces: a user-facing inventory, integration snippets, upstream font record, and final QA evidence.

- [ ] **Step 1: Add the failing documentation inventory test**

Append to `generate-assets.test.mjs`:

```js
test("README names every supported delivery target and appearance", async () => {
  const readme = await fs.readFile(path.join(root, "README.md"), "utf8");
  for (const term of ["macOS", "Windows", "Linux", "PWA", "Light mode", "Dark mode", "Print", "SIL Open Font License 1.1"]) {
    assert.match(readme, new RegExp(term.replaceAll(".", "\\."), "i"));
  }
});
```

- [ ] **Step 2: Run the test and confirm the missing-README failure**

Run:

```bash
npm run brand:test -- --test-name-pattern="README names"
```

Expected: FAIL because `docs/brand/README.md` does not exist.

- [ ] **Step 3: Write the pack README**

`docs/brand/README.md` must include:

- Approved palette and Fredoka 400 at 96% width.
- A table mapping every use case to one preferred file.
- Separate light-mode, dark-mode, and monochrome guidance.
- macOS, Windows, Linux, and PWA integration commands or configuration fragments.
- The PWA `icons` manifest fragment.
- Clear space and minimum sizes.
- The rule that app icons contain no wordmark.
- The rule that generated outputs are never edited directly.
- A note that the shipping Tauri icon is not replaced by this task.
- Links to the specification, contact sheet, small-size sheet, and printable PDF.

- [ ] **Step 4: Write the font README**

`docs/brand/fonts/README.md` must record:

```text
Family: Fredoka
Source: https://github.com/google/fonts/tree/main/ofl/fredoka
Licence: SIL Open Font License 1.1
Bundled files: Fredoka-Variable.ttf, Fredoka-Variable.woff2, OFL.txt
Wordmark settings: weight 400, width 96
```

Include this CSS:

```css
@font-face {
  font-family: "Fredoka";
  src: url("./Fredoka-Variable.woff2") format("woff2-variations");
  font-style: normal;
  font-weight: 300 700;
  font-display: swap;
}

.mote-wordmark {
  font-family: "Fredoka", sans-serif;
  font-variation-settings: "wght" 400, "wdth" 96;
  letter-spacing: -0.035em;
}
```

- [ ] **Step 5: Update the design QA record**

Add a platform-asset section to `docs/brand/design-qa.md` with:

- Source master paths.
- Contact-sheet and small-size-sheet paths.
- A4 PDF and rendered preview paths.
- Light and dark comparison evidence.
- 16, 24, 32, 48, and 64 pixel inspection results.
- ICO and ICNS unpacking results.
- Font family and axis validation.
- PWA manifest validation.
- `final result: passed` only when no P0, P1, or P2 finding remains.

- [ ] **Step 6: Run the complete automated verification**

Run:

```bash
npm run brand:generate
npm run brand:test
npm run check
npm run test --workspace @photo-viewer/interface
npm run build --workspace @photo-viewer/interface -- --mode memory
python3 docs/brand/tools/validate_brand_sheet.py
iconutil -c iconset docs/brand/icons/macos/Mote.icns -o /tmp/mote-verification.iconset
git diff --check
```

Expected:

- Brand generator exits 0.
- All brand tests pass.
- Repository style check passes.
- All interface unit tests pass.
- Interface build passes.
- PDF validator prints `brand sheet validated`.
- `iconutil` recreates the ten-file iconset.
- `git diff --check` prints nothing and exits 0.

- [ ] **Step 7: Inspect repository scope**

Run:

```bash
git status --short
git diff -- apps/desktop/src-tauri/icons apps/desktop/src-tauri/tauri.conf.json
```

Expected: the shipping icon and Tauri configuration have no changes. Every new production asset is inside `docs/brand/`, with only root package metadata changed for generation dependencies and scripts.

- [ ] **Step 8: Commit the completed asset pack**

```bash
git add package.json package-lock.json docs/brand
git commit -m "docs: add complete Mote brand asset pack"
```
