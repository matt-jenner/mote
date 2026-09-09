# Linux Flatpak Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Mote from locked source as a portal-sandboxed x86-64 Flatpak bundle that installs on the current Linux workstation and Fedora.

**Architecture:** `flatpak-builder` compiles the existing React interface and Tauri/Rust desktop process inside GNOME SDK 49 using the Node 24 and Rust Stable 25.08 SDK extensions. Checked-in Cargo and npm source declarations make build commands offline; a thin repository script validates, builds, bundles, installs, runs, and inspects `io.github.matt_jenner.mote` without changing host package or remote configuration.

**Tech Stack:** Tauri 2.11, Rust 1.97.1, Node.js 24, npm 11, Flatpak, flatpak-builder, GNOME Platform/SDK 49, XDG desktop portals, Node test runner

**Spec:** `docs/superpowers/specs/2026-09-08-linux-flatpak-packaging-design.md`

## Global Constraints

- Flatpak/Flathub uses `io.github.matt_jenner.mote`; Tauri/macOS uses
  `io.github.matt-jenner.mote`. Flathub demangles the former to GitHub owner
  `matt-jenner`, while Tauri rejects underscores.
- No migration from `app.photoviewer.desktop` data or cache paths is included.
- The installed Linux executable is `mote`.
- The runtime and SDK are `org.gnome.Platform//49` and `org.gnome.Sdk//49`.
- Build with `org.freedesktop.Sdk.Extension.node24//25.08` and `org.freedesktop.Sdk.Extension.rust-stable//25.08`.
- Build commands run with npm and Cargo network access disabled.
- Runtime permissions are limited to Wayland, fallback X11, DRI, and shared IPC.
- Do not add filesystem, network, or source-write permissions.
- Reuse the approved Linux icons in `docs/brand/icons/linux/hicolor`; do not create new artwork.
- The acceptance build is x86-64, while paths and helper commands remain architecture-neutral.
- Do not add or modify a GitHub Actions workflow in this phase.

---

### Task 1: Establish Mote's permanent desktop identity

**Files:**
- Modify: `docs/brand/tools/desktop-integration.test.mjs`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `README.md`

**Interfaces:**
- Consumes: Tauri's `identifier` configuration and existing profile documentation
- Produces: the permanent Tauri/macOS ID `io.github.matt-jenner.mote`; later
  Flatpak packaging continues to use `io.github.matt_jenner.mote`.

- [ ] **Step 1: Add a failing application-ID test**

Add this import and constant to `docs/brand/tools/desktop-integration.test.mjs`:

```js
import path from "node:path";

const repositoryRoot = path.resolve(import.meta.dirname, "../../..");
const tauriConfig = JSON.parse(
	fs.readFileSync(
		path.join(repositoryRoot, "apps/desktop/src-tauri/tauri.conf.json"),
		"utf8",
	),
);
```

Add this test after the existing icon tests:

```js
test("Tauri uses the hyphenated Mote application identifier", () => {
	assert.equal(tauriConfig.identifier, "io.github.matt-jenner.mote");
});
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
npm run brand:test -- --test-name-pattern="hyphenated Mote application identifier"
```

Expected: FAIL because the actual value is `app.photoviewer.desktop`.

- [ ] **Step 3: Change the Tauri identifier and documented macOS paths**

In `apps/desktop/src-tauri/tauri.conf.json`, set:

```json
"identifier": "io.github.matt-jenner.mote"
```

In the macOS development section of `README.md`, replace the identifier and paths with:

```markdown
per-user macOS paths below the `io.github.matt-jenner.mote` Tauri identifier:

- `~/Library/Application Support/io.github.matt-jenner.mote/profiles/<profile>/catalog.sqlite`
- `~/Library/Caches/io.github.matt-jenner.mote/profiles/<profile>/`
```

- [ ] **Step 4: Run identity and desktop tests**

Run:

```bash
npm run brand:test
rg 'app\.photoviewer\.desktop' README.md apps/desktop
```

Expected: all brand tests PASS; `rg` returns no matches.

- [ ] **Step 5: Commit the identity change**

```bash
git add docs/brand/tools/desktop-integration.test.mjs apps/desktop/src-tauri/tauri.conf.json README.md
git commit -m "build: adopt permanent Mote application id"
```

---

### Task 2: Add Linux desktop and AppStream metadata

**Files:**
- Create: `packaging/flatpak/io.github.matt_jenner.mote.desktop`
- Create: `packaging/flatpak/io.github.matt_jenner.mote.metainfo.xml`
- Create: `tests/packaging/flatpak-metadata.test.mjs`
- Modify: `package.json`

