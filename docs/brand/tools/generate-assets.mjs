import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { XMLValidator } from "fast-xml-parser";
import { openSync } from "fontkit";
import pngToIco from "png-to-ico";
import sharp from "sharp";
import { BRAND, PATHS, PLATFORM_SIZES, expectedOutputs } from "./config.mjs";
import { exportSvgSources, symbolSvg } from "./svg.mjs";

const execFileAsync = promisify(execFile);
const generatedDirectories = ["source", "svg", "fonts", "icons"];

async function renderPng(svg, size, output) {
  await fs.mkdir(path.dirname(output), { recursive: true });
  await sharp(Buffer.from(svg))
    .resize(size, size)
    .png({ compressionLevel: 9 })
    .withMetadata({ icc: "srgb" })
    .toFile(output);
}

async function writeText(output, contents) {
  await fs.mkdir(path.dirname(output), { recursive: true });
  await fs.writeFile(output, `${contents.trim()}\n`, "utf8");
}

async function writeIco(pngPaths, output) {
  await fs.mkdir(path.dirname(output), { recursive: true });
  await fs.writeFile(output, await pngToIco(pngPaths));
}

async function writeModernIcns(iconset, output) {
  const representations = [
    ["icp4", "icon_16x16.png"],
    ["ic11", "icon_16x16@2x.png"],
    ["icp5", "icon_32x32.png"],
    ["ic12", "icon_32x32@2x.png"],
    ["ic07", "icon_128x128.png"],
    ["ic13", "icon_128x128@2x.png"],
    ["ic08", "icon_256x256.png"],
    ["ic14", "icon_256x256@2x.png"],
    ["ic09", "icon_512x512.png"],
    ["ic10", "icon_512x512@2x.png"],
  ];
  const chunks = [];
  for (const [type, filename] of representations) {
    const image = await fs.readFile(path.join(iconset, filename));
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, "ascii");
    header.writeUInt32BE(image.length + 8, 4);
    chunks.push(header, image);
  }
  const body = Buffer.concat(chunks);
  const header = Buffer.alloc(8);
  header.write("icns", 0, 4, "ascii");
  header.writeUInt32BE(body.length + 8, 4);
  await fs.writeFile(output, Buffer.concat([header, body]));
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

function macIconSvg(mode) {
  return symbolSvg({ mode, tile: true, size: 1024 })
    .replace(
      /(<svg[^>]*>)/,
      '$1<defs><filter id="tile-shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#000000" flood-opacity="0.22"/></filter></defs>',
    )
    .replace('data-layer="tile"', 'data-layer="tile" filter="url(#tile-shadow)"');
}

