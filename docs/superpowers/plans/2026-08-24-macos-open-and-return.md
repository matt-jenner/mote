# macOS Open and Return implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver checkpoint 1 as a runnable unsigned macOS Tauri application that opens one folder through a native picker, remembers the active source, restores it after restart, and persists System, Light, or Dark appearance.

**Architecture:** A new host-neutral Rust application service composes the existing catalog, library service, cache startup checks, and typed installation settings. A shared React interface depends only on a `PhotoService` TypeScript contract. The Tauri host implements that contract with three narrow commands and keeps native paths out of the WebView.

**Tech Stack:** Rust 1.97.1 and edition 2024; SQLite through the existing `photo-catalog`; Tauri Rust 2.11.5; Tauri build 2.6.3; Tauri CLI 2.11.4; dialog plugin 2.7.2; Node.js 24.18.0; npm 11.16.0; React 19.2.8; TypeScript 7.0.2; Vite 8.2.2; TanStack Query 5.102.2; Vitest 4.1.11 with Playwright WebKit; Biome 2.5.10.

**Spec:** `docs/superpowers/specs/2026-08-24-macos-browse-vertical-slice-design.md`

## Global Constraints

- This plan implements checkpoint 1, Open and Return, only. Progressive indexing and the photo wall begin in the next checkpoint plan.
- Every task follows test-driven development. Run the named failing test before adding production code.
- Every feature change must finish in the running macOS application. Backend-only green tests do not complete this checkpoint.
- Source media is read-only. No production code may write, rename, move, or delete anything below a selected source.
- The WebView receives stable IDs, display names, availability, and settings. It never receives authority to read a native path.
- React components import only the `PhotoService` contract. Only `tauriPhotoService.ts` imports `@tauri-apps/api`.
- The desktop host opens no local HTTP port.
- System appearance is the default. Light and Dark are persisted overrides.
- Tauri local state uses `app_data_dir()` for SQLite and `app_cache_dir()` for generated cache data.
- A named development profile creates separate data and cache subdirectories. It never deletes or renames the default profile.
- The development bundle identifier is `app.photoviewer.desktop` and the visible application name is `Photo Viewer`.
- The first application bundle is unsigned, native-architecture, and local only.
- The selected Canvas First mockup at `docs/superpowers/specs/assets/2026-08-24-macos-canvas-first.png` controls visual hierarchy and tone.
- Shared styles must render at 1440 by 1024, 834 by 1194, and 390 by 844 CSS pixels.
- Preserve all 73 existing Rust tests and the read-only-source invariant.
- Pin direct npm dependencies to the exact versions in this plan and commit `package-lock.json`.
- Keep the Tauri crate outside the root Cargo workspace for this macOS checkpoint. The existing cross-platform Rust matrix must not acquire WebKit system-library requirements.
- Checkpoint 1 uses Tauri's development icon. A custom application icon belongs to the later signed-distribution checkpoint.

## File map

### Shared Rust

- `crates/core/src/local_state.rs`: canonical local-state validation and private directory creation shared by server and desktop assembly.
- `crates/domain/src/settings.rs`: storage-independent `Appearance` enum.
- `crates/catalog/migrations/0003_app_state.sql`: one typed installation-state row.
- `crates/catalog/src/settings_repo.rs`: appearance and active-selection persistence.
- `crates/app-service`: host-neutral startup, bootstrap, folder opening, and setting updates.

### Shared interface

- `package.json` and `package-lock.json`: npm workspace and pinned frontend tools.
- `apps/interface/src/services`: `PhotoService`, in-memory adapter, and Tauri adapter.
- `apps/interface/src/app`: query-backed application controller and service context.
- `apps/interface/src/components`: Canvas First shell, source rail, empty or selected source state, and appearance menu.
- `apps/interface/src/styles`: semantic light and dark tokens plus responsive layout.
- `apps/interface/src/**/*.browser.test.tsx`: real WebKit component and accessibility checks.

### macOS host

- `apps/desktop/src-tauri`: excluded Cargo project that owns local profile resolution, managed application service, command mapping, native dialog, Tauri configuration, and `.app` build.
- `apps/desktop/package.json`: Tauri CLI scripts only; it contains no interface implementation.
- `.github/workflows/ci.yml`: interface checks plus a macOS application build.
- `README.md`: exact local development, clean-profile, test, and bundle commands.

---

### Task 1: Share local-state path safety

**Files:**
- Create: `crates/core/src/local_state.rs`
- Create: `crates/core/tests/local_state.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/server/src/config.rs`
- Modify: `crates/server/Cargo.toml`
- Test: `crates/server/tests/health_api.rs`

**Interfaces:**
- Consumes: Existing server path-normalization behavior and source-overlap rules.
- Produces: `LocalStatePaths::new(PathBuf, PathBuf)`, `data_dir()`, `cache_dir()`, `catalog_path()`, `validate_source_roots(&[PathBuf])`, and `prepare(&[PathBuf])`.

- [ ] **Step 1: Write the failing core tests**

```rust
// crates/core/tests/local_state.rs
use photo_core::{LocalStateError, LocalStatePaths};

#[test]
fn validation_happens_before_any_local_directory_is_created() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let state = LocalStatePaths::new(
        source.join("app-data"),
        temp.path().join("cache-not-created"),
    );

    assert!(matches!(
        state.prepare(std::slice::from_ref(&source)),
        Err(LocalStateError::InsideSourceRoot)
    ));
    assert!(!state.cache_dir().exists());
}

#[cfg(unix)]
#[test]
fn symlink_aliases_are_resolved_before_overlap_checks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let alias = temp.path().join("photos-alias");
    std::fs::create_dir(&source).unwrap();
    symlink(&source, &alias).unwrap();
    let state = LocalStatePaths::new(alias.join("data"), temp.path().join("cache"));

    assert!(matches!(
        state.validate_source_roots(&[source]),
        Err(LocalStateError::InsideSourceRoot)
    ));
}

#[cfg(unix)]
#[test]
fn prepared_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let state = LocalStatePaths::new(temp.path().join("data"), temp.path().join("cache"));
    state.prepare(&[]).unwrap();

    for path in [state.data_dir(), state.cache_dir()] {
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
```

- [ ] **Step 2: Run the core test and verify the missing API failure**

Run: `cargo test -p photo-core --test local_state`

Expected: FAIL because `LocalStatePaths` and `LocalStateError` are not exported.

- [ ] **Step 3: Implement the shared path guard**

```rust
// crates/core/src/local_state.rs
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct LocalStatePaths {
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum LocalStateError {
    #[error("catalog data and cache directories must not overlap a source root")]
    InsideSourceRoot,
    #[error("local-state filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
}

impl LocalStatePaths {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self { data_dir, cache_dir }
    }

    pub fn data_dir(&self) -> &Path { &self.data_dir }
    pub fn cache_dir(&self) -> &Path { &self.cache_dir }
    pub fn catalog_path(&self) -> PathBuf { self.data_dir.join("catalog.sqlite") }

    pub fn validate_source_roots(&self, sources: &[PathBuf]) -> Result<(), LocalStateError> {
        let data = resolve_for_comparison(&self.data_dir)?;
        let cache = resolve_for_comparison(&self.cache_dir)?;
        let catalog = resolve_for_comparison(&self.catalog_path())?;
        for source in sources {
            let source = resolve_for_comparison(source)?;
            if paths_overlap(&data, &source)
                || paths_overlap(&cache, &source)
                || paths_overlap(&catalog, &source)
            {
                return Err(LocalStateError::InsideSourceRoot);
            }
        }
        Ok(())
    }

    pub fn prepare(&self, sources: &[PathBuf]) -> Result<(), LocalStateError> {
        self.validate_source_roots(sources)?;
        create_private_directory(&self.data_dir)?;
        create_private_directory(&self.cache_dir)?;
        Ok(())
    }
}
```