**Interfaces:**
- Consumes: the Flatpak application ID and approved Linux icon assets
- Produces: freedesktop launcher and AppStream component metadata installed by Task 3

- [ ] **Step 1: Add failing metadata contract tests**

Create `tests/packaging/flatpak-metadata.test.mjs`:

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { XMLParser } from "fast-xml-parser";

const APP_ID = "io.github.matt_jenner.mote";
const root = path.resolve(import.meta.dirname, "../..");
const packaging = path.join(root, "packaging/flatpak");

function read(filename) {
	return fs.readFileSync(path.join(packaging, filename), "utf8");
}

test("desktop entry launches Mote with the permanent application ID", () => {
	const desktop = read(`${APP_ID}.desktop`);
	for (const line of [
		"[Desktop Entry]",
		"Type=Application",
		"Name=Mote",
		"Exec=mote",
		`Icon=${APP_ID}`,
		"Terminal=false",
		"Categories=Graphics;Photography;",
	]) {
		assert.ok(desktop.split(/\r?\n/).includes(line), `missing desktop entry line: ${line}`);
	}
});

test("AppStream metadata matches the Flatpak and desktop IDs", () => {
	const parsed = new XMLParser({
		ignoreAttributes: false,
		attributeNamePrefix: "@",
	}).parse(read(`${APP_ID}.metainfo.xml`));
	const component = parsed.component;
	assert.equal(component["@type"], "desktop-application");
	assert.equal(component.id, APP_ID);
	assert.equal(component.name, "Mote");
	assert.equal(component.metadata_license, "CC0-1.0");
	assert.equal(component.launchable["@type"], "desktop-id");
	assert.equal(component.launchable["#text"], `${APP_ID}.desktop`);
	assert.equal(component.url["@type"], "homepage");
	assert.equal(component.url["#text"], "https://github.com/matt-jenner/mote");
});

test("approved Linux icons cover Flatpak desktop integration", () => {
	for (const size of ["16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256", "512x512"]) {
		assert.ok(fs.existsSync(path.join(root, `docs/brand/icons/linux/hicolor/${size}/apps/mote.png`)));
	}
	assert.ok(fs.existsSync(path.join(root, "docs/brand/icons/linux/hicolor/scalable/apps/mote.svg")));
	assert.ok(fs.existsSync(path.join(root, "docs/brand/icons/linux/hicolor/scalable/apps/mote-symbolic.svg")));
});
```

Add the packaging test directory to the existing Biome check and add a focused test script in `package.json`:

```json
"check": "biome check apps tests/deployment tests/packaging",
"test:flatpak": "node --test tests/packaging/*.test.mjs"
```

- [ ] **Step 2: Run the tests and verify missing metadata fails**

Run:

```bash
npm run test:flatpak
```

Expected: FAIL with `ENOENT` for `io.github.matt_jenner.mote.desktop`.

- [ ] **Step 3: Create the desktop entry**

Create `packaging/flatpak/io.github.matt_jenner.mote.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Mote
Comment=A simple space for your photos
Exec=mote
Icon=io.github.matt_jenner.mote
Terminal=false
Categories=Graphics;Photography;
StartupNotify=true
```

- [ ] **Step 4: Create the AppStream metadata**

Create `packaging/flatpak/io.github.matt_jenner.mote.metainfo.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>io.github.matt_jenner.mote</id>
  <name>Mote</name>
  <summary>A simple space for your photos</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <developer id="io.github.matt_jenner.mote">
    <name>Mote contributors</name>
  </developer>
  <description>
    <p>Mote indexes local folders and mounted photo shares into a private catalogue, then presents a fast photo wall and immersive viewer.</p>
    <p>Photo sources are read-only. Mote stores its catalogue and generated previews separately from the selected source folders.</p>
  </description>
  <launchable type="desktop-id">io.github.matt_jenner.mote.desktop</launchable>
  <url type="homepage">https://github.com/matt-jenner/mote</url>
  <content_rating type="oars-1.1" />
  <recommends>
    <display_length compare="ge">360</display_length>
  </recommends>
  <supports>
    <control>pointing</control>
    <control>keyboard</control>
    <control>touch</control>
  </supports>
  <releases>
    <release version="0.1.0" date="2026-09-08">
      <description>
        <p>Initial private Linux test package.</p>
      </description>
    </release>
  </releases>
