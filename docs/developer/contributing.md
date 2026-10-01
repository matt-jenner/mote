# Contribute to Mote

Small, focused changes are easiest to review. Open an issue before starting a
large feature or a change to storage, source safety, package formats, or public
behaviour.

## Requirements

Before changing code:

1. Read the relevant guide and existing tests.
2. Keep photo sources read-only in production code and tests.
3. Use the pinned Node, npm, Rust, and package toolchains.
4. Preserve platform behaviour unless the change explicitly targets one
   platform.
5. Add or update tests for changed behaviour.

Do not commit generated build directories, local catalogues, caches, runtime
state, personal photos, or credentials.

## Develop the change

Install locked dependencies with `npm ci`, then run `npm run test:photos` to
create disposable fixtures under `runtime/test-photos`. Start with the focused
command for the code you are changing, then run the broader checks before
opening a pull request.

For interface changes:

```bash
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
```

For Rust changes:

```bash
npm run rust:verify
```

Packaging or hosted changes also need the matching checks described in the
[build guide](building.md) and [debugging guide](debugging.md).

## Commits

Write a short imperative subject that states the change, for example:

```text
docs: split installation and developer guides
fix: retain cached previews for offline sources
```

Keep unrelated formatting, generated assets, and refactors out of the same
commit. Do not rewrite another contributor's work to make the diff look clean.

## Pull requests

The pull request description should include:

- the problem and the user-visible result;
- the implementation choices that need reviewer attention;
- exact verification commands and their results;
- screenshots or recordings for visible interface changes;
- remaining limits or follow-up work.

CI runs Rust checks on Ubuntu, macOS, and Windows, plus interface, packaging,
and hosted-container jobs. A green CI result does not replace a platform smoke
test when the change affects an installer, native folder picker, permissions,
or release package.

## Bug reports

Use the evidence list in [Debug Mote](debugging.md#report-a-useful-bug). Remove
private paths, filenames, addresses, and photo contents before posting logs or
screenshots publicly.