Move `normalize_absolute`, `resolve_for_comparison`, case-aware `path_starts_with`, `paths_overlap`, and `create_private_directory` from `crates/server/src/config.rs` into this file without changing their existing bodies. Export `LocalStateError` and `LocalStatePaths` from `crates/core/src/lib.rs`.

- [ ] **Step 4: Refactor the server configuration to compose the shared guard**

```rust
// relevant shape in crates/server/src/config.rs
use photo_core::{LocalStateError, LocalStatePaths};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    local: LocalStatePaths,
    bind: SocketAddr,
    source_roots: Vec<PathBuf>,
}

impl ServerConfig {
    pub fn new(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        bind: Option<&str>,
        source_roots: Vec<PathBuf>,
    ) -> Result<Self, ConfigError> {
        Ok(Self {
            local: LocalStatePaths::new(data_dir, cache_dir),
            bind: bind.unwrap_or("127.0.0.1:8080").parse()?,
            source_roots,
        })
    }

    pub fn prepare(&self) -> Result<(), ConfigError> {
        self.local
            .prepare(&self.source_roots)
            .map_err(ConfigError::from_local_state)
    }

    pub fn validate_source_roots(&self, roots: &[PathBuf]) -> Result<(), ConfigError> {
        self.local
            .validate_source_roots(roots)
            .map_err(ConfigError::from_local_state)
    }

    pub fn data_dir(&self) -> &Path { self.local.data_dir() }
    pub fn cache_dir(&self) -> &Path { self.local.cache_dir() }
    pub fn catalog_path(&self) -> PathBuf { self.local.catalog_path() }
}

impl ConfigError {
    fn from_local_state(error: LocalStateError) -> Self {
        match error {
            LocalStateError::InsideSourceRoot => Self::InsideSourceRoot,
            LocalStateError::Io(error) => Self::Io(error),
        }
    }
}
```

Keep `ConfigError::InsideSourceRoot` and `ConfigError::Io` stable so the existing health tests remain meaningful. Add `photo-core` to `crates/server/Cargo.toml`. Remove only the helper bodies now owned by `photo-core`.

- [ ] **Step 5: Run focused and workspace verification**

Run: `cargo test -p photo-core --test local_state`

Expected: 3 tests PASS on macOS and Linux; 1 platform-neutral test PASS on Windows.

Run: `cargo test -p photo-server --test health_api`

Expected: 9 tests PASS.

Run: `cargo fmt --all --check`

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Run: `cargo test --workspace --all-features`

Expected: PASS with no regression to the existing 73 tests.

- [ ] **Step 6: Commit the shared guard**

```bash
git add crates/core crates/server Cargo.lock
git commit -m "refactor: share local state path safety"
```

---

### Task 2: Persist appearance and the active source

**Files:**
- Create: `crates/domain/src/settings.rs`
- Create: `crates/catalog/migrations/0003_app_state.sql`
- Create: `crates/catalog/src/settings_repo.rs`
- Create: `crates/catalog/tests/settings_round_trip.rs`
- Modify: `crates/domain/src/lib.rs`
- Modify: `crates/catalog/src/lib.rs`
- Modify: `crates/catalog/src/migrate.rs`
- Modify: `crates/core/src/library_service.rs`

**Interfaces:**
- Consumes: `LibraryId`, `RelativePathKey`, and the existing migrated `Catalog`.
- Produces: `Appearance`, `StoredSourceSelection`, `AppStateRecord`, `Catalog::load_app_state`, `Catalog::set_appearance`, `Catalog::set_active_selection`, and `LibraryService::catalog_mut`.

- [ ] **Step 1: Write the failing persistence test**

```rust
// crates/catalog/tests/settings_round_trip.rs
use std::path::Path;
use photo_catalog::{Catalog, NewLibrary, StoredSourceSelection};
use photo_domain::{Appearance, RelativePathKey};

#[test]
fn appearance_and_active_selection_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let library = catalog
        .add_library(&NewLibrary::recent("Family", Path::new("/Volumes/Family")))
        .unwrap();
    let selection = StoredSourceSelection {
        library_id: library.id,
        relative_folder: RelativePathKey::from_relative_path(Path::new("")).unwrap(),
    };

    catalog.set_appearance(Appearance::Dark).unwrap();
    catalog.set_active_selection(Some(&selection)).unwrap();
    drop(catalog);

    let catalog = Catalog::open(&path).unwrap();
    let state = catalog.load_app_state().unwrap();
    assert_eq!(state.appearance, Appearance::Dark);
    assert_eq!(state.active_selection, Some(selection));
}

#[test]
fn a_new_catalog_defaults_to_system_without_a_selection() {
    let catalog = Catalog::open_in_memory().unwrap();
    assert_eq!(catalog.load_app_state().unwrap().appearance, Appearance::System);
    assert_eq!(catalog.load_app_state().unwrap().active_selection, None);
}
```

- [ ] **Step 2: Run the test and verify the missing types failure**

Run: `cargo test -p photo-catalog --test settings_round_trip`

Expected: FAIL because `Appearance`, `StoredSourceSelection`, and the settings repository do not exist.

- [ ] **Step 3: Add the storage-independent appearance type**

```rust
// crates/domain/src/settings.rs
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
```

Export `Appearance` from `crates/domain/src/lib.rs`.

- [ ] **Step 4: Add migration 3**

```sql
-- crates/catalog/migrations/0003_app_state.sql
CREATE TABLE app_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  appearance TEXT NOT NULL CHECK(appearance IN ('system', 'light', 'dark'))
);

CREATE TABLE active_source_selection (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1)
    REFERENCES app_state(singleton) ON DELETE CASCADE,
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  relative_folder_key BLOB NOT NULL
);

INSERT INTO app_state (singleton, appearance) VALUES (1, 'system');

PRAGMA user_version = 3;
```

Append `include_str!("../migrations/0003_app_state.sql")` to the ordered migration list in `crates/catalog/src/migrate.rs`.

- [ ] **Step 5: Implement the typed settings repository**

