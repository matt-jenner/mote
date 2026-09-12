# Build Asset Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every repository-owned Mote build clean its temporary assets on success, failure, and handled interruption while retaining one stable macOS app and one current Flatpak bundle.

**Architecture:** A shared POSIX-shell lifecycle library owns guarded temporary directories, PID leases, signal handling, stale-run recovery, and safe deletion. Desktop, Rust, container, and Flatpak entry points use that library or an equivalent run-scoped ownership contract, then promote validated deliverables only after successful builds.

**Tech Stack:** POSIX shell, Bash, Node.js 24 test runner, npm workspaces, Cargo, Tauri 2, Podman/Docker, Flatpak Builder, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-12-build-asset-lifecycle-design.md`

## Global Constraints

- Retain the latest local macOS bundle at `dist/macos/Mote.app`.
- Retain only the latest versioned Flatpak bundle in `dist/flatpak/`.
- Never delete source photos, application databases, generated runtime caches, installed applications, named container volumes, unrelated container assets, running containers, or another live build.
- Clean managed working assets on success, failure, `HUP`, `INT`, and `TERM`.
- Recover crash leftovers only after confirming that their recorded host process is no longer alive.
- Never run a system-wide container image, volume, builder, or network prune.
- A failed package build must preserve the previous stable artifact.
- Preserve the pre-existing changes in `.github/workflows/ci.yml`, `.gitignore`, `README.md`, `package.json`, and `.github/workflows/build-macos.yml`. Review their combined diffs before staging them.

---

## File structure

- Create `scripts/build-lifecycle.sh`: shared temporary-directory, lease, trap, and guarded-removal functions.
- Create `scripts/with-ephemeral-cargo-target.sh`: run one arbitrary command with a temporary Cargo target.
- Create `scripts/rust-verify.sh`: run the documented Rust verification sequence in one temporary target.
- Create `scripts/desktop-build.sh`: build and transactionally publish the stable macOS app.
- Create `scripts/clean-build-assets.sh`: remove legacy repository outputs and dead Mote-owned temporary assets.
- Create `scripts/hosted-smoke-assets.sh`: inspect and remove only dead-owner Mote smoke assets.
- Create `tests/build/build-lifecycle.test.mjs`: lifecycle, signal, safe-path, Rust-wrapper, desktop-promotion, and legacy-cleanup tests.
- Modify `scripts/hosted-smoke.sh`: unique image ownership, stale smoke cleanup, and exit-time image removal.
- Modify `tests/deployment/hosted-smoke-engine.test.mjs`: executable fake-engine coverage of build failure cleanup.
- Modify `scripts/flatpak.sh`: ephemeral package intermediates and transactional single-bundle publication.
- Modify `tests/packaging/flatpak-cli.test.mjs`: command-surface and fake-tool package cleanup coverage.
- Modify `package.json`: managed build, verification, and cleanup commands.
- Modify `README.md`, `packaging/flatpak/README.md`, and `docs/deployment/hosted.md`: make managed commands the documented paths.
- Modify `.github/workflows/build-macos.yml`: consume `dist/macos/Mote.app`.
- Inspect `.github/workflows/ci.yml`: preserve its pre-existing macOS-job extraction diff without adding local cleanup to ephemeral CI runners.

---

### Task 1: Shared build lifecycle

**Files:**
- Create: `scripts/build-lifecycle.sh`
- Create: `tests/build/build-lifecycle.test.mjs`

**Interfaces:**
- Produces: `mote_reap_stale_builds`, `mote_create_build_dir KIND`, `mote_install_cleanup_traps`, `mote_cleanup_build_dir`, and `mote_remove_managed_path PATH PREFIX`.
- Produces environment: `MOTE_BUILD_DIR` and `MOTE_BUILD_TMP_ROOT`.
- Consumes: `TMPDIR`, with `/tmp` as fallback.

- [ ] **Step 1: Write failing lifecycle tests**

Create `tests/build/build-lifecycle.test.mjs` with helpers that run a shell under a fixture `TMPDIR`. Cover successful cleanup, original failure-status preservation, `TERM` cleanup, a live PID lease, a dead PID lease, a malformed lease, and rejection outside the exact managed prefix.

```js
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const lifecycle = path.join(root, "scripts/build-lifecycle.sh");

function fixture() {
	return fs.mkdtempSync(path.join(os.tmpdir(), "mote-lifecycle-test-"));
}