</component>
```

- [ ] **Step 5: Validate the metadata and rerun repository checks**

Run:

```bash
desktop-file-validate packaging/flatpak/io.github.matt_jenner.mote.desktop
appstreamcli validate --no-net packaging/flatpak/io.github.matt_jenner.mote.metainfo.xml
npm run test:flatpak
npm run check
```

Expected: both platform validators exit zero; Node tests and Biome PASS.

- [ ] **Step 6: Commit the desktop metadata**

```bash
git add packaging/flatpak/io.github.matt_jenner.mote.desktop packaging/flatpak/io.github.matt_jenner.mote.metainfo.xml tests/packaging/flatpak-metadata.test.mjs package.json
git commit -m "build: add Mote Linux desktop metadata"
```

---

### Task 3: Define the source-built Flatpak manifest

**Files:**
- Create: `packaging/flatpak/io.github.matt_jenner.mote.yml`
- Modify: `tests/packaging/flatpak-metadata.test.mjs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: Task 2 metadata, `package-lock.json`, `apps/desktop/src-tauri/Cargo.lock`, and generated sources from Task 4
- Produces: a `flatpak-builder` recipe whose acceptance ref is `app/io.github.matt_jenner.mote/x86_64/stable`

- [ ] **Step 1: Add failing manifest and sandbox tests**

Add imports to `tests/packaging/flatpak-metadata.test.mjs`:

```js
import { parse } from "yaml";
```

Add these tests:

```js
test("manifest pins the approved runtime and build SDKs", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	assert.equal(manifest.id, APP_ID);
	assert.equal(manifest.runtime, "org.gnome.Platform");
	assert.equal(manifest["runtime-version"], "49");
	assert.equal(manifest.sdk, "org.gnome.Sdk");
	assert.equal(manifest.command, "mote");
	assert.equal(manifest["default-branch"], "stable");
	assert.deepEqual(manifest["sdk-extensions"], [
		"org.freedesktop.Sdk.Extension.node24",
		"org.freedesktop.Sdk.Extension.rust-stable",
	]);
});

test("manifest grants display acceleration without host file or network access", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	assert.deepEqual(manifest["finish-args"], [
		"--socket=wayland",
		"--socket=fallback-x11",
		"--device=dri",
		"--share=ipc",
	]);
	const serialized = JSON.stringify(manifest["finish-args"]);
	assert.doesNotMatch(serialized, /--filesystem|--share=network|--socket=session-bus|--socket=system-bus/);
});

test("manifest builds npm and Cargo offline and installs matching metadata", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	const module = manifest.modules.find(({ name }) => name === "mote");
	assert.equal(module.buildsystem, "simple");
	assert.equal(module["build-options"].env.CARGO_NET_OFFLINE, "true");
	assert.equal(module["build-options"].env.npm_config_offline, "true");
	assert.match(module["build-options"]["append-path"], /node24/);
	assert.match(module["build-options"]["append-path"], /rust-stable/);
	const commands = module["build-commands"].join("\n");
	assert.match(commands, /npm ci --offline/);
	assert.match(commands, /desktop:build -- --no-bundle --ci/);
	assert.match(commands, /target\/release\/photo-viewer-desktop/);
	assert.ok(commands.includes(`${APP_ID}.desktop`));
	assert.ok(commands.includes(`${APP_ID}.metainfo.xml`));
	assert.match(commands, /for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512/);
	assert.ok(commands.includes(`/app/share/icons/hicolor/\${size}/apps/${APP_ID}.png`));
	assert.ok(commands.includes(`/app/share/icons/hicolor/scalable/apps/${APP_ID}.svg`));
	assert.ok(commands.includes(`/app/share/icons/hicolor/scalable/apps/${APP_ID}-symbolic.svg`));
	assert.ok(module.sources.some((source) => source === "generated/cargo-sources.json"));
	assert.ok(module.sources.some((source) => source === "generated/node-sources.json"));
});
```

- [ ] **Step 2: Run the focused tests and verify the manifest is missing**

Run:

```bash
npm run test:flatpak
```

Expected: metadata tests PASS and manifest tests FAIL with `ENOENT`.

- [ ] **Step 3: Create the Flatpak manifest**

Create `packaging/flatpak/io.github.matt_jenner.mote.yml`:

```yaml
id: io.github.matt_jenner.mote
runtime: org.gnome.Platform
runtime-version: "49"
sdk: org.gnome.Sdk
command: mote
default-branch: stable

sdk-extensions:
  - org.freedesktop.Sdk.Extension.node24
  - org.freedesktop.Sdk.Extension.rust-stable

finish-args:
  - --socket=wayland
  - --socket=fallback-x11
  - --device=dri
  - --share=ipc

modules:
  - name: mote
    buildsystem: simple
    build-options:
      append-path: /usr/lib/sdk/node24/bin:/usr/lib/sdk/rust-stable/bin
      env:
        HOME: /run/build/mote
        CARGO_HOME: /run/build/mote/cargo-home
        CARGO_NET_OFFLINE: "true"
        XDG_CACHE_HOME: /run/build/mote/flatpak-node/cache
        npm_config_cache: /run/build/mote/flatpak-node/npm-cache
        npm_config_nodedir: /usr/lib/sdk/node24
        npm_config_offline: "true"
    build-commands:
      - install -Dm0644 cargo/config .cargo/config.toml
      - npm ci --offline
      - npm run desktop:build -- --no-bundle --ci
      - install -Dm0755 apps/desktop/src-tauri/target/release/photo-viewer-desktop /app/bin/mote
      - install -Dm0644 packaging/flatpak/io.github.matt_jenner.mote.desktop /app/share/applications/io.github.matt_jenner.mote.desktop
      - install -Dm0644 packaging/flatpak/io.github.matt_jenner.mote.metainfo.xml /app/share/metainfo/io.github.matt_jenner.mote.metainfo.xml
      - |
        for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
          install -Dm0644 "docs/brand/icons/linux/hicolor/${size}/apps/mote.png" "/app/share/icons/hicolor/${size}/apps/io.github.matt_jenner.mote.png"
        done
      - install -Dm0644 docs/brand/icons/linux/hicolor/scalable/apps/mote.svg /app/share/icons/hicolor/scalable/apps/io.github.matt_jenner.mote.svg
      - install -Dm0644 docs/brand/icons/linux/hicolor/scalable/apps/mote-symbolic.svg /app/share/icons/hicolor/scalable/apps/io.github.matt_jenner.mote-symbolic.svg
    sources:
      - type: dir
        path: ../..
        skip:
          - .flatpak-builder
          - .git
          - node_modules
          - target
          - apps/desktop/src-tauri/target
          - apps/interface/dist
          - dist
      - generated/cargo-sources.json
      - generated/node-sources.json
```

- [ ] **Step 4: Ignore only generated build outputs**

Append to `.gitignore`:

```gitignore
.flatpak-builder/
build/flatpak/
dist/flatpak/
```

Do not ignore `packaging/flatpak/generated/`; its locked source declarations ship with the manifest.

- [ ] **Step 5: Run manifest contract and formatting checks**

Run:

```bash
npm run test:flatpak
npm run check
git diff --check
```

Expected: all commands PASS. A full `flatpak-builder` run remains intentionally blocked until Task 4 supplies generated sources.

- [ ] **Step 6: Commit the manifest**

```bash
git add packaging/flatpak/io.github.matt_jenner.mote.yml tests/packaging/flatpak-metadata.test.mjs .gitignore
git commit -m "build: define sandboxed Mote Flatpak"
```

---

### Task 4: Lock npm and Cargo sources for offline builds

**Files:**
- Create: `scripts/update-flatpak-sources.sh`
- Create: `packaging/flatpak/generated/node-sources.json` through the official generator
- Create: `packaging/flatpak/generated/cargo-sources.json` through the official generator
- Create: `packaging/flatpak/generated/source-lock.json` through the update script
- Modify: `tests/packaging/flatpak-metadata.test.mjs`

**Interfaces:**
- Consumes: `package-lock.json`, `apps/desktop/src-tauri/Cargo.lock`, and a caller-provided checkout of `flatpak/flatpak-builder-tools`
- Produces: complete external source declarations plus SHA-256 freshness evidence

- [ ] **Step 1: Add failing generated-source freshness tests**

Add this import to `tests/packaging/flatpak-metadata.test.mjs`:

```js
import { createHash } from "node:crypto";
```

Add these helpers and test:

```js
function sha256(filename) {
	return createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
}

test("generated Flatpak sources are nonempty and match both lockfiles", () => {
	const generated = path.join(packaging, "generated");
	const sourceLock = JSON.parse(fs.readFileSync(path.join(generated, "source-lock.json"), "utf8"));
	assert.equal(sourceLock.lockfiles["package-lock.json"], sha256(path.join(root, "package-lock.json")));
	assert.equal(
		sourceLock.lockfiles["apps/desktop/src-tauri/Cargo.lock"],
		sha256(path.join(root, "apps/desktop/src-tauri/Cargo.lock")),
	);
	assert.match(sourceLock.generator.repository, /^https:\/\/github\.com\/flatpak\/flatpak-builder-tools(?:\.git)?$/);
	assert.match(sourceLock.generator.commit, /^[0-9a-f]{40}$/);
	for (const filename of ["node-sources.json", "cargo-sources.json"]) {
		const sources = JSON.parse(fs.readFileSync(path.join(generated, filename), "utf8"));
		assert.ok(Array.isArray(sources));
		assert.ok(sources.length > 0);
	}
});
```