```rust
// crates/catalog/src/settings_repo.rs
use photo_domain::{Appearance, LibraryId, RelativePathKey};
use rusqlite::params;

use crate::library_repo::decode_uuid;
use crate::{Catalog, CatalogError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSourceSelection {
    pub library_id: LibraryId,
    pub relative_folder: RelativePathKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppStateRecord {
    pub appearance: Appearance,
    pub active_selection: Option<StoredSourceSelection>,
}

impl Catalog {
    pub fn load_app_state(&self) -> Result<AppStateRecord, CatalogError> {
        let (appearance, library_id, relative_folder): (
            String,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
        ) = self.connection.query_row(
            "SELECT app_state.appearance, active.library_id, active.relative_folder_key \
             FROM app_state \
             LEFT JOIN active_source_selection AS active USING (singleton) \
             WHERE app_state.singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        let appearance = decode_appearance(&appearance)?;
        let active_selection = match (library_id, relative_folder) {
            (None, None) => None,
            (Some(library_id), Some(relative_folder)) => Some(StoredSourceSelection {
                library_id: LibraryId::from_uuid(decode_uuid(library_id, 1)?),
                relative_folder: RelativePathKey::from_bytes(relative_folder).map_err(|error| {
                    CatalogError::InvalidData(format!("invalid active folder key: {error}"))
                })?,
            }),
            _ => {
                return Err(CatalogError::InvalidData(
                    "active source selection is incomplete".into(),
                ));
            }
        };

        Ok(AppStateRecord { appearance, active_selection })
    }

    pub fn set_appearance(&mut self, appearance: Appearance) -> Result<(), CatalogError> {
        let changed = self.connection.execute(
            "UPDATE app_state SET appearance = ?1 WHERE singleton = 1",
            [encode_appearance(appearance)],
        )?;
        require_singleton(changed)
    }

    pub fn set_active_selection(
        &mut self,
        selection: Option<&StoredSourceSelection>,
    ) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM active_source_selection WHERE singleton = 1", [])?;
        if let Some(selection) = selection {
            transaction.execute(
                "INSERT INTO active_source_selection \
                 (singleton, library_id, relative_folder_key) VALUES (1, ?1, ?2)",
                params![
                    selection.library_id.as_uuid().as_bytes(),
                    selection.relative_folder.as_bytes(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

fn encode_appearance(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

fn decode_appearance(value: &str) -> Result<Appearance, CatalogError> {
    match value {
        "system" => Ok(Appearance::System),
        "light" => Ok(Appearance::Light),
        "dark" => Ok(Appearance::Dark),
        other => Err(CatalogError::InvalidData(format!(
            "unknown appearance {other}"
        ))),
    }
}

fn require_singleton(changed: usize) -> Result<(), CatalogError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(CatalogError::InvalidData("app state row is missing".into()))
    }
}
```

Add this private repository test. It verifies that deleting a library clears the separate selection row through the foreign key. Do not expose arbitrary SQL or a production delete API for the test.

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    use photo_domain::RelativePathKey;
    use rusqlite::params;

    use super::StoredSourceSelection;
    use crate::{Catalog, NewLibrary};

    #[test]
    fn deleting_the_active_library_clears_the_selection() {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::recent("Family", Path::new("/Volumes/Family")))
            .unwrap();
        catalog
            .set_active_selection(Some(&StoredSourceSelection {
                library_id: library.id,
                relative_folder: RelativePathKey::from_relative_path(Path::new("")).unwrap(),
            }))
            .unwrap();

        catalog
            .connection
            .execute(
                "DELETE FROM library_roots WHERE id = ?1",
                params![library.id.as_uuid().as_bytes()],
            )
            .unwrap();

        assert_eq!(catalog.load_app_state().unwrap().active_selection, None);
    }
}
```

Export all three types from `crates/catalog/src/lib.rs`. Add this narrow accessor for application-service composition:

```rust
// crates/core/src/library_service.rs
pub fn catalog_mut(&mut self) -> &mut Catalog {
    &mut self.catalog
}
```

- [ ] **Step 6: Run the persistence and migration tests**

Run: `cargo test -p photo-catalog --test settings_round_trip`

Expected: PASS.

Run: `cargo test -p photo-catalog`

Expected: All catalog tests PASS, including migration rollback.

- [ ] **Step 7: Commit installation-state persistence**

```bash
git add crates/domain crates/catalog crates/core Cargo.lock
git commit -m "feat: persist desktop installation state"
```

---

### Task 3: Add the host-neutral application service

**Files:**
- Create: `crates/app-service/Cargo.toml`
- Create: `crates/app-service/src/lib.rs`
- Create: `crates/app-service/src/config.rs`
- Create: `crates/app-service/src/dto.rs`
- Create: `crates/app-service/src/service.rs`
- Create: `crates/app-service/tests/open_and_return.rs`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: `LocalStatePaths`, `Catalog`, `CacheWriter`, `LibraryService<RealSourceFs>`, `Appearance`, and `StoredSourceSelection`.
- Produces: `AppConfig`, `AppService::open`, `bootstrap`, `open_recent`, `update_appearance`, `BootstrapState`, `SettingsState`, `SourceSummary`, `SourceAvailability`, and `AppServiceError`.

- [ ] **Step 1: Write the failing reopen test**

```rust
// crates/app-service/tests/open_and_return.rs
use photo_app_service::{AppConfig, AppService};
use photo_domain::Appearance;

#[test]
fn source_and_appearance_restore_from_the_same_profile() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Iceland 2025");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let mut first = AppService::open(config.clone()).unwrap();
    let selected = first.open_recent(&photos).unwrap();
    assert_eq!(selected.active_source.unwrap().display_name, "Iceland 2025");
    first.update_appearance(Appearance::Dark).unwrap();
    drop(first);

    let reopened = AppService::open(config).unwrap();
    let state = reopened.bootstrap().unwrap();
    assert_eq!(state.settings.appearance, Appearance::Dark);
    assert_eq!(state.active_source.unwrap().display_name, "Iceland 2025");
}

#[test]
fn bootstrap_does_not_expose_a_native_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Private Folder Name");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let mut service = AppService::open(config).unwrap();
    let state = service.open_recent(&photos).unwrap();

    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("Private Folder Name"));
    assert!(!json.contains(&temp.path().to_string_lossy().to_string()));
}
```

- [ ] **Step 2: Run the service test and verify the missing crate failure**

Run: `cargo test -p photo-app-service --test open_and_return`

Expected: FAIL because `photo-app-service` does not exist.

- [ ] **Step 3: Create the crate and exact DTOs**

```toml
# crates/app-service/Cargo.toml
[package]
name = "photo-app-service"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
photo-cache = { path = "../cache" }
photo-catalog = { path = "../catalog" }
photo-core = { path = "../core" }
photo-domain = { path = "../domain" }
serde.workspace = true
thiserror.workspace = true

[dev-dependencies]
serde_json.workspace = true
tempfile.workspace = true
```

```rust
// crates/app-service/src/dto.rs
use photo_domain::Appearance;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub settings: SettingsState,
    pub active_source: Option<SourceSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsState {
    pub appearance: Appearance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: String,
    pub display_name: String,
    pub availability: SourceAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceAvailability {
    Available,
    RootOffline,
    Missing,
    Unreadable,
}
```

Wire the crate root explicitly:

```rust
// crates/app-service/src/lib.rs
mod config;
mod dto;
mod service;

pub use config::AppConfig;
pub use dto::{BootstrapState, SettingsState, SourceAvailability, SourceSummary};
pub use service::{AppService, AppServiceError};
```

- [ ] **Step 4: Implement safe service startup**

```rust
// crates/app-service/src/config.rs
#[derive(Clone, Debug)]
pub struct AppConfig {
    local: LocalStatePaths,
}

impl AppConfig {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self { local: LocalStatePaths::new(data_dir, cache_dir) }
    }
    pub fn data_dir(&self) -> &Path { self.local.data_dir() }
    pub fn cache_dir(&self) -> &Path { self.local.cache_dir() }
    pub fn catalog_path(&self) -> PathBuf { self.local.catalog_path() }
    pub(crate) fn validate_source_roots(
        &self,
        roots: &[PathBuf],
    ) -> Result<(), LocalStateError> {
        self.local.validate_source_roots(roots)
    }
    pub(crate) fn prepare(&self, roots: &[PathBuf]) -> Result<(), LocalStateError> {
        self.local.prepare(roots)
    }
}
```

Implement `AppServiceError` and `AppService::open` in this exact order:

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppServiceError {
    #[error("local state setup failed: {0}")]
    LocalState(#[from] LocalStateError),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] CatalogError),
    #[error("cache setup failed: {0}")]
    Cache(#[from] CacheError),
    #[error("library service setup failed: {0}")]
    LibrarySetup(#[source] std::io::Error),
    #[error("folder selection failed: {0}")]
    OpenRecent(#[from] AddLibraryError),
}

impl AppService {
    pub fn open(config: AppConfig) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;
        Ok(Self { libraries })
    }
}
```