async function generateMac(stage) {
  const directory = path.join(stage, "icons/macos");
  const iconset = path.join(directory, "Mote.iconset");
  await fs.mkdir(iconset, { recursive: true });
  await renderPng(macIconSvg("light"), 1024, path.join(directory, "icon-1024.png"));
  await renderPng(macIconSvg("dark"), 1024, path.join(directory, "icon-1024-dark.png"));

  const entries = [
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
  await Promise.all(
    entries.map(([size, filename]) =>
      renderPng(
        symbolSvg({ mode: "light", tile: true, size }),
        size,
        path.join(iconset, filename),
      ),
    ),
  );
  await writeModernIcns(iconset, path.join(directory, "Mote.icns"));
}

async function generateWindows(stage) {
  const directory = path.join(stage, "icons/windows");
  const pngDirectory = path.join(directory, "png");
  const pngs = [];
  for (const size of PLATFORM_SIZES.windows) {
    const output = path.join(pngDirectory, `mote-${size}.png`);
    await renderPng(symbolSvg({ mode: "light", tile: true, size }), size, output);
    pngs.push(output);
  }
  await writeIco(pngs, path.join(directory, "Mote.ico"));
}

async function generateLinux(stage) {
  const hicolor = path.join(stage, "icons/linux/hicolor");
  for (const size of PLATFORM_SIZES.linux) {
    await renderPng(
      symbolSvg({ mode: "light", tile: true, size }),
      size,
      path.join(hicolor, `${size}x${size}/apps/mote.png`),
    );
  }
  const scalable = path.join(hicolor, "scalable/apps");
  await writeText(path.join(scalable, "mote.svg"), symbolSvg({ mode: "light", tile: true }));
  await writeText(
    path.join(scalable, "mote-symbolic.svg"),
    symbolSvg({ mode: "light", monochrome: true }),
  );
  await writeText(
    path.join(scalable, "mote-symbolic-dark.svg"),
    symbolSvg({ mode: "dark", monochrome: true }),
  );
}

function adaptiveFaviconSvg() {
  return symbolSvg({ mode: "light", tile: true })
    .replace(
      /(<svg[^>]*>)/,
      `$1<style>:root{--mote-tile:${BRAND.colors.graphite};--mote-rear:${BRAND.colors.graphite}}@media(prefers-color-scheme:dark){:root{--mote-tile:${BRAND.colors.darkTile};--mote-rear:${BRAND.colors.white}}}</style>`,
    )
    .replace(`fill="${BRAND.colors.graphite}"`, 'fill="var(--mote-tile)"')
    .replace(`stroke="${BRAND.colors.graphite}"`, 'stroke="var(--mote-rear)"');
}

async function generateWeb(stage) {
  const directory = path.join(stage, "icons/web");
  await fs.mkdir(directory, { recursive: true });

  const standard = [192, 512];
  for (const size of standard) {
    await renderPng(
      symbolSvg({ mode: "light", tile: true, size }),
      size,
      path.join(directory, `icon-${size}.png`),
    );
    await renderPng(
      symbolSvg({ mode: "light", tile: true, maskable: true, size }),
      size,
      path.join(directory, `maskable-${size}.png`),
    );
  }
  await renderPng(
    symbolSvg({ mode: "light", tile: true, size: 180 }),
    180,
    path.join(directory, "apple-touch-icon-180.png"),
  );

  const faviconPngs = [];
  for (const size of [16, 32, 48]) {
    const filename = path.join(directory, `.favicon-${size}.png`);
    await renderPng(symbolSvg({ mode: "light", tile: true, size }), size, filename);
    faviconPngs.push(filename);
  }
  await writeIco(faviconPngs, path.join(directory, "favicon.ico"));
  await Promise.all(faviconPngs.map((filename) => fs.unlink(filename)));

  await writeText(path.join(directory, "favicon.svg"), adaptiveFaviconSvg());
  await writeText(
    path.join(directory, "service-monochrome.svg"),
    symbolSvg({ mode: "light", monochrome: true }),
  );
  await writeText(
    path.join(directory, "manifest-icons.json"),
    JSON.stringify(
      {
        icons: [
          { src: "icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
          { src: "icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
          {
            src: "maskable-192.png",
            sizes: "192x192",
            type: "image/png",
            purpose: "maskable",
          },
          {
            src: "maskable-512.png",
            sizes: "512x512",
            type: "image/png",
            purpose: "maskable",
          },
          {
            src: "service-monochrome.svg",
            sizes: "any",
            type: "image/svg+xml",
            purpose: "monochrome",
          },
        ],
      },
      null,
      2,
    ),
  );
}

async function validateStage(stage) {
  for (const output of expectedOutputs()) {
    const relative = output.replace(/^docs\/brand\//, "");
    await fs.access(path.join(stage, relative));
  }
  const ttf = openSync(path.join(stage, "fonts/Fredoka-Variable.ttf"));
  const woff2 = openSync(path.join(stage, "fonts/Fredoka-Variable.woff2"));
  if (ttf.familyName !== woff2.familyName) throw new Error("WOFF2 family name changed");
  if (!woff2.variationAxes.wght || !woff2.variationAxes.wdth) {
    throw new Error("WOFF2 variation axes are incomplete");
  }
  for (const directory of ["source", "svg", "icons/linux", "icons/web"]) {
    const queue = [path.join(stage, directory)];
    while (queue.length > 0) {
      const current = queue.pop();
      for (const entry of await fs.readdir(current, { withFileTypes: true })) {
        const filename = path.join(current, entry.name);
        if (entry.isDirectory()) queue.push(filename);
        else if (entry.name.endsWith(".svg")) {
          const result = XMLValidator.validate(await fs.readFile(filename, "utf8"));
          if (result !== true) throw new Error(`Invalid SVG: ${filename}`);
        }
      }
    }
  }
}

async function preserveManualFiles(stage, destination) {
  for (const relative of ["fonts/README.md"]) {
    const source = path.join(destination, relative);
    try {
      await fs.access(source);
      const target = path.join(stage, relative);
      await fs.mkdir(path.dirname(target), { recursive: true });
      await fs.copyFile(source, target);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
}

async function replaceGeneratedDirectories(stage, destination) {
  const token = `${process.pid}-${Date.now()}`;
  const swapped = [];
  try {
    for (const directory of generatedDirectories) {
      const current = path.join(destination, directory);
      const next = path.join(stage, directory);
      const backup = path.join(destination, `.${directory}.backup-${token}`);
      let hadCurrent = true;
      try {
        await fs.rename(current, backup);
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
        hadCurrent = false;
      }
      await fs.rename(next, current);
      swapped.push({ current, backup, hadCurrent });
    }
  } catch (error) {
    for (const item of swapped.reverse()) {
      await fs.rm(item.current, { recursive: true, force: true });
      if (item.hadCurrent) await fs.rename(item.backup, item.current);
    }
    throw error;
  }
  await Promise.all(
    swapped.filter((item) => item.hadCurrent).map((item) => fs.rm(item.backup, { recursive: true })),
  );
}

export async function generateAll({ destination = PATHS.brandDir, keepStage = false } = {}) {
  const stage = await fs.mkdtemp(path.join(os.tmpdir(), "mote-brand-"));
  try {
    await exportSvgSources(stage);
    await copyFontPackage(stage);
    await Promise.all([
      generateMac(stage),
      generateWindows(stage),
      generateLinux(stage),
      generateWeb(stage),
    ]);
    await validateStage(stage);
    await preserveManualFiles(stage, destination);
    await replaceGeneratedDirectories(stage, destination);
  } finally {
    if (!keepStage) await fs.rm(stage, { recursive: true, force: true });
  }
}

const currentFile = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === currentFile) {
  await generateAll();
}
