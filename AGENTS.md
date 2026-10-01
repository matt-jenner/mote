# AGENTS.md

These instructions apply to the whole repository. They are for automated
coding agents and contributors using agent-assisted tools.

## Protect user data

- Treat every configured photo source as read-only. Production code must not
  write, rename, move, or delete source files.
- Run `npm run test:photos` and use `runtime/test-photos/demo-photos`, or use a disposable copy. Never
  run destructive tests against a personal library, mounted archive, or NAS.
- Keep SQLite data and derivative caches outside photo sources.
- Do not expose native server paths, filenames, credentials, or photo contents
  in logs, fixtures, bug reports, screenshots, or pull requests.
- Do not delete local catalogues, caches, release packages, or untracked files
  unless the user names the exact target and approves the deletion.

## Inspect before editing

1. Read `README.md` and the relevant page under `docs/`.
2. Check `git status --short`. Existing changes belong to the user.
3. Read the nearest tests and recent commits for the area being changed.
4. Keep the change focused. Do not reformat or refactor unrelated files.

Use `rg` and `rg --files` for repository searches. Use `apply_patch` for
manual file edits. Prefer checked-in scripts over retyping build commands.

## Set up the machine

The shared toolchain is:

- Node.js 24.18.0 or newer;
- npm 11.16.0 or newer;
- Rust 1.97.1 through rustup, with Rustfmt and Clippy;
- Git, CMake, and the platform C or C++ compiler.

Install locked JavaScript dependencies with:

```bash
npm ci
```

Install Playwright WebKit on macOS and Chromium on Windows or Linux. Read
`docs/developer/README.md` before installing platform package tools. Do not
change lockfiles or tool versions only to work around a local setup problem.

## Run and debug

Start the desktop app with a named profile when clean state matters:

```bash
PHOTO_VIEWER_PROFILE=agent-debug npm run desktop:dev
```

Choose a unique profile name for concurrent or repeated investigations. Run
`npm run test:photos`, then use `runtime/test-photos/demo-photos` unless the task
needs another explicit fixture.

Debug the smallest failing layer first:

- interface behaviour: `npm test` or `npm run test:browser`;
- TypeScript contracts: `npm run typecheck`;
- Rust behaviour: the narrowest `cargo test -p <crate> <test>` command;
- hosted lifecycle: `npm run test:deployment` or `./scripts/hosted-smoke.sh`;
- packaging: `npm run test:build` or `npm run test:flatpak`.

After a failed command, report the file or component, the direct cause, and the
next fix. After three failed fixes for the same symptom, stop and name the
assumption that now looks doubtful.

## Build packages

Follow `docs/developer/building.md`. Build desktop packages on their destination
operating system. Do not invent cross-compilation paths.

HEIC is enabled by default. Use the documented `--no-heic` flag to test the
disabled branch. Do not edit Cargo feature sets or packaging manifests as a
shortcut.

Use `npm run clean:build` for known build outputs. Never replace it with a broad
recursive delete. The command preserves source photos, application state, and
retained packages.

## Verify changes

Run checks in proportion to the changed area. The normal interface gate is:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
```

The Rust gate is:

```bash
npm run rust:verify
```

Documentation-only changes need a Markdown link check, searches for stale moved
paths, and review of every changed page. Do not claim a check passed without
fresh command output from the current working tree.

## Bug reports

Record the operating system, architecture, Mote version or commit, app variant,
exact reproduction steps, expected result, actual result, and the smallest
relevant log excerpt. For hosted problems, include the redacted `/healthz`
response and container engine.

Ask before requesting a source photo. Prefer format details and metadata first.
Never post a personal photo or native library path in a public issue.

## Commits and pull requests

- Use imperative commit subjects with a useful area prefix such as `fix:`,
  `feat:`, `docs:`, or `build:`.
- Keep unrelated user changes out of commits. Never reset or discard them.
- State verification commands and results in the pull request.
- Include screenshots for visible interface changes and platform smoke results
  for installer, picker, permission, or package changes.
- Link the relevant issue or design record when one exists.

Do not push, open a pull request, publish a release, or change remote state
unless the user asks for that action.