- [ ] **Step 2: Run the test and verify generated files are missing**

Run:

```bash
npm run test:flatpak
```

Expected: FAIL with `ENOENT` for `packaging/flatpak/generated/source-lock.json`.

- [ ] **Step 3: Create the source-update script**

Create executable `scripts/update-flatpak-sources.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/update-flatpak-sources.sh /path/to/flatpak-builder-tools" >&2
  exit 2
fi

tools_root="$(cd -- "$1" && pwd)"
cargo_generator="$tools_root/cargo/flatpak-cargo-generator.py"
output_root="$repository_root/packaging/flatpak/generated"

command -v flatpak-node-generator >/dev/null || {
  echo "flatpak-node-generator is required" >&2
  exit 1
}
[[ -f "$cargo_generator" ]] || {
  echo "flatpak-cargo-generator.py was not found below $tools_root" >&2
  exit 1
}

generator_commit="$(git -C "$tools_root" rev-parse HEAD)"
generator_repository="$(git -C "$tools_root" remote get-url origin)"
temporary_root="$(mktemp -d)"
cleanup() {
  rm -rf -- "$temporary_root"
}
trap cleanup EXIT

mkdir -p "$temporary_root/apps/desktop" "$temporary_root/apps/interface" "$output_root"
install -m 0644 "$repository_root/package.json" "$temporary_root/package.json"
install -m 0644 "$repository_root/package-lock.json" "$temporary_root/package-lock.json"
install -m 0644 "$repository_root/apps/desktop/package.json" "$temporary_root/apps/desktop/package.json"
install -m 0644 "$repository_root/apps/interface/package.json" "$temporary_root/apps/interface/package.json"

flatpak-node-generator \
  --no-requests-cache \
  --node-sdk-extension org.freedesktop.Sdk.Extension.node24//25.08 \
  --output "$output_root/node-sources.json" \
  npm "$temporary_root/package-lock.json"

python3 "$cargo_generator" \
  "$repository_root/apps/desktop/src-tauri/Cargo.lock" \
  --output "$output_root/cargo-sources.json"

node --input-type=module - "$repository_root" "$generator_repository" "$generator_commit" <<'NODE'
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const [root, repository, commit] = process.argv.slice(2);
const digest = (filename) => createHash("sha256").update(readFileSync(path.join(root, filename))).digest("hex");
const lock = {
  generator: { repository, commit },
  lockfiles: {
    "package-lock.json": digest("package-lock.json"),
    "apps/desktop/src-tauri/Cargo.lock": digest("apps/desktop/src-tauri/Cargo.lock"),
  },
};
writeFileSync(
  path.join(root, "packaging/flatpak/generated/source-lock.json"),
  `${JSON.stringify(lock, null, 2)}\n`,
);
NODE
```

Run:

```bash
chmod +x scripts/update-flatpak-sources.sh
```

- [ ] **Step 4: Generate the locked sources with isolated Python tooling**

Run from the repository root:

```bash
generator_root="$(mktemp -d /tmp/mote-flatpak-generators.XXXXXX)"
git clone https://github.com/flatpak/flatpak-builder-tools.git "$generator_root/tools"
python3 -m venv "$generator_root/venv"
"$generator_root/venv/bin/pip" install "$generator_root/tools/node" tomlkit aiohttp
PATH="$generator_root/venv/bin:$PATH" scripts/update-flatpak-sources.sh "$generator_root/tools"
```

Expected: the three files below `packaging/flatpak/generated/` are created; both source arrays are nonempty; `source-lock.json` records the exact generator commit and both lockfile hashes.

- [ ] **Step 5: Verify offline source freshness**

Run:

```bash
npm run test:flatpak
npm run check
git diff --check
```

Expected: all commands PASS.

- [ ] **Step 6: Commit the source declarations and updater**

```bash
git add scripts/update-flatpak-sources.sh packaging/flatpak/generated tests/packaging/flatpak-metadata.test.mjs
git commit -m "build: lock Flatpak npm and Cargo sources"
```

---

### Task 5: Add the non-interactive packaging interface