The error retains the underlying value for logs. The Tauri layer maps it to bounded user messages.

- [ ] **Step 5: Implement bootstrap, folder opening, and appearance updates**

```rust
use photo_catalog::StoredSourceSelection;
use photo_domain::{Appearance, Availability};

pub struct AppService {
    libraries: LibraryService<RealSourceFs>,
}

impl AppService {
    pub fn bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        let stored = self.libraries.catalog().load_app_state()?;
        let active_source = match stored.active_selection {
            Some(selection) => self
                .libraries
                .catalog()
                .find_library(selection.library_id)?
                .map(|library| SourceSummary {
                    id: library.id.as_uuid().hyphenated().to_string(),
                    display_name: library.display_name,
                    availability: map_availability(library.availability),
                }),
            None => None,
        };
        Ok(BootstrapState {
            settings: SettingsState { appearance: stored.appearance },
            active_source,
        })
    }

    pub fn open_recent(&mut self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let selection = self.libraries.open_recent(folder)?;
        self.libraries
            .catalog_mut()
            .set_active_selection(Some(&StoredSourceSelection {
                library_id: selection.library_id,
                relative_folder: selection.relative_folder,
            }))?;
        self.bootstrap()
    }

    pub fn update_appearance(
        &mut self,
        appearance: Appearance,
    ) -> Result<BootstrapState, AppServiceError> {
        self.libraries.catalog_mut().set_appearance(appearance)?;
        self.bootstrap()
    }
}

fn map_availability(availability: Availability) -> SourceAvailability {
    match availability {
        Availability::Available => SourceAvailability::Available,
        Availability::RootOffline => SourceAvailability::RootOffline,
        Availability::Missing => SourceAvailability::Missing,
        Availability::Unreadable => SourceAvailability::Unreadable,
    }
}
```

`open_recent` never scans, opens, or writes a file below the chosen folder in this checkpoint.

- [ ] **Step 6: Run service and workspace tests**

Run: `cargo test -p photo-app-service --test open_and_return`

Expected: 2 tests PASS.

Run: `cargo test --workspace --all-features`

Expected: all existing tests plus the new local-state, settings, and app-service tests PASS.

- [ ] **Step 7: Commit the application service**

```bash
git add crates/app-service Cargo.lock
git commit -m "feat: add desktop application service"
```

---

### Task 4: Establish the isolated TypeScript service contract

**Files:**
- Create: `package.json`
- Create: `apps/interface/package.json`
- Create: `apps/interface/tsconfig.json`
- Create: `apps/interface/vite.config.ts`
- Create: `apps/interface/vitest.config.ts`
- Create: `apps/interface/src/services/photoService.ts`
- Create: `apps/interface/src/services/inMemoryPhotoService.ts`
- Create: `apps/interface/src/services/photoService.test.ts`
- Modify: `.gitignore`
- Generate: `package-lock.json`

**Interfaces:**
- Consumes: The JSON shape of `BootstrapState` from Task 3.
- Produces: `PhotoService`, `BootstrapState`, `ChooseFolderResult`, `Appearance`, `SourceSummary`, `PhotoServiceError`, and `createInMemoryPhotoService`.

- [ ] **Step 1: Add the pinned npm workspace manifests**

Root `package.json`:

```json
{
  "name": "photo-viewer",
  "private": true,
  "workspaces": ["apps/*"],
  "engines": {
    "node": ">=24.18.0",
    "npm": ">=11.16.0"
  },
  "scripts": {
    "check": "biome check apps",
    "check:write": "biome check --write apps",
    "typecheck": "npm run typecheck --workspace @photo-viewer/interface",
    "test": "npm run test --workspace @photo-viewer/interface",
    "test:browser": "npm run test:browser --workspace @photo-viewer/interface"
  },
  "devDependencies": {
    "@testing-library/jest-dom": "7.0.1",
    "@testing-library/react": "16.3.2",
    "@biomejs/biome": "2.5.10"
  }
}
```

`apps/interface/package.json`:

```json
{
  "name": "@photo-viewer/interface",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite --host 127.0.0.1 --port 1420",
    "dev:memory": "vite --mode memory --host 127.0.0.1 --port 1420",
    "build": "tsc -b && vite build",
    "typecheck": "tsc -b --pretty false",
    "test": "vitest run --project unit",
    "test:browser": "vitest run --project browser"
  },
  "dependencies": {
    "@tanstack/react-query": "5.102.2",
    "@tauri-apps/api": "2.11.1",
    "lucide-react": "1.34.0",
    "react": "19.2.8",
    "react-dom": "19.2.8"
  },
  "devDependencies": {
    "@types/node": "26.2.0",
    "@types/react": "19.2.18",
    "@types/react-dom": "19.2.5",
    "@vitejs/plugin-react": "6.1.0",
    "@vitest/browser-playwright": "4.1.11",
    "axe-core": "4.13.0",
    "typescript": "7.0.2",
    "vite": "8.2.2",
    "vitest": "4.1.11",
    "vitest-browser-react": "2.2.0"
  }
}
```

Add `node_modules/`, `apps/interface/dist/`, and `apps/interface/.vite/` to `.gitignore`. Run `npm install` once and commit the generated lockfile. Do not use unpinned `npx` installs.

- [ ] **Step 2: Configure strict TypeScript, Vite, and separate test projects**

`apps/interface/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "Bundler",
    "allowImportingTsExtensions": false,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "types": ["vite/client"]
  },
  "include": ["src", "vite.config.ts", "vitest.config.ts"]
}
```

Configure Vite with a strict fixed development port:

```ts
// apps/interface/vite.config.ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
});
```

Use this separate Vitest project configuration:

```ts
// apps/interface/vitest.config.ts
import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    projects: [
      {
        test: {
          name: "unit",
          include: ["src/**/*.test.ts"],
          environment: "node",
        },
      },
      {
        plugins: [react()],
        test: {
          name: "browser",
          include: ["src/**/*.browser.test.tsx"],
          browser: {
            enabled: true,
            provider: playwright(),
            headless: true,
            instances: [{ browser: "webkit" }],
          },
        },
      },
    ],
  },
});
```

- [ ] **Step 3: Write the failing adapter-contract test**

```ts
// apps/interface/src/services/photoService.test.ts
import { describe, expect, it } from "vitest";
import { createInMemoryPhotoService } from "./inMemoryPhotoService";

describe("PhotoService contract", () => {
  it("persists a selected source and appearance for the adapter lifetime", async () => {
    const service = createInMemoryPhotoService({ selectedFolderName: "Iceland 2025" });
    expect((await service.getBootstrapState()).activeSource).toBeNull();

    const chosen = await service.chooseFolder();
    expect(chosen.kind).toBe("selected");
    if (chosen.kind !== "selected") throw new Error("expected selected folder");
    expect(chosen.state.activeSource?.displayName).toBe("Iceland 2025");

    const updated = await service.updateAppearance("dark");
    expect(updated.settings.appearance).toBe("dark");
    expect((await service.getBootstrapState()).activeSource?.displayName).toBe("Iceland 2025");
  });

  it("represents picker cancellation without throwing", async () => {
    const service = createInMemoryPhotoService({ cancelFolderPicker: true });
    await expect(service.chooseFolder()).resolves.toEqual({ kind: "cancelled" });
  });
});
```

