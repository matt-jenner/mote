# Build asset lifecycle design

## Purpose

Mote's repository-owned development, verification, packaging, and smoke-test
commands must clean up their working assets when they finish. This applies on
success, command failure, and handled interruption. A later invocation must
also remove leftovers from a crashed process after confirming that the owning
process is no longer alive.

The repository retains only intentional deliverables:

- the latest local macOS bundle at `dist/macos/Mote.app`;
- the latest versioned Flatpak bundle in `dist/flatpak/`;
- an installed application outside the repository;
- an image used by a running long-lived hosted deployment; and
- persistent application data and derivative caches.

Build targets, development bundles, packaging repositories, smoke-test images,
containers, and networks are temporary. Cleanup must never alter source photos,
application databases, generated runtime caches, installed applications,
unrelated container assets, or another live build.

## Ownership model

Every managed local build creates a unique directory below the operating
system's temporary directory. The directory name uses a Mote-specific prefix
and contains an ownership file with the host process ID. Repository scripts
install exit and signal traps immediately after creating the directory.

On startup, the lifecycle helper examines only directories matching its exact
Mote prefix. It removes a directory only when its ownership file contains a
valid process ID and that process is no longer alive. It skips live owners and
malformed ownership files. PID reuse may retain an old directory, which is a
safe leak; it must never cause deletion of a live directory.

All deletion functions validate their paths against exact repository or
temporary-root prefixes. They reject empty paths, filesystem roots, home
directories, the repository root, and paths outside the allowed Mote-owned
locations.

## Shared lifecycle helper

`scripts/build-lifecycle.sh` supplies the reusable POSIX-shell operations for:

- finding the repository and temporary roots without relying on the caller's
  current directory;
- reaping dead Mote-owned temporary directories;
- creating a unique working directory and ownership file;
- installing cleanup traps while preserving the wrapped command's exit code;
- checking whether an ownership process is alive;
- validating a path before recursive deletion; and
- promoting a completed artifact without exposing a partial replacement.

Cleanup failures are printed to standard error but do not replace the wrapped
command's original failure status. A successful command becomes unsuccessful
when its required cleanup cannot be completed, because silent accumulation
would break this feature's contract.

## Rust verification and desktop development

The repository adds a managed `rust:verify` command. Rustfmt runs without a
build target. Clippy, workspace tests, and the benchmark smoke test share one
temporary `CARGO_TARGET_DIR` for that invocation. The helper removes it when
verification finishes.

`desktop:dev` also receives a unique temporary `CARGO_TARGET_DIR`. The target
remains present for the complete Tauri development process, including rebuilds,
and is removed after the app and development command exit.

Raw `cargo` commands cannot be intercepted by repository code. Documentation
therefore uses the managed commands. A separate scoped cleanup command removes
legacy targets created by raw commands.

## macOS packaging

`scripts/desktop-build.sh` accepts the native and universal build modes used by
the package scripts. It builds inside a temporary Cargo target. After Tauri
returns successfully, the script requires exactly one bundle at the expected
mode-specific path and validates the bundle structure before promotion.

The script copies the candidate beside `dist/macos/Mote.app`, verifies the
copy, moves any previous stable bundle to a temporary backup, and renames the
candidate into the stable path. If promotion fails, it restores the previous
bundle. Once promotion succeeds, it deletes the backup and temporary Cargo
target.

Both native and universal builds publish to `dist/macos/Mote.app`; the latest
successful package is the retained repository artifact. The macOS GitHub
workflow verifies that stable bundle and creates its downloadable zip from the
stable path. GitHub's runner and artifact-retention policy handle remote build
storage.

## Hosted container lifecycle

The hosted smoke script uses an image tag unique to its run instead of the
long-lived `localhost/photo-viewer:dev` deployment tag. The image, temporary
container, and temporary network carry Mote smoke-test labels and the same run
identifier.

The existing exit trap removes the run's container and network, then removes
its image without forcing deletion. If another container references the image,
the engine refuses removal and the script reports the retained image. The
script never invokes a system-wide image, volume, builder, or network prune.

Build options request removal of intermediate build containers on both Docker
and Podman. A subsequent smoke run may inspect Mote-labelled smoke assets from
earlier crashed runs. It removes only stopped containers, unused networks, and
unreferenced images whose recorded host owner is no longer alive. Running
containers are never treated as stale.

The long-lived Compose deployment continues to use
`localhost/photo-viewer:dev`. Its documented shutdown command removes the
stopped service container, Compose network, and now-unused local image while
preserving bind-mounted data and cache directories.

## Flatpak packaging

The `flatpak package` path creates its build directory and exported repository
inside a managed temporary directory. It retains only the resulting versioned
`.flatpak` file. After a candidate bundle is complete, the helper removes older
`Mote-*.flatpak` bundles and promotes the candidate. A failed build or bundle
keeps the previous bundle.

The helper removes the split `build` and `bundle` commands because they require
an unowned export repository to persist between processes. `package` is the
only artifact-producing command. The `check`, `install`, `run`, and `inspect`
commands do not create packaging intermediates.

Installed Flatpak applications and runtimes are outside this cleanup policy.

## Scoped legacy cleanup

`npm run clean:build-assets` removes only known generated locations:

- root Cargo `target/` directories in the main checkout and repository-managed
  worktrees;
- desktop Cargo targets below `apps/desktop/src-tauri/` in those checkouts;
- interface `dist/` and `.vite/` output in those checkouts;
- Mote-owned temporary build directories whose recorded owner is dead;
- abandoned Mote smoke-test container assets that are stopped and unreferenced;
  and
- Flatpak build and export intermediates.

The command preserves `dist/macos/Mote.app`, the current Flatpak bundle,
application-support data, application caches, named container volumes, running
containers, installed apps, and any build with a live ownership marker. It
prints each removed category and reports skipped live or malformed assets.

## Failure and interruption behavior

Scripts handle `HUP`, `INT`, and `TERM`, forward the resulting failure status,
and run the same guarded cleanup used on normal exit. An uncatchable process
death or machine restart may leave a working directory. The ownership check on
the next managed invocation provides recovery.

No temporary build may replace a stable deliverable until all build and
artifact validation steps succeed. Cleanup is idempotent so an explicit signal
handler and the exit trap cannot delete the same asset unsafely.

## Verification

Automated tests use temporary fixtures and fake commands or container engines
to prove:

- temporary Cargo targets disappear after success, failure, and termination;
- a live ownership marker prevents stale cleanup;
- path validation rejects deletion outside Mote-owned locations;
- failed desktop and Flatpak packages preserve the previous stable artifact;
- successful packaging leaves exactly one stable artifact;
- the hosted smoke script uses a unique image and removes only its own image,
  container, and network; and
- cleanup preserves the wrapped command's exit status and reports cleanup
  failures separately.

Final local verification runs the complete relevant automated suites, a real
native macOS package build, and the Podman hosted smoke test. After each real
run, checks confirm that no managed temporary directory or smoke image remains.
The final repository state contains no Cargo target tree or development app
bundle and retains only `dist/macos/Mote.app` as the local macOS package.