**Files:**
- Create: `scripts/flatpak.sh`
- Create: `tests/packaging/flatpak-cli.test.mjs`
- Modify: `package.json`

**Interfaces:**
- Consumes: manifest and generated sources from Tasks 3 and 4, preinstalled Flatpak runtimes, optional `MOTE_FLATPAK_ARCH`
- Produces: `build/flatpak/`, `dist/flatpak/repo/`, and the acceptance artifact `dist/flatpak/Mote-0.1.0-x86_64.flatpak`

- [ ] **Step 1: Add failing command-interface tests**

Create `tests/packaging/flatpak-cli.test.mjs`:

```js
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const script = path.join(root, "scripts/flatpak.sh");

test("Flatpak helper exposes the complete local packaging interface", () => {
	const help = execFileSync("bash", [script, "help"], { cwd: root, encoding: "utf8" });
	for (const command of ["check", "build", "bundle", "package", "install", "run", "inspect"]) {
		assert.match(help, new RegExp(`^  ${command}(?: |$)`, "m"));
	}
});

test("Flatpak helper is valid Bash and never escalates privileges", () => {
	execFileSync("bash", ["-n", script], { cwd: root });
	const source = fs.readFileSync(script, "utf8");
	assert.doesNotMatch(source, /\bsudo\b/);
	assert.match(source, /--noninteractive/);
	assert.match(source, /--runtime-repo=https:\/\/flathub\.org\/repo\/flathub\.flatpakrepo/);
	assert.match(source, /--show-permissions/);
});
```

- [ ] **Step 2: Run the focused tests and verify the helper is missing**

Run:

```bash
npm run test:flatpak
```

Expected: metadata tests PASS and CLI tests FAIL because `scripts/flatpak.sh` does not exist.

- [ ] **Step 3: Create the packaging helper**

Create executable `scripts/flatpak.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
app_id="io.github.matt_jenner.mote"
manifest="$repository_root/packaging/flatpak/$app_id.yml"
build_dir="${MOTE_FLATPAK_BUILD_DIR:-$repository_root/build/flatpak}"
repo_dir="${MOTE_FLATPAK_REPO_DIR:-$repository_root/dist/flatpak/repo}"
bundle_dir="${MOTE_FLATPAK_BUNDLE_DIR:-$repository_root/dist/flatpak}"

usage() {
  cat <<'EOF'
usage: scripts/flatpak.sh COMMAND

  check    validate tools, runtimes, manifest, and desktop metadata
  build    build and export Mote to the repository-local Flatpak repository
  bundle   create the single-file Flatpak bundle from that repository
  package  run build followed by bundle
  install  install or update the bundle for the current user
  run      launch the installed Mote Flatpak
  inspect  show installed application metadata and sandbox permissions
  help     show this help
EOF
}

require_command() {
  command -v "$1" >/dev/null || {
    echo "required command not found: $1" >&2
    exit 1
  }
}

flatpak_arch() {
  if [[ -n "${MOTE_FLATPAK_ARCH:-}" ]]; then
    printf '%s\n' "$MOTE_FLATPAK_ARCH"
  else
    flatpak --default-arch
  fi
}

app_version() {
  node -p "require('$repository_root/apps/desktop/src-tauri/tauri.conf.json').version"
}

bundle_path() {
  printf '%s/Mote-%s-%s.flatpak\n' "$bundle_dir" "$(app_version)" "$(flatpak_arch)"
}

check_requirements() {
  for command in flatpak flatpak-builder node desktop-file-validate appstreamcli; do
    require_command "$command"
  done
  for runtime in \
    org.gnome.Platform//49 \
    org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.node24//25.08 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08; do
    flatpak info "$runtime" >/dev/null || {
      echo "required Flatpak runtime is not installed: $runtime" >&2
      exit 1
    }
  done
  flatpak-builder --show-manifest "$manifest" >/dev/null
  desktop-file-validate "$repository_root/packaging/flatpak/$app_id.desktop"
  appstreamcli validate --no-net "$repository_root/packaging/flatpak/$app_id.metainfo.xml"
  npm run test:flatpak
}

build() {
  check_requirements
  mkdir -p "$build_dir" "$repo_dir" "$bundle_dir"
  flatpak-builder --force-clean --repo="$repo_dir" "$build_dir" "$manifest"
}

bundle() {
  require_command flatpak
  require_command node
  mkdir -p "$bundle_dir"
  flatpak build-bundle \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    --arch="$(flatpak_arch)" \
    "$repo_dir" "$(bundle_path)" "$app_id" stable
  printf 'bundle: %s\n' "$(bundle_path)"
}

install_bundle() {
  require_command flatpak
  flatpak install --user --noninteractive -y "$(bundle_path)"
}

case "${1:-help}" in
  check) check_requirements ;;
  build) build ;;
  bundle) bundle ;;
  package) build; bundle ;;
  install) install_bundle ;;
  run) require_command flatpak; flatpak run "$app_id" ;;
  inspect)
    require_command flatpak
    flatpak info "$app_id"
    flatpak info --show-permissions "$app_id"
    ;;
  help|-h|--help) usage ;;
  *) usage >&2; exit 2 ;;
esac
```