- [ ] **Step 4: Run the contract test and verify the missing module failure**

Run: `npm exec --workspace @photo-viewer/interface vitest run --project unit src/services/photoService.test.ts`

Expected: FAIL because the service modules do not exist.

- [ ] **Step 5: Implement the host-neutral contract and in-memory adapter**

```ts
// apps/interface/src/services/photoService.ts
export type Appearance = "system" | "light" | "dark";
export type SourceAvailability = "available" | "rootOffline" | "missing" | "unreadable";

export interface SettingsState {
  appearance: Appearance;
}

export interface SourceSummary {
  id: string;
  displayName: string;
  availability: SourceAvailability;
}

export interface BootstrapState {
  settings: SettingsState;
  activeSource: SourceSummary | null;
}

export type ChooseFolderResult =
  | { kind: "cancelled" }
  | { kind: "selected"; state: BootstrapState };

export interface PhotoServiceCapabilities {
  chooseFolder: boolean;
  locateFolder: boolean;
}

export interface PhotoService {
  readonly capabilities: PhotoServiceCapabilities;
  getBootstrapState(): Promise<BootstrapState>;
  chooseFolder(): Promise<ChooseFolderResult>;
  updateAppearance(appearance: Appearance): Promise<BootstrapState>;
}

export class PhotoServiceError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "PhotoServiceError";
  }
}
```

```ts
// apps/interface/src/services/inMemoryPhotoService.ts
import type {
  Appearance,
  BootstrapState,
  PhotoService,
} from "./photoService";

interface InMemoryOptions {
  selectedFolderName?: string;
  cancelFolderPicker?: boolean;
}

export function createInMemoryPhotoService(options: InMemoryOptions = {}): PhotoService {
  let state: BootstrapState = {
    settings: { appearance: "system" },
    activeSource: null,
  };
  const copy = (): BootstrapState => structuredClone(state);

  return {
    capabilities: { chooseFolder: true, locateFolder: false },
    async getBootstrapState() {
      return copy();
    },
    async chooseFolder() {
      if (options.cancelFolderPicker) return { kind: "cancelled" };
      state = {
        ...state,
        activeSource: {
          id: "memory-source",
          displayName: options.selectedFolderName ?? "Selected Folder",
          availability: "available",
        },
      };
      return { kind: "selected", state: copy() };
    },
    async updateAppearance(appearance: Appearance) {
      state = { ...state, settings: { appearance } };
      return copy();
    },
  };
}
```

- [ ] **Step 6: Run contract checks and commit**

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run check`

Expected: PASS.

```bash
git add package.json package-lock.json apps/interface .gitignore
git commit -m "feat: define isolated photo service contract"
```

---

### Task 5: Build the responsive Canvas First shell

**Files:**
- Create: `apps/interface/index.html`
- Create: `apps/interface/src/main.tsx`
- Create: `apps/interface/src/app/PhotoServiceContext.tsx`
- Create: `apps/interface/src/app/useAppController.ts`
- Create: `apps/interface/src/components/AppShell.tsx`
- Create: `apps/interface/src/components/NavigationRail.tsx`
- Create: `apps/interface/src/components/SourceCanvas.tsx`
- Create: `apps/interface/src/components/AppearanceMenu.tsx`
- Create: `apps/interface/src/components/App.browser.test.tsx`
- Create: `apps/interface/src/styles/tokens.css`
- Create: `apps/interface/src/styles/global.css`
- Create: `apps/interface/src/styles/appShell.module.css`
- Create: `apps/interface/src/theme/applyAppearance.ts`

**Interfaces:**
- Consumes: `PhotoService` and DTOs from Task 4.
- Produces: `PhotoServiceProvider`, `usePhotoService`, `useAppController`, `applyAppearance`, and the checkpoint 1 React application.

- [ ] **Step 1: Write browser tests for the visible feature**

```tsx
// apps/interface/src/components/App.browser.test.tsx
import axe from "axe-core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { beforeEach, describe, expect, it } from "vitest";
import { AppShell } from "./AppShell";
import { PhotoServiceProvider } from "../app/PhotoServiceContext";
import { createInMemoryPhotoService } from "../services/inMemoryPhotoService";

function renderApp(service = createInMemoryPhotoService({ selectedFolderName: "Iceland 2025" })) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <PhotoServiceProvider service={service}>
        <AppShell />
      </PhotoServiceProvider>
    </QueryClientProvider>,
  );
}