function run(source, tmpdir) {
	return spawnSync("sh", ["-c", `. "${lifecycle}"\n${source}`], {
		env: { ...process.env, TMPDIR: tmpdir },
		encoding: "utf8",
	});
}

test("managed build directory is removed after success", () => {
	const tmpdir = fixture();
	try {
		const result = run(
			"mote_create_build_dir unit\nprintf '%s' \"$MOTE_BUILD_DIR\" > \"$TMPDIR/path\"\nmote_install_cleanup_traps",
			tmpdir,
		);
		assert.equal(result.status, 0, result.stderr);
		const build = fs.readFileSync(path.join(tmpdir, "path"), "utf8");
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("cleanup preserves a wrapped failure status", () => {
	const tmpdir = fixture();
	try {
		const result = run(
			"mote_create_build_dir unit\nprintf '%s' \"$MOTE_BUILD_DIR\" > \"$TMPDIR/path\"\nmote_install_cleanup_traps\nexit 23",
			tmpdir,
		);
		assert.equal(result.status, 23);
		const build = fs.readFileSync(path.join(tmpdir, "path"), "utf8");
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});
```

The signal test must spawn a shell, wait until it writes its build path, send `SIGTERM`, assert exit status `143`, and assert the directory no longer exists. The lease tests must create `mote-build-unit.*` directories manually with `.mote-owner-pid` files containing the current Node PID, a known dead PID, and invalid text.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs
```

Expected: FAIL because `scripts/build-lifecycle.sh` does not exist.

- [ ] **Step 3: Implement guarded lifecycle functions**

Create `scripts/build-lifecycle.sh` as a source-only POSIX library. Use these exact contracts:

```sh
#!/bin/sh

mote_build_tmp_root() {
	raw_root=${MOTE_BUILD_TMP_ROOT:-${TMPDIR:-/tmp}}
	case "$raw_root" in ''|'/')
		printf '%s\n' "refusing unsafe Mote temporary root: $raw_root" >&2
		return 1
	esac
	[ -d "$raw_root" ] || {
		printf '%s\n' "Mote temporary root does not exist: $raw_root" >&2
		return 1
	}
	canonical_root=$(CDPATH= cd -- "$raw_root" && pwd -P)
	if [ -n "${HOME:-}" ]; then
		canonical_home=$(CDPATH= cd -- "$HOME" && pwd -P)
		[ "$canonical_root" != "$canonical_home" ] || {
			printf '%s\n' "refusing home directory as Mote temporary root" >&2
			return 1
		}
	fi
	printf '%s\n' "$canonical_root"
}

mote_remove_managed_path() {
	candidate=$1
	prefix=$2
	case "$candidate" in
		"$prefix"/*) ;;
		*) printf '%s\n' "refusing to remove unmanaged path: $candidate" >&2; return 1 ;;
	esac
	[ -n "$candidate" ] && [ "$candidate" != "/" ] && [ "$candidate" != "$prefix" ] || {
		printf '%s\n' "refusing to remove unsafe path: $candidate" >&2
		return 1
	}
	rm -rf -- "$candidate"
}

mote_reap_stale_builds() {
	root=$(mote_build_tmp_root)
	for candidate in "$root"/mote-build-*; do
		[ -d "$candidate" ] || continue
		owner_file="$candidate/.mote-owner-pid"
		if ! IFS= read -r owner_pid <"$owner_file"; then
			printf '%s\n' "skipping Mote build with missing owner: $candidate" >&2
			continue
		fi
		case "$owner_pid" in ''|*[!0-9]*)
			printf '%s\n' "skipping Mote build with invalid owner: $candidate" >&2
			continue
		esac
		if kill -0 "$owner_pid" 2>/dev/null; then
			continue
		fi
		mote_remove_managed_path "$candidate" "$root" || return 1
	done
}

mote_create_build_dir() {
	kind=$1
	case "$kind" in ''|*[!a-z0-9-]*)
		printf '%s\n' "invalid Mote build kind: $kind" >&2
		return 1
	esac
	MOTE_BUILD_TMP_ROOT=$(mote_build_tmp_root)
	export MOTE_BUILD_TMP_ROOT
	mote_reap_stale_builds
	MOTE_BUILD_DIR=$(mktemp -d "$MOTE_BUILD_TMP_ROOT/mote-build-$kind.XXXXXX")
	export MOTE_BUILD_DIR
	printf '%s\n' "$$" >"$MOTE_BUILD_DIR/.mote-owner-pid"
}

mote_cleanup_build_dir() {
	[ -n "${MOTE_BUILD_DIR:-}" ] || return 0
	directory=$MOTE_BUILD_DIR
	MOTE_BUILD_DIR=
	export MOTE_BUILD_DIR
	mote_remove_managed_path "$directory" "$MOTE_BUILD_TMP_ROOT"
}

mote_cleanup_on_exit() {
	command_status=$?
	trap - EXIT HUP INT TERM
	cleanup_status=0
	mote_cleanup_build_dir || cleanup_status=$?
	if [ "$command_status" -ne 0 ]; then
		exit "$command_status"
	fi
	exit "$cleanup_status"
}

mote_install_cleanup_traps() {
	trap mote_cleanup_on_exit EXIT
	trap 'exit 129' HUP
	trap 'exit 130' INT
	trap 'exit 143' TERM
}
```

Do not add a generic arbitrary-path deletion API. Every caller must supply an already validated exact prefix.

- [ ] **Step 4: Run the lifecycle tests and verify GREEN**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs
```

Expected: all lifecycle tests PASS with no managed fixture left under the test temporary roots.

- [ ] **Step 5: Commit the isolated lifecycle files**

```bash
git add scripts/build-lifecycle.sh tests/build/build-lifecycle.test.mjs
git commit -m "build: add guarded temporary asset lifecycle"
```

---

### Task 2: Managed Rust verification and desktop development

**Files:**
- Create: `scripts/with-ephemeral-cargo-target.sh`
- Create: `scripts/rust-verify.sh`
- Modify: `tests/build/build-lifecycle.test.mjs`
- Modify with the pre-existing diff in Task 7: `package.json`

**Interfaces:**
- Consumes: lifecycle functions from Task 1.
- Produces: `scripts/with-ephemeral-cargo-target.sh COMMAND...` and `scripts/rust-verify.sh`.
- Later package commands: `npm run rust:verify` and managed `npm run desktop:dev`.

- [ ] **Step 1: Write failing wrapper tests**

Add tests that put a fake `cargo` and fake arbitrary command at the front of `PATH`. Each fake command records `CARGO_TARGET_DIR`, creates one file there, and exits with a configured status. Assert all commands in one Rust verification run receive the same temporary target, that the target is gone afterward, and that `with-ephemeral-cargo-target.sh` returns the child's status.

```js
test("ephemeral Cargo wrapper removes the target and returns the child status", () => {
	const tmpdir = fixture();
	const bin = path.join(tmpdir, "bin");
	fs.mkdirSync(bin);
	const command = path.join(bin, "fake-command");
	fs.writeFileSync(
		command,
		"#!/bin/sh\nprintf '%s' \"$CARGO_TARGET_DIR\" > \"$TMPDIR/cargo-path\"\nmkdir -p \"$CARGO_TARGET_DIR\"\nexit 19\n",
	);
	fs.chmodSync(command, 0o755);
	const result = spawnSync(
		path.join(root, "scripts/with-ephemeral-cargo-target.sh"),
		[command],
		{ env: { ...process.env, TMPDIR: tmpdir }, encoding: "utf8" },
	);
	assert.equal(result.status, 19);
	const target = fs.readFileSync(path.join(tmpdir, "cargo-path"), "utf8");
	assert.equal(fs.existsSync(target), false);
});
```

- [ ] **Step 2: Run the wrapper tests and verify RED**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs
```

Expected: FAIL because the wrapper scripts do not exist.

- [ ] **Step 3: Implement both wrappers**

`scripts/with-ephemeral-cargo-target.sh` must source the lifecycle helper, create one build directory, install traps, set `CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"`, and invoke the passed command without `exec` so the EXIT trap runs.

```sh
#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_dir/build-lifecycle.sh"

[ "$#" -gt 0 ] || { printf '%s\n' "usage: $0 COMMAND [ARG ...]" >&2; exit 2; }
mote_create_build_dir cargo
mote_install_cleanup_traps
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR"
"$@"
```

`scripts/rust-verify.sh` must create one lifecycle and target, then run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p catalog-bench --test benchmark_smoke
```

The script must stop on the first failure and clean the shared target through the installed trap.

- [ ] **Step 4: Run the wrapper tests and verify GREEN**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs
```

Expected: PASS. The fake command's target path no longer exists.

- [ ] **Step 5: Commit only the new scripts and their test additions**

```bash
git add scripts/with-ephemeral-cargo-target.sh scripts/rust-verify.sh tests/build/build-lifecycle.test.mjs
git commit -m "build: isolate local Cargo verification assets"
```

Do not stage `package.json` in this task because it had a pre-existing uncommitted desktop-build change.

---

### Task 3: Transactional macOS app packaging

**Files:**
- Create: `scripts/desktop-build.sh`
- Modify: `tests/build/build-lifecycle.test.mjs`

**Interfaces:**
- Consumes: lifecycle functions from Task 1 and Tauri through the desktop npm workspace.
- Produces: `scripts/desktop-build.sh native [TAURI_ARGS...]` and `scripts/desktop-build.sh universal [TAURI_ARGS...]`.
- Produces artifact: `dist/macos/Mote.app`.

- [ ] **Step 1: Write failing desktop publication tests**

Build a temporary repository fixture containing copies of `desktop-build.sh` and `build-lifecycle.sh`. Put fake `npm` and `ditto` commands on `PATH`. Fake `npm` must create the expected `Mote.app/Contents/MacOS/photo-viewer-desktop` under `CARGO_TARGET_DIR`, or exit with a configured failure. Cover:

1. a failed build preserves an existing stable bundle;
2. a successful native build replaces it;
3. a successful universal build reads the universal target path;
4. an invalid or duplicate candidate bundle fails without replacing the stable bundle; and
5. every case removes its temporary Cargo target and promotion staging paths.

```js
test("failed desktop build preserves the stable app", () => {
	const repository = desktopFixture({ existingMarker: "accepted", buildStatus: 17 });
	const result = runDesktopBuild(repository, "native");
	assert.equal(result.status, 17);
	assert.equal(
		fs.readFileSync(path.join(repository, "dist/macos/Mote.app/marker"), "utf8"),
		"accepted",
	);
	assert.deepEqual(findMoteApps(path.join(repository, "dist/macos")), [
		path.join(repository, "dist/macos/Mote.app"),
	]);
});
```

- [ ] **Step 2: Run the desktop tests and verify RED**

Run:

```bash
node --test --test-name-pattern='desktop build' tests/build/build-lifecycle.test.mjs
```

Expected: FAIL because `scripts/desktop-build.sh` does not exist.

- [ ] **Step 3: Implement the desktop build script**

The script must:

1. validate `native|universal` before creating anything;
2. create a managed `desktop` build directory and set `CARGO_TARGET_DIR`;
3. run Tauri with `--bundles app`, adding `--target universal-apple-darwin` only in universal mode;
4. require `Contents/Info.plist` and exactly one executable regular file in `Contents/MacOS`;
5. copy with `ditto` to `dist/macos/.Mote.app.next.$$`;
6. validate the copy;
7. move the prior stable app to `.Mote.app.previous.$$`;
8. rename the candidate to `dist/macos/Mote.app`;
9. restore the previous app if candidate promotion fails; and
10. remove backup, staging, and managed target paths in all exit paths.

The mode-specific candidate paths are exact:

```sh
case "$mode" in
	native) candidate="$CARGO_TARGET_DIR/release/bundle/macos/Mote.app" ;;
	universal) candidate="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle/macos/Mote.app" ;;
esac
```

The successful invocation is:

```sh
if [ "$mode" = universal ]; then
	npm exec --workspace @photo-viewer/desktop -- tauri build \
		--target universal-apple-darwin --bundles app "$@"
else
	npm exec --workspace @photo-viewer/desktop -- tauri build --bundles app "$@"
fi
```

- [ ] **Step 4: Run desktop tests and verify GREEN**

Run:

```bash
node --test --test-name-pattern='desktop build' tests/build/build-lifecycle.test.mjs
```

Expected: all desktop publication cases PASS.

- [ ] **Step 5: Commit the isolated desktop script and tests**

```bash
git add scripts/desktop-build.sh tests/build/build-lifecycle.test.mjs
git commit -m "build: publish one stable macOS app"
```

---

### Task 4: Self-cleaning hosted smoke images

**Files:**
- Create: `scripts/hosted-smoke-assets.sh`
- Modify: `scripts/hosted-smoke.sh`
- Modify: `tests/deployment/hosted-smoke-engine.test.mjs`

**Interfaces:**
- Produces per-run image: `localhost/mote-smoke:${run_id}`.
- Labels: `io.github.matt-jenner.mote.asset=hosted-smoke` and `io.github.matt-jenner.mote.owner-pid=$$`.
- Produces: `mote_reap_stale_smoke_assets ENGINE` for the smoke and manual cleanup commands.
- Preserves: `localhost/photo-viewer:dev` for the long-lived Compose deployment.

- [ ] **Step 1: Write a failing fake-engine cleanup test**

Create a fake Podman executable that appends every argument list to a log, returns valid data for `info`, fails the `build` command with status `31`, and succeeds for cleanup commands. Run `scripts/hosted-smoke.sh` with that executable as `CONTAINER_ENGINE`.

```js
test("hosted smoke removes its unique image after a failed build", () => {
	const fixtureDirectory = mkdtempSync(path.join(os.tmpdir(), "mote-engine-"));
	const fakeEngine = path.join(fixtureDirectory, "podman");
	const log = path.join(fixtureDirectory, "calls.log");
	writeFileSync(
		fakeEngine,
		`#!/bin/sh\nprintf '%s\\n' "$*" >>"${log}"\nif [ "$1" = info ]; then printf '%s\\n' '[]'; exit 0; fi\nif [ "$1" = build ]; then exit 31; fi\nexit 0\n`,
	);
	chmodSync(fakeEngine, 0o755);
	try {
		const result = spawnSync(smokeScript, [], {
			cwd: repositoryRoot,
			env: { ...process.env, CONTAINER_ENGINE: fakeEngine },
			encoding: "utf8",
		});
		assert.equal(result.status, 31);
		const calls = readFileSync(log, "utf8");
		const tag = calls.match(/--tag (localhost\/mote-smoke:[^ ]+)/)?.[1];
		assert.ok(tag);
		assert.match(calls, new RegExp(`image rm ${escapeRegExp(tag)}`));
		assert.doesNotMatch(calls, /system prune|volume prune|image prune/);
	} finally {
		rmSync(fixtureDirectory, { recursive: true, force: true });
	}
});
```

- [ ] **Step 2: Run the hosted engine test and verify RED**

Run:

```bash
node --test tests/deployment/hosted-smoke-engine.test.mjs
```

Expected: FAIL because the current script builds the shared `localhost/photo-viewer:dev` image and never removes it.

- [ ] **Step 3: Add unique image ownership and cleanup**

Set:

```sh
image_name="localhost/mote-smoke:${run_id}"
asset_label="io.github.matt-jenner.mote.asset=hosted-smoke"
owner_label="io.github.matt-jenner.mote.owner-pid=$$"
```

Use `$image_name` for build, inspect, and run. Add build flags `--rm --force-rm`, both labels, and the unique tag. Extend the existing cleanup function after container and network removal:

```sh
if ! "$container_engine" image rm "$image_name" >/dev/null 2>&1; then
	printf '%s\n' "retained smoke image still referenced by a container: $image_name" >&2
	[ "$exit_status" -ne 0 ] || exit_status=1
fi
exit "$exit_status"
```

Label the run container and network. Implement `mote_reap_stale_smoke_assets` in `scripts/hosted-smoke-assets.sh`. It lists only names beginning with `photo-viewer-smoke-`, parses the final PID component, uses `kill -0` to skip live owners, removes only stopped containers and unused networks, and removes only `localhost/mote-smoke:*` images without `--force`. Source the helper from `hosted-smoke.sh` and call it after validating the engine. Do not call any prune command.

Add a second fake-engine case for stale recovery. Return one running asset with a live owner PID, one stopped asset with a live owner PID, and one stopped asset with a known dead owner PID. Assert that only the stopped dead-owner container, its unused network, and its unreferenced image receive removal calls.

- [ ] **Step 4: Run hosted engine tests and verify GREEN**

Run:

```bash
npm run test:deployment
```

Expected: PASS, including failure cleanup and absence of broad prune calls.

- [ ] **Step 5: Commit hosted smoke changes**

```bash
git add scripts/hosted-smoke-assets.sh scripts/hosted-smoke.sh tests/deployment/hosted-smoke-engine.test.mjs
git commit -m "build: remove hosted smoke container assets"
```

---

### Task 5: Ephemeral Flatpak packaging

**Files:**
- Modify: `scripts/flatpak.sh`
- Modify: `tests/packaging/flatpak-cli.test.mjs`

**Interfaces:**
- Consumes: lifecycle functions from Task 1.
- Produces command surface: `check`, `package`, `install`, `run`, `inspect`, and `help`.
- Removes command surface: split `build` and `bundle` commands.
- Produces one artifact matching `dist/flatpak/Mote-<version>-<arch>.flatpak`.

- [ ] **Step 1: Write failing Flatpak lifecycle tests**

Update the help test to require `package` and reject `build` and `bundle`. Add a fake-tool test with `TMPDIR` and `MOTE_FLATPAK_BUNDLE_DIR` pointing at a fixture. Fake `flatpak-builder` must create its build and repository arguments; fake `flatpak build-bundle` must create the candidate path. Seed an older `Mote-0.0.9-x86_64.flatpak`, run `package`, and assert:

- the command succeeds;
- the old bundle is gone;
- exactly one new non-empty bundle remains; and
- no `mote-build-flatpak.*` directory remains below the fixture temporary root.

Add a failure case where `flatpak build-bundle` exits `29`; assert the previous bundle remains and the temporary directory is gone.

- [ ] **Step 2: Run Flatpak tests and verify RED**

Run:

```bash
node --test tests/packaging/flatpak-cli.test.mjs
```

Expected: FAIL because split commands remain and packaging uses persistent `build/flatpak` plus `dist/flatpak/repo`.

- [ ] **Step 3: Implement transactional package behavior**

Source `scripts/build-lifecycle.sh`. In `package`, create a `flatpak` managed directory, install cleanup traps, and set:

```bash
build_dir="$MOTE_BUILD_DIR/build"
repo_dir="$MOTE_BUILD_DIR/repo"
candidate_bundle="$MOTE_BUILD_DIR/$(basename "$(bundle_path)")"
```

Run requirement checks, `flatpak-builder --force-clean --repo="$repo_dir"`, and `flatpak build-bundle` against `$candidate_bundle`. Require the candidate to be a non-empty regular file. Move it to a hidden staging path inside `dist/flatpak`, then move existing `Mote-*.flatpak` files into a hidden backup directory beside the staging path. Rename the candidate to its versioned final path. If that rename fails, move every backed-up bundle to its original name. Remove the backup directory only after promotion succeeds.

Remove `build` and `bundle` from help and dispatch. Keep `MOTE_FLATPAK_BUNDLE_DIR` as the test and operator override for the stable output directory. Never delete installed runtimes or applications.

- [ ] **Step 4: Run Flatpak tests and verify GREEN**

Run:

```bash
npm run test:flatpak
```

Expected: PASS with one fixture bundle and no fixture intermediate directories.

- [ ] **Step 5: Commit Flatpak code and tests**

```bash
git add scripts/flatpak.sh tests/packaging/flatpak-cli.test.mjs
git commit -m "build: clean Flatpak package intermediates"
```

---

### Task 6: Scoped cleanup for legacy repository assets

**Files:**
- Create: `scripts/clean-build-assets.sh`
- Modify: `tests/build/build-lifecycle.test.mjs`

**Interfaces:**
- Consumes: lifecycle functions from Task 1 and `mote_reap_stale_smoke_assets` from Task 4.
- Produces: `scripts/clean-build-assets.sh` and later `npm run clean:build-assets`.

- [ ] **Step 1: Write failing cleanup-scope tests**

Copy the cleanup and lifecycle scripts into a temporary repository fixture. Create main and `.worktrees/example` target trees, desktop targets, interface `dist` and `.vite` trees, Flatpak intermediates, stable macOS and Flatpak artifacts, application-like data outside the repository, a dead managed temporary build, and a live managed temporary build.

Run the cleanup script and assert it removes only:

```text
target/
apps/desktop/src-tauri/target/
apps/interface/dist/
apps/interface/.vite/
build/flatpak/
dist/flatpak/repo/
.worktrees/example/target/
.worktrees/example/apps/desktop/src-tauri/target/
dead mote-build-* temporary directory
```

Assert it preserves:

```text
dist/macos/Mote.app/
dist/flatpak/Mote-0.1.0-x86_64.flatpak
live mote-build-* temporary directory
all paths outside the repository fixture and Mote temporary prefix
```

Run the fixture with the Task 4 fake engine and assert the cleanup command calls the shared stale-smoke reaper, preserves its running and live-owner assets, and never issues a prune command.

- [ ] **Step 2: Run scoped cleanup tests and verify RED**

Run:

```bash
node --test --test-name-pattern='legacy build assets' tests/build/build-lifecycle.test.mjs
```

Expected: FAIL because the cleanup script does not exist.

- [ ] **Step 3: Implement exact-path cleanup**

Derive `repository_root` from the script location. Define `clean_checkout` to accept only the repository root or a direct child of `$repository_root/.worktrees`. For each accepted checkout, remove only the four generated target/interface paths listed in Step 1. Remove repository Flatpak intermediates through exact path checks. Call `mote_reap_stale_builds` for temporary assets. Source `hosted-smoke-assets.sh`; when `${CONTAINER_ENGINE:-podman}` exists and its daemon is available, call `mote_reap_stale_smoke_assets` with that engine. If the engine or daemon is unavailable, print a skip message and continue cleaning filesystem assets.

Do not glob arbitrary `target` or `dist` directories outside those exact relative locations. Print `removed: PATH`, `absent: PATH`, or the lifecycle helper's `skipping` message for every category.

- [ ] **Step 4: Run cleanup tests and verify GREEN**

Run:

```bash
node --test --test-name-pattern='legacy build assets' tests/build/build-lifecycle.test.mjs
```

Expected: PASS with stable artifacts and the live fixture untouched.

- [ ] **Step 5: Commit cleanup command and tests**

```bash
git add scripts/clean-build-assets.sh tests/build/build-lifecycle.test.mjs
git commit -m "build: add scoped legacy asset cleanup"
```

---

### Task 7: Route commands, workflows, and documentation through managed paths

**Files:**
- Modify with pre-existing changes: `package.json`
- Modify with pre-existing changes: `README.md`
- Modify: `packaging/flatpak/README.md`
- Modify: `docs/deployment/hosted.md`
- Modify with pre-existing changes: `.github/workflows/build-macos.yml`
- Inspect with pre-existing changes: `.github/workflows/ci.yml`
- Preserve with pre-existing changes: `.gitignore`
- Test: `tests/build/build-lifecycle.test.mjs`
- Test: `tests/packaging/flatpak-release-workflow.test.mjs`

**Interfaces:**
- Produces npm commands: `rust:verify`, managed `desktop:dev`, `desktop:build`, `desktop:build:universal`, and `clean:build-assets`.
- Stable macOS workflow input: `dist/macos/Mote.app`.

- [ ] **Step 1: Write failing command-contract tests**

Add assertions that parse `package.json` and verify exact managed commands:

```js
assert.equal(packageJson.scripts["rust:verify"], "sh scripts/rust-verify.sh");
assert.equal(
	packageJson.scripts["desktop:dev"],
	"sh scripts/with-ephemeral-cargo-target.sh npm exec --workspace @photo-viewer/desktop -- tauri dev",
);
assert.equal(packageJson.scripts["desktop:build"], "sh scripts/desktop-build.sh native");
assert.equal(
	packageJson.scripts["desktop:build:universal"],
	"sh scripts/desktop-build.sh universal",
);
assert.equal(
	packageJson.scripts["clean:build-assets"],
	"sh scripts/clean-build-assets.sh",
);
```

Extend workflow tests to require `dist/macos/Mote.app` and reject app paths below a Cargo target. Extend Flatpak tests to reject documentation of split `build` and `bundle` helper commands.

- [ ] **Step 2: Run command-contract tests and verify RED**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs tests/packaging/*.test.mjs
```

Expected: FAIL because package scripts, workflow paths, and documentation still expose unmanaged commands.

- [ ] **Step 3: Update package scripts and macOS workflow**

Merge these entries into the existing `package.json` diff without removing unrelated scripts:

```json
"rust:verify": "sh scripts/rust-verify.sh",
"desktop:dev": "sh scripts/with-ephemeral-cargo-target.sh npm exec --workspace @photo-viewer/desktop -- tauri dev",
"desktop:build": "sh scripts/desktop-build.sh native",
"desktop:build:universal": "sh scripts/desktop-build.sh universal",
"clean:build-assets": "sh scripts/clean-build-assets.sh"
```

In `.github/workflows/build-macos.yml`, verify and archive `dist/macos/Mote.app`. Keep the existing universal architecture verification, artifact name, 14-day retention, and uncommitted workflow separation from `ci.yml`.

Do not route GitHub-hosted Rust jobs through `rust:verify` when doing so would defeat `swatinem/rust-cache`; hosted runners are ephemeral and their cache action owns retention. Use the managed command for local documentation and any non-cached local workflow only.

- [ ] **Step 4: Update documentation**

Replace direct local Rust verification with `npm run rust:verify`. Document that direct Cargo commands can leave `target/` and that `npm run clean:build-assets` removes scoped leftovers. Change desktop build output to `dist/macos/Mote.app` and explain that it is replaced only after a successful build.

Update Flatpak documentation to list only `check`, `package`, `install`, `inspect`, and `run`. Update hosted documentation so smoke images are ephemeral and the long-lived Compose shutdown uses `compose down --rmi local` without `--volumes`.

- [ ] **Step 5: Run command, packaging, and formatting tests**

Run:

```bash
node --test tests/build/build-lifecycle.test.mjs tests/deployment/*.test.mjs tests/packaging/*.test.mjs
npm run check
```

Expected: PASS with no formatting changes required.

- [ ] **Step 6: Review overlapping diffs before staging**

Run:

```bash
git diff -- .github/workflows/ci.yml .github/workflows/build-macos.yml .gitignore README.md package.json
git diff --check
```

Confirm the earlier universal macOS workflow, README, ignore, and package changes remain present and that lifecycle edits extend them rather than replace them. Do not commit these overlapping files without explicit user confirmation.

---

### Task 8: Real build verification and current-machine cleanup

**Files:**
- Runtime output retained: `dist/macos/Mote.app`
- Runtime output removed: repository Cargo targets, interface builds, managed temporary directories, and Mote smoke images.

**Interfaces:**
- Consumes all managed commands from Tasks 1 through 7.
- Produces final evidence only; no new production interface.

- [ ] **Step 1: Run the complete automated verification suite**

Run:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run brand:test
npm run test:deployment
npm run test:flatpak
npm run rust:verify
```

Expected: every command exits `0`. `rust:verify` leaves no `mote-build-cargo.*` directory and no repository `target/`.

- [ ] **Step 2: Run a real native macOS package build**

Run:

```bash
npm run desktop:build
```

Expected: exit `0`; `dist/macos/Mote.app/Contents/Info.plist` exists; exactly one executable exists in its `Contents/MacOS`; no desktop Cargo target remains in the repository or Mote temporary root.

- [ ] **Step 3: Run a real Podman hosted smoke test**

Run:

```bash
CONTAINER_ENGINE=podman ./scripts/hosted-smoke.sh
```

Expected: smoke test exits `0`; no container, network, or image carrying this run's ID remains. Existing Papertrail containers, named volumes, and `localhost/photo-viewer:dev` remain unchanged.

- [ ] **Step 4: Run scoped legacy cleanup**

Run:

```bash
npm run clean:build-assets
```

Expected: existing root, desktop, interface, worktree, and Flatpak intermediate outputs are removed. `dist/macos/Mote.app` and the current Flatpak bundle remain.

- [ ] **Step 5: Verify the final asset inventory**

Run:

```bash
find . .worktrees -type d \( -name target -o -name .vite \) -prune -print 2>/dev/null
find . .worktrees -type d -name Mote.app -prune -print 2>/dev/null
find "${TMPDIR:-/tmp}" -maxdepth 1 -type d -name 'mote-build-*' -print 2>/dev/null
podman ps -a --filter name=photo-viewer-smoke-
podman images --filter reference='localhost/mote-smoke:*'
df -h /System/Volumes/Data
git status --short
```

Expected:

- no target, `.vite`, temporary-build, smoke-container, or smoke-image output;
- exactly one repository app at `./dist/macos/Mote.app`;
- the post-cleanup free-space inventory is reported; and
- only the known pre-existing working-tree changes plus the intended lifecycle implementation.

- [ ] **Step 6: Review completion evidence**

Use `superpowers:requesting-code-review` for the complete implementation diff. Address actionable findings through `superpowers:receiving-code-review`, rerun the failed task's focused tests, then rerun Steps 1 through 5 before reporting completion.