Run:

```bash
chmod +x scripts/flatpak.sh
```

Add this script to `package.json`:

```json
"flatpak": "bash scripts/flatpak.sh"
```

- [ ] **Step 4: Run command-interface and repository checks**

Run:

```bash
npm run test:flatpak
npm run check
npm run flatpak -- help
```

Expected: all tests PASS and help lists every supported operation. Do not run `build` until the host prerequisites in Task 7 are installed.

- [ ] **Step 5: Commit the packaging interface**

```bash
git add scripts/flatpak.sh tests/packaging/flatpak-cli.test.mjs package.json
git commit -m "build: add Flatpak packaging commands"
```

---

### Task 6: Document local, Fedora, and future CI workflows

**Files:**
- Create: `packaging/flatpak/README.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: commands from Task 5
- Produces: exact setup, build, transfer, installation, portal-test, and dependency-refresh instructions

- [ ] **Step 1: Add a failing documentation assertion**

Add this test to `tests/packaging/flatpak-metadata.test.mjs`:

```js
test("Flatpak guide covers both local distributions and defers CI", () => {
	const guide = read("README.md");
	for (const text of [
		"Omarchy or Arch Linux",
		"Fedora",
		"npm run flatpak -- package",
		"npm run flatpak -- install",
		"io.github.matt_jenner.mote",
		"GitHub Actions is deferred",
		"portal",
	]) {
		assert.ok(guide.toLowerCase().includes(text.toLowerCase()), `missing guide text: ${text}`);
	}
});
```

- [ ] **Step 2: Run the focused test and verify the guide is missing**

Run:

```bash
npm run test:flatpak
```

Expected: FAIL because `packaging/flatpak/README.md` is absent.

- [ ] **Step 3: Write the packaging guide**

Create `packaging/flatpak/README.md` with these exact sections and commands:

````markdown
# Mote Flatpak

Mote is built from locked source inside GNOME SDK 49. The installed app uses the
ID `io.github.matt_jenner.mote` and receives photo-folder access only through the
desktop file chooser portal.

## Host setup

On Omarchy or Arch Linux:

```bash
omarchy pkg add flatpak flatpak-builder
```

On Fedora:

```bash
sudo dnf install flatpak flatpak-builder
```

Add Flathub and install the build/runtime references for the current user:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive -y flathub \
  org.gnome.Platform//49 \
  org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.node24//25.08 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
```

The repository helper deliberately does not install packages, add remotes, or
install runtimes.

## Build and install

```bash
npm run flatpak -- check
npm run flatpak -- package
npm run flatpak -- install
npm run flatpak -- inspect
npm run flatpak -- run
```

For version 0.1.0 on the acceptance architecture, the bundle is written as
`dist/flatpak/Mote-0.1.0-x86_64.flatpak`. The helper derives later filenames
from the Tauri version and `flatpak --default-arch`.
Build commands have no network access; `flatpak-builder` fetches only the
URL-and-checksum sources declared in the manifest before entering the build
sandbox.

## Install the bundle on Fedora

Copy the `.flatpak` file to Fedora, then run:

```bash
flatpak install --user ./Mote-0.1.0-x86_64.flatpak
flatpak run io.github.matt_jenner.mote
flatpak info --show-permissions io.github.matt_jenner.mote
```

The bundle records Flathub as its runtime source. Flatpak may offer to download
GNOME Platform 49 if Fedora does not already have it.

## Portal acceptance test

Choose `apps/interface/public/demo-photos` in Mote's system folder picker,
confirm that the photo wall and viewer load, quit Mote, and confirm the same
library works after relaunch. Repeat with a mounted NAS photo folder visible in
the system picker. Installed permissions must not contain `filesystems` or
`shared=network` entries.

## Refresh locked sources

After either lockfile changes, use an isolated checkout and virtual environment:

```bash
generator_root="$(mktemp -d /tmp/mote-flatpak-generators.XXXXXX)"
git clone https://github.com/flatpak/flatpak-builder-tools.git "$generator_root/tools"
python3 -m venv "$generator_root/venv"
"$generator_root/venv/bin/pip" install "$generator_root/tools/node" tomlkit aiohttp
PATH="$generator_root/venv/bin:$PATH" scripts/update-flatpak-sources.sh "$generator_root/tools"
npm run test:flatpak
```

Commit both generated JSON files and `source-lock.json` with the lockfile change.

## CI status

GitHub Actions is deferred for this phase. The checked-in manifest, sources,
tests, and non-interactive helper are intended to be called unchanged from a
later Ubuntu runner.
````

- [ ] **Step 4: Link the Flatpak guide from the root README**

Add a `Build and run the Linux Flatpak` section immediately after the macOS desktop section:

```markdown
## Build and run the Linux Flatpak

Mote's distribution-independent Linux package is built from source with
`flatpak-builder` and uses portal-only folder access. Follow the
[Flatpak build and installation guide](packaging/flatpak/README.md) to create a
local `.flatpak` bundle, install it on this workstation, or copy it to Fedora.
```

- [ ] **Step 5: Run documentation and formatting checks**

Run:

```bash
npm run test:flatpak
npm run check
git diff --check
```

Expected: all commands PASS.

- [ ] **Step 6: Commit the documentation**

```bash
git add packaging/flatpak/README.md README.md tests/packaging/flatpak-metadata.test.mjs
git commit -m "docs: document Mote Flatpak workflow"
```

---

### Task 7: Build, install, and hand off the x86-64 bundle

**Files:**
- Verify only: `.github/workflows/ci.yml`
- Generated and ignored: `build/flatpak/`
- Generated and ignored: `dist/flatpak/repo/`
- Generated and ignored: `dist/flatpak/Mote-0.1.0-x86_64.flatpak`

**Interfaces:**
- Consumes: completed Tasks 1 through 6 and user-level Flatpak prerequisites
- Produces: installed `io.github.matt_jenner.mote` plus a transferable x86-64 bundle

- [ ] **Step 1: Install local Flatpak tooling with the platform-supported package wrapper**

On the current Omarchy workstation, run with user approval:

```bash
omarchy pkg add flatpak flatpak-builder
```

Expected: `flatpak --version` and `flatpak-builder --version` both succeed.

- [ ] **Step 2: Configure the user remote and install required SDKs**

Run with user approval:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive -y flathub \
  org.gnome.Platform//49 \
  org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.node24//25.08 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
```

Expected: `flatpak info` resolves all four references.

- [ ] **Step 3: Run the existing and new test gates**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p photo-app-service --test task7_source_safety
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm run check
npm run typecheck
npm test
npm run test:browser
npm run test:flatpak
```

Expected: every command exits zero.

- [ ] **Step 4: Build and bundle Mote inside Flatpak**

Run:

```bash
MOTE_FLATPAK_ARCH=x86_64 npm run flatpak -- package
```

Expected: `flatpak-builder` completes without a network permission in its build sandbox and creates `dist/flatpak/Mote-0.1.0-x86_64.flatpak`.

- [ ] **Step 5: Install and inspect the package**

Run:

```bash
MOTE_FLATPAK_ARCH=x86_64 npm run flatpak -- install
npm run flatpak -- inspect
flatpak info --show-permissions io.github.matt_jenner.mote | tee /tmp/mote-flatpak-permissions.txt
```

Expected: the installed ID is `io.github.matt_jenner.mote`, the branch is `stable`, the architecture is `x86_64`, and permissions contain only display/graphics/IPC access. They contain no `filesystems` value and no `shared=network` value.

- [ ] **Step 6: Confirm the existing CI workflow was untouched**

Run:

```bash
git diff --exit-code 6378aa497539e3115a627ca92cbf809c9d75b140...HEAD -- .github/workflows/ci.yml
```

Expected: no output and exit code zero.

- [ ] **Step 7: Launch for the user's portal acceptance test**

Run:

```bash
npm run flatpak -- run
```

Ask the user to complete the documented demo-folder, relaunch, and mounted-NAS checks. Record any failure with the Flatpak version, portal backend, selected mount type, and `journalctl --user` messages before changing permissions.

- [ ] **Step 8: Provide the Fedora handoff**

Provide `dist/flatpak/Mote-0.1.0-x86_64.flatpak` together with the three Fedora commands from `packaging/flatpak/README.md`. Do not commit the bundle or the repository-local Flatpak repository.