describe("open and return shell", () => {
  beforeEach(async () => {
    await page.viewport(1440, 1024);
  });

  it("opens a folder and shows its persisted display name", async () => {
    const screen = renderApp();
    await screen.getByRole("button", { name: "Choose Folder" }).click();
    await expect.element(screen.getByText("Iceland 2025")).toBeVisible();
    await expect.element(screen.getByText("Folder ready")).toBeVisible();
  });

  it("applies an explicit dark override", async () => {
    const screen = renderApp();
    await screen.getByRole("button", { name: "Appearance" }).click();
    await screen.getByRole("radio", { name: "Dark" }).click();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("uses a drawer trigger instead of a permanent rail on a phone", async () => {
    await page.viewport(390, 844);
    const screen = renderApp();
    await expect.element(screen.getByRole("button", { name: "Open sources" })).toBeVisible();
    await expect.element(screen.getByRole("navigation", { name: "Sources" })).not.toBeVisible();
  });

  it("renders its canvas at the approved desktop, tablet, and phone sizes", async () => {
    for (const [width, height] of [[1440, 1024], [834, 1194], [390, 844]] as const) {
      await page.viewport(width, height);
      const screen = renderApp();
      await expect.element(screen.getByRole("main")).toBeVisible();
      screen.unmount();
    }
  });

  it("has no serious accessibility violations", async () => {
    renderApp();
    const result = await axe.run(document);
    expect(result.violations.filter((violation) => violation.impact === "serious" || violation.impact === "critical")).toEqual([]);
  });
});
```

- [ ] **Step 2: Run the browser test and verify the missing component failure**

Run: `npm run test:browser`

Expected: FAIL because `AppShell` and the service context do not exist.

- [ ] **Step 3: Implement the service context and query-backed controller**

```tsx
// apps/interface/src/app/PhotoServiceContext.tsx
const PhotoServiceContext = createContext<PhotoService | null>(null);

export function PhotoServiceProvider({ service, children }: PropsWithChildren<{ service: PhotoService }>) {
  return <PhotoServiceContext.Provider value={service}>{children}</PhotoServiceContext.Provider>;
}

export function usePhotoService(): PhotoService {
  const service = useContext(PhotoServiceContext);
  if (!service) throw new Error("PhotoServiceProvider is missing");
  return service;
}
```

```ts
// apps/interface/src/app/useAppController.ts
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { Appearance, BootstrapState } from "../services/photoService";
import { applyAppearance } from "../theme/applyAppearance";
import { usePhotoService } from "./PhotoServiceContext";

const bootstrapKey = ["bootstrap"] as const;

export function useAppController() {
  const service = usePhotoService();
  const queryClient = useQueryClient();
  const bootstrap = useQuery({
    queryKey: bootstrapKey,
    queryFn: () => service.getBootstrapState(),
  });
  useEffect(() => {
    if (bootstrap.data) applyAppearance(bootstrap.data.settings.appearance);
  }, [bootstrap.data]);

  const folder = useMutation({
    mutationFn: () => service.chooseFolder(),
    onSuccess(result) {
      if (result.kind === "selected") {
        queryClient.setQueryData<BootstrapState>(bootstrapKey, result.state);
      }
    },
  });
  const appearance = useMutation({
    mutationFn: (value: Appearance) => service.updateAppearance(value),
    onSuccess(state) {
      queryClient.setQueryData<BootstrapState>(bootstrapKey, state);
      applyAppearance(state.settings.appearance);
    },
  });

  return {
    state: bootstrap.data,
    loading: bootstrap.isPending,
    error: bootstrap.error ?? folder.error ?? appearance.error,
    capabilities: service.capabilities,
    chooseFolder: folder.mutate,
    updateAppearance: appearance.mutate,
  };
}
```

- [ ] **Step 4: Implement theme application and semantic tokens**

```ts
// apps/interface/src/theme/applyAppearance.ts
import type { Appearance } from "../services/photoService";

export function applyAppearance(appearance: Appearance): void {
  document.documentElement.dataset.theme = appearance;
  document.documentElement.style.colorScheme = appearance === "system" ? "light dark" : appearance;
}
```

Use these exact semantic tokens. System defaults to the light block and switches inside `prefers-color-scheme: dark`. Explicit Light and Dark reuse the same values outside media queries.

```css
/* apps/interface/src/styles/tokens.css */
:root,
:root[data-theme="system"],
:root[data-theme="light"] {
  --canvas: #f4f3ef;
  --canvas-elevated: #ffffff;
  --rail: #ebe9e3;
  --text-primary: #171816;
  --text-secondary: #686a66;
  --separator: #d7d5cf;
  --selection: #dce6f2;
  --focus-ring: #245f99;
  --progress-accent: #326aa3;
  --control: #ffffff;
  --control-hover: #e3e1db;
  --unavailable: #858780;
}

:root[data-theme="dark"] {
  --canvas: #1a1b1b;
  --canvas-elevated: #242625;
  --rail: #121313;
  --text-primary: #f0f0eb;
  --text-secondary: #a5a7a1;
  --separator: #343735;
  --selection: #283a4f;
  --focus-ring: #7fb5f0;
  --progress-accent: #6c9edb;
  --control: #2a2c2b;
  --control-hover: #363937;
  --unavailable: #777b76;
}

@media (prefers-color-scheme: dark) {
  :root[data-theme="system"] {
    --canvas: #1a1b1b;
    --canvas-elevated: #242625;
    --rail: #121313;
    --text-primary: #f0f0eb;
    --text-secondary: #a5a7a1;
    --separator: #343735;
    --selection: #283a4f;
    --focus-ring: #7fb5f0;
    --progress-accent: #6c9edb;
    --control: #2a2c2b;
    --control-hover: #363937;
    --unavailable: #777b76;
  }
}

:root {
  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-6: 24px;
  --radius-small: 6px;
  --radius-medium: 10px;
  --image-gap: 4px;
  --fade-duration: 160ms;
}
```

All focusable controls use `outline: 2px solid var(--focus-ring)` with a 2 CSS pixel offset under `:focus-visible`. `global.css` uses the platform system font, safe-area padding, and 44 CSS pixel minimum touch targets below 640 CSS pixels. Under `prefers-reduced-motion: reduce`, set `--fade-duration: 0ms` and disable smooth scrolling.

- [ ] **Step 5: Implement the checkpoint shell**

`AppShell` has one `main` region, one source navigation region at desktop width, and one compact toolbar. Before selection it shows the exact primary action `Choose Folder`. After selection it shows the source display name and the status `Folder ready`. It does not claim that indexing has begun.

`AppearanceMenu` uses one button named `Appearance` and a radio group containing `System`, `Light`, and `Dark`. The control stays behind the toolbar action rather than occupying permanent space.

At widths below 640 CSS pixels, hide the permanent rail and show a button named `Open sources`. Opening it renders the same source controls in a modal drawer. At 640 through 899 pixels use the compact 56 CSS pixel rail. At 900 pixels and above use a 72 CSS pixel icon rail for this checkpoint. Do not add the expanded 220 pixel library rail until source navigation needs it.

- [ ] **Step 6: Add the application entry point**

```tsx
// apps/interface/src/main.tsx
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Number.POSITIVE_INFINITY, retry: false } },
});

const service = createInMemoryPhotoService({ cancelFolderPicker: true });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <PhotoServiceProvider service={service}>
        <AppShell />
      </PhotoServiceProvider>
    </QueryClientProvider>
  </StrictMode>,
);
```

This temporary composition root makes `npm run dev --workspace @photo-viewer/interface` a browser-previewable interface. Task 6 replaces only the selected adapter for Tauri builds; components remain unchanged.

- [ ] **Step 7: Verify desktop, tablet, phone, and accessibility states**

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run test:browser`

Run: `npm run check`

Run: `npm run --workspace @photo-viewer/interface build`

Expected: PASS. Browser tests run in headless WebKit at all three named dimensions.

- [ ] **Step 8: Commit the Canvas First shell**

```bash
git add apps/interface package-lock.json
git commit -m "feat: add responsive canvas first shell"
```

---

### Task 6: Embed the service in a Tauri macOS host

**Files:**
- Create: `apps/interface/src/services/tauriPhotoService.ts`
- Create: `apps/interface/src/services/tauriPhotoService.test.ts`
- Modify: `apps/interface/src/main.tsx`
- Create: `apps/desktop/package.json`
- Create: `apps/desktop/src-tauri/Cargo.toml`
- Create: `apps/desktop/src-tauri/Cargo.lock`
- Create: `apps/desktop/src-tauri/build.rs`
- Create: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `apps/desktop/src-tauri/capabilities/default.json`
- Create: `apps/desktop/src-tauri/src/main.rs`
- Create: `apps/desktop/src-tauri/src/lib.rs`
- Create: `apps/desktop/src-tauri/src/commands.rs`
- Create: `apps/desktop/src-tauri/src/dto.rs`
- Create: `apps/desktop/src-tauri/src/profile.rs`
- Create: `apps/desktop/src-tauri/src/state.rs`
- Modify: `Cargo.toml`
- Modify: `package.json`
- Modify: `package-lock.json`

**Interfaces:**
- Consumes: `AppService`, `BootstrapState`, `Appearance`, and the TypeScript `PhotoService` contract.
- Produces: Tauri commands `get_bootstrap_state`, `choose_folder`, and `update_appearance`; `createTauriPhotoService`; `ProfileRoots`; and the unsigned local application host.

- [ ] **Step 1: Write the failing Tauri-adapter test**

```ts
// apps/interface/src/services/tauriPhotoService.test.ts
import { describe, expect, it } from "vitest";
import { createTauriPhotoService, type InvokeCommand } from "./tauriPhotoService";

describe("Tauri PhotoService", () => {
  it("uses only the three checkpoint commands", async () => {
    const responses: unknown[] = [
      { settings: { appearance: "system" }, activeSource: null },
      { kind: "cancelled" },
      { settings: { appearance: "dark" }, activeSource: null },
    ];
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const invoke: InvokeCommand = async <T>(command, args) => {
      calls.push([command, args]);
      return responses.shift() as T;
    };
    const service = createTauriPhotoService(invoke);

    await service.getBootstrapState();
    await service.chooseFolder();
    await service.updateAppearance("dark");

    expect(calls).toEqual([
      ["get_bootstrap_state", undefined],
      ["choose_folder", undefined],
      ["update_appearance", { appearance: "dark" }],
    ]);
  });
});
```

- [ ] **Step 2: Run the adapter test and verify the missing module failure**

Run: `npm exec --workspace @photo-viewer/interface vitest run --project unit src/services/tauriPhotoService.test.ts`

Expected: FAIL because `tauriPhotoService.ts` does not exist.

- [ ] **Step 3: Implement the single Tauri import boundary**

```ts
// apps/interface/src/services/tauriPhotoService.ts
import { invoke } from "@tauri-apps/api/core";
import type { Appearance, BootstrapState, ChooseFolderResult, PhotoService } from "./photoService";

export type InvokeCommand = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function createTauriPhotoService(invokeCommand: InvokeCommand = invoke): PhotoService {
  return {
    capabilities: { chooseFolder: true, locateFolder: false },
    getBootstrapState: () => invokeCommand<BootstrapState>("get_bootstrap_state", undefined),
    chooseFolder: () => invokeCommand<ChooseFolderResult>("choose_folder", undefined),
    updateAppearance: (appearance: Appearance) =>
      invokeCommand<BootstrapState>("update_appearance", { appearance }),
  };
}
```

Change `main.tsx` to choose the in-memory adapter only when `import.meta.env.MODE === "memory"`; otherwise create the Tauri adapter. The `dev:memory` interface script selects that Vite mode without shell-specific environment syntax. Do not detect Tauri through globals or hostnames.

- [ ] **Step 4: Write failing profile-root tests**

```rust
// unit tests in apps/desktop/src-tauri/src/profile.rs
#[test]
fn named_profile_isolated_below_both_app_roots() {
    let roots = ProfileRoots::from_bases(
        Path::new("/app-data"),
        Path::new("/app-cache"),
        Some("checkpoint-1"),
    ).unwrap();
    assert_eq!(roots.data_dir, PathBuf::from("/app-data/profiles/checkpoint-1"));
    assert_eq!(roots.cache_dir, PathBuf::from("/app-cache/profiles/checkpoint-1"));
}

#[test]
fn profile_names_reject_separators_and_empty_values() {
    for value in ["", "../escape", "a/b", "a b", "."] {
        assert!(ProfileRoots::from_bases(Path::new("data"), Path::new("cache"), Some(value)).is_err());
    }
}
```

- [ ] **Step 5: Create the excluded Tauri project**

Add `exclude = ["apps/desktop/src-tauri"]` under the root `[workspace]`. The Tauri crate has its own `Cargo.lock` and this manifest:

```toml
# apps/desktop/src-tauri/Cargo.toml
[package]
name = "photo-viewer-desktop"
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"

[workspace]

[lib]
name = "photo_viewer_desktop_lib"
crate-type = ["lib", "cdylib", "staticlib"]

[build-dependencies]
tauri-build = "2.6.3"

[dependencies]
photo-app-service = { path = "../../../crates/app-service" }
photo-domain = { path = "../../../crates/domain" }
serde = { version = "1.0.229", features = ["derive"] }
tauri = "2.11.5"
tauri-plugin-dialog = "2.7.2"
thiserror = "2.0.20"
tracing = "0.1.44"
tracing-subscriber = "0.3.23"
```

`build.rs` calls `tauri_build::build()`. `main.rs` calls `photo_viewer_desktop_lib::run()`.

- [ ] **Step 6: Implement named local profiles and managed state**

```rust
// apps/desktop/src-tauri/src/profile.rs
pub struct ProfileRoots {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl ProfileRoots {
    pub fn from_bases(data: &Path, cache: &Path, profile: Option<&str>) -> Result<Self, ProfileError> {
        let Some(profile) = profile else {
            return Ok(Self { data_dir: data.to_owned(), cache_dir: cache.to_owned() });
        };
        if profile.is_empty()
            || profile.len() > 48
            || !profile.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ProfileError::InvalidName);
        }
        Ok(Self {
            data_dir: data.join("profiles").join(profile),
            cache_dir: cache.join("profiles").join(profile),
        })
    }
}
```

Resolve the base paths with `app.path().app_data_dir()` and `app.path().app_cache_dir()`. Read `PHOTO_VIEWER_PROFILE` as Unicode; invalid Unicode returns startup failure. Store `AppService` in `DesktopState { service: Mutex<AppService> }`.

- [ ] **Step 7: Implement bounded command DTOs and native folder selection**

```rust
// apps/desktop/src-tauri/src/dto.rs
#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ChooseFolderResult {
    Cancelled,
    Selected { state: BootstrapState },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: &'static str,
    pub message: &'static str,
}
```

Map errors to fixed codes and messages such as `folderUnavailable`, `folderNotDirectory`, `folderOverlapsSource`, `folderOverlapsLocalState`, `localStateUnavailable`, and `internal`. Log the underlying Rust error without returning paths or SQLite details to React.

```rust
// apps/desktop/src-tauri/src/commands.rs
use photo_app_service::{AppServiceError, BootstrapState};
use photo_core::AddLibraryError;
use photo_domain::Appearance;
use tauri::{AppHandle, State, Theme, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::dto::{ChooseFolderResult, CommandError};
use crate::state::DesktopState;

#[tauri::command]
pub fn get_bootstrap_state(
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    state
        .service
        .lock()
        .map_err(|_| CommandError::internal())?
        .bootstrap()
        .map_err(map_service_error)
}

#[tauri::command]
pub async fn choose_folder(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<ChooseFolderResult, CommandError> {
    let selected = app.dialog().file().blocking_pick_folder();
    let Some(selected) = selected else {
        return Ok(ChooseFolderResult::Cancelled);
    };
    let path = selected.into_path().map_err(|error| {
        tracing::error!(%error, "native dialog returned an unusable path");
        CommandError::internal()
    })?;
    let state = state
        .service
        .lock()
        .map_err(|_| CommandError::internal())?
        .open_recent(&path)
        .map_err(map_service_error)?;
    Ok(ChooseFolderResult::Selected { state })
}

#[tauri::command]
pub fn update_appearance(
    appearance: Appearance,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<BootstrapState, CommandError> {
    let bootstrap = state
        .service
        .lock()
        .map_err(|_| CommandError::internal())?
        .update_appearance(appearance)
        .map_err(map_service_error)?;
    let theme = match appearance {
        Appearance::System => None,
        Appearance::Light => Some(Theme::Light),
        Appearance::Dark => Some(Theme::Dark),
    };
    window.set_theme(theme).map_err(|error| {
        tracing::error!(%error, "could not apply the native window theme");
        CommandError::internal()
    })?;
    Ok(bootstrap)
}

fn map_service_error(error: AppServiceError) -> CommandError {
    tracing::error!(%error, "desktop command failed");
    match error {
        AppServiceError::OpenRecent(AddLibraryError::Io(_)) => {
            CommandError::new("folderUnavailable", "The selected folder is unavailable.")
        }
        AppServiceError::OpenRecent(AddLibraryError::NotDirectory(_)) => {
            CommandError::new("folderNotDirectory", "Choose a folder, not a file.")
        }
        AppServiceError::OpenRecent(AddLibraryError::Overlaps { .. }) => CommandError::new(
            "folderOverlapsSource",
            "That folder overlaps an existing source.",
        ),
        AppServiceError::OpenRecent(AddLibraryError::OverlapsLocalState) => CommandError::new(
            "folderOverlapsLocalState",
            "That folder overlaps Photo Viewer's local data.",
        ),
        AppServiceError::LocalState(_) => CommandError::new(
            "localStateUnavailable",
            "Photo Viewer cannot open its local data.",
        ),
        _ => CommandError::internal(),
    }
}
```

Add `CommandError::new` and `CommandError::internal` constructors in `dto.rs`. Both return only the static code and message fields shown above. The service lock is acquired only after the dialog closes.

Add this private mapping test to `commands.rs` so user-visible error codes cannot drift:

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use photo_app_service::AppServiceError;
    use photo_core::AddLibraryError;

    use super::map_service_error;

    #[test]
    fn non_directory_selection_has_a_bounded_error() {
        let error = map_service_error(AppServiceError::OpenRecent(
            AddLibraryError::NotDirectory(PathBuf::from("/private/source-name")),
        ));
        assert_eq!(error.code, "folderNotDirectory");
        assert_eq!(error.message, "Choose a folder, not a file.");
        assert!(!error.message.contains("source-name"));
    }
}
```

- [ ] **Step 8: Configure and run the Tauri host**

Use this Tauri configuration. It keeps native decorations and an opaque background. Folder selection runs in Rust, so JavaScript receives no dialog or filesystem plugin permission.

`apps/desktop/src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Photo Viewer",
  "version": "0.1.0",
  "identifier": "app.photoviewer.desktop",
  "build": {
    "beforeDevCommand": {
      "script": "npm run dev --workspace @photo-viewer/interface",
      "cwd": "../..",
      "wait": false
    },
    "devUrl": "http://127.0.0.1:1420",
    "beforeBuildCommand": {
      "script": "npm run build --workspace @photo-viewer/interface",
      "cwd": "../.."
    },
    "frontendDist": "../../interface/dist"
  },
  "app": {
    "security": {
      "csp": "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'"
    },
    "windows": [
      {
        "label": "main",
        "title": "Photo Viewer",
        "width": 1180,
        "height": 780,
        "minWidth": 720,
        "minHeight": 520,
        "resizable": true,
        "decorations": true,
        "transparent": false
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": ["app"],
    "macOS": {
      "minimumSystemVersion": "11.0"
    }
  }
}
```

`apps/desktop/src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Core window capability for the shared interface",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

In `lib.rs`, initialize `tracing_subscriber` and `tauri_plugin_dialog`, create the profile and `AppService` inside `setup`, apply the persisted native theme to the main window, manage `DesktopState`, and register all three commands in one `generate_handler!` call.

`apps/desktop/package.json`:

```json
{
  "name": "@photo-viewer/desktop",
  "private": true,
  "version": "0.1.0",
  "scripts": {
    "dev": "tauri dev",
    "build": "tauri build"
  },
  "devDependencies": {
    "@tauri-apps/cli": "2.11.4"
  }
}
```

Add these exact entries to the root `scripts` object:

```json
"desktop:dev": "npm exec --workspace @photo-viewer/desktop -- tauri dev",
"desktop:build": "npm exec --workspace @photo-viewer/desktop -- tauri build"
```

The Tauri hook objects set their current working directory to the repository root explicitly.

- [ ] **Step 9: Run adapter, Rust host, and package checks**

Run: `npm install`

Expected: `package-lock.json` includes the desktop workspace and remains free of unpinned direct dependencies.

Run: `npm run typecheck`

Run: `npm test`

Run: `npm run test:browser`

Run: `npm run check`

Expected: PASS.

Run: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`

Expected: profile and command-mapping tests PASS.

Run: `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`

Expected: PASS.

- [ ] **Step 10: Commit the embedded host**

```bash
git add Cargo.toml package.json package-lock.json apps/interface apps/desktop
git commit -m "feat: embed photo service in macos app"
```

---

### Task 7: Verify, package, and demonstrate checkpoint 1

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`
- Create during verification only: `target/demo-artifacts/checkpoint-1/`

**Interfaces:**
- Consumes: The complete running checkpoint from Tasks 1 through 6.
- Produces: CI gates, operator documentation, an unsigned `.app`, and a captured Open and Return demonstration.

- [ ] **Step 1: Confirm the operator and CI contracts are absent**

Run: `rg -F 'PHOTO_VIEWER_PROFILE=clean-demo' README.md`

Expected: exit 1 because the clean-profile workflow is not documented yet.

Run: `rg -F 'test:browser' .github/workflows/ci.yml`

Expected: exit 1 because CI does not run the interface browser checks yet.

- [ ] **Step 2: Document the desktop workflow**

Add README sections for Node 24.18.0, `npm ci`, interface checks, WebKit installation, normal desktop development, a named clean profile, the default profile, and unsigned app creation. State clearly that a named profile isolates local SQLite and cache state but does not copy or modify selected photos.

Use these exact commands:

```bash
npm ci
npm exec playwright install webkit
npm run typecheck
npm test
npm run test:browser
PHOTO_VIEWER_PROFILE=clean-demo npm run desktop:dev
npm run desktop:build -- --bundles app
```

- [ ] **Step 3: Extend CI without adding Tauri dependencies to the Rust matrix**

Add an `interface` job on `ubuntu-latest` that checks out code, installs Node 24, runs `npm ci`, installs Playwright WebKit with system dependencies, then runs `npm run check`, `npm run typecheck`, `npm test`, `npm run test:browser`, and the interface production build.

Add a `macos-app` job on `macos-latest` that installs Rust 1.97.1 and Node 24, runs `npm ci`, runs the interface production build, tests and lints `apps/desktop/src-tauri/Cargo.toml`, and runs `npm run desktop:build -- --bundles app`. Leave the existing three-platform root Rust job unchanged.

- [ ] **Step 4: Run the complete fresh verification suite**

Run each command separately and stop on the first failure:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm run check
npm run typecheck
npm test
npm run test:browser
npm run --workspace @photo-viewer/interface build
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all --check
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm run desktop:build -- --bundles app
```

Expected: every command exits 0. The root suite includes at least the existing 73 tests plus the new tests. The bundle command creates `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`.

- [ ] **Step 5: Demonstrate first launch and cancellation**

Start with a unique named profile:

```bash
PHOTO_VIEWER_PROFILE=checkpoint-1-cancel npm run desktop:dev
```

In the running app, confirm System appearance follows macOS. Choose Folder, cancel the native picker, and confirm the empty state remains without an error banner. Capture the empty state and appearance menu into `target/demo-artifacts/checkpoint-1/`.

- [ ] **Step 6: Demonstrate selection, appearance persistence, and restart**

Start with another unique profile:

```bash
PHOTO_VIEWER_PROFILE=checkpoint-1-return npm run desktop:dev
```

Choose a controlled local folder through the native picker. Confirm only its display name reaches the interface. Switch to Dark, close the app, then run the same command again. Confirm the same source and Dark appearance restore without reopening the picker. Repeat with Light. Capture selected-source and restored-source screenshots.

Inspect the selected source before and after the demonstration with a read-only directory listing and file hashes. Expected: no source file names, sizes, modified times, or hashes change.

- [ ] **Step 7: Smoke the unsigned bundle with an isolated profile**

Locate the built application at `apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app`. Run its inner executable with `PHOTO_VIEWER_PROFILE=checkpoint-1-bundle` so the environment reaches the app process. Confirm the native picker opens, the interface uses the embedded Rust service, and reopening the executable restores the source.

- [ ] **Step 8: Commit CI and operator documentation**

```bash
git add .github/workflows/ci.yml README.md
git commit -m "test: verify macos open and return checkpoint"
```

- [ ] **Step 9: Request review and present the feature demonstration**

Use `superpowers:requesting-code-review` against the full checkpoint diff. Resolve Critical and Important findings through `superpowers:receiving-code-review`, rerun Step 4, and report the exact commit, verification counts, `.app` location, screenshots, and remaining checkpoint 2 limitations to the user.

Do not begin checkpoint 2 until checkpoint 1 has been demonstrated and accepted.
