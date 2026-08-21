# Catalog Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a tested Rust catalog service that registers local and mounted sources, applies per-library folder policies, indexes files progressively into local SQLite, normalizes initial metadata with provenance, manages derivative cache records, reconciles unavailable sources safely, and exposes a minimal health API.

**Architecture:** A Rust workspace separates stable domain types, SQLite persistence, library policy, metadata extraction, staged indexing, cache accounting, and the HTTP host. Every source path enters through a library or recent-source ID; source media stays read-only, while SQLite and cache files stay under local application data. This plan implements Slice 1 from the approved system design and produces a headless, independently testable service.

**Tech Stack:** Rust 1.97.1, edition 2024; rusqlite 0.40.2 with bundled SQLite; Tokio 1.53.1; Axum 0.8.9; Serde 1.0.229; Notify 8.2.0; quick-xml 0.41.0; kamadak-exif 0.6.1; imagesize 0.15.0; BLAKE3 1.8.7.

**Spec:** `docs/superpowers/specs/2026-08-21-photo-viewer-design.md`

## Global Constraints

- Support macOS, Windows, and Linux from the first commit; do not introduce platform-only path assumptions.
- Source roots are read-only. Tests must fail if catalog or cache code attempts to create, update, rename, or delete a source file.
- SQLite and derivative data always live on local writable storage, never inside a library root or SMB mount.
- Use one SQLite catalog per installation and stable library IDs so relinking does not break references.
- Use SQLite 3.51.3 or newer because earlier WAL builds contain the WAL-reset race documented in the design research.
- Preserve catalog entries when a root is offline; an unavailable root must never be interpreted as mass deletion.
- Folder policy is per library. First matching policy wins; unmatched structures use recursive inclusion.
- Explicit adjacent XMP rating wins over embedded metadata, which wins over a path-derived rating.
- Keywords are the normalized union of sidecar XMP, embedded XMP, and IPTC values with provenance retained.
- Catalog metadata survives until a user removes a library. Durable wall thumbnails and large derivatives are separate cache tiers.
- Large-derivative eviction is by physical folder group, ordered by oldest `last_viewed_at`; never evict one asset independently.
- No Tauri shell, React interface, photo-wall query API, smart collections, deep-zoom generation, or Docker image in this plan.
- All production code changes follow test-driven development and end with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --all-features`.

## File Map

- `crates/domain`: IDs, path keys, media and availability enums, file signatures, metadata provenance, folder-policy types, and service errors. It depends on no storage or host crate.
- `crates/catalog`: SQLite opening, migrations, repositories, transactions, scan generations, warnings, and derivative accounting.
- `crates/core`: library registration, recent-source behavior, overlap checks, promotion, relinking, and folder-policy evaluation.
- `crates/metadata`: representative colour and shape probes, embedded EXIF adapter, XMP sidecar adapter, and field-specific resolution.
- `crates/indexer`: staged discovery events, catalog writer, priority scheduling, filesystem hints, and authoritative reconciliation.
- `crates/cache`: derivative keys, atomic writes, group recency, and eviction planning.
- `crates/server`: application assembly and the minimal health endpoint.
- `crates/catalog-bench`: deterministic synthetic-catalog benchmark for the one-million-asset target.
- `.github/workflows/ci.yml`: macOS, Windows, and Linux checks for the foundation.

Each new crate manifest uses workspace versions and only these dependencies:

- `photo-catalog`: `photo-domain`, `chrono`, `rusqlite`, `serde`, `serde_json`, `thiserror`, `uuid`; dev dependency `tempfile`.
- `photo-core`: `photo-domain`, `photo-catalog`, `globset`, `serde`, `serde_json`, `thiserror`; dev dependency `tempfile`.
- `photo-metadata`: `photo-domain`, `chrono`, `image`, `imagesize`, `kamadak-exif`, `quick-xml`, `serde`, `thiserror`; dev dependency `tempfile`.
- `photo-indexer`: `photo-domain`, `photo-catalog`, `photo-core`, `photo-metadata`, `notify`, `tokio`, `tracing`, `walkdir`, `thiserror`; dev dependency `tempfile`.
- `photo-cache`: `photo-domain`, `photo-catalog`, `blake3`, `thiserror`, `uuid`; dev dependency `tempfile`.
- `photo-server`: every production crate above plus `axum`, `http-body-util`, `serde`, `serde_json`, `tokio`, `tower-http`, `tracing`, and `tracing-subscriber`; dev dependencies `tempfile` and `tower`.
- `catalog-bench`: `photo-domain`, `photo-catalog`, `photo-cache`, `serde`, `serde_json`, and `tempfile`.

---

### Task 1: Workspace and stable domain model

**Files:**
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml`
- Create: `crates/domain/Cargo.toml`
- Create: `crates/domain/src/lib.rs`
- Create: `crates/domain/src/ids.rs`
- Create: `crates/domain/src/path_key.rs`
- Create: `crates/domain/src/media.rs`
- Test: `crates/domain/tests/domain_contract.rs`

**Interfaces:**
- Consumes: No earlier task.
- Produces: `LibraryId`, `FolderGroupId`, `AssetId`, `DerivativeId`, `NativePathKey`, `RelativePathKey`, `LibraryKind`, `Availability`, `MediaKind`, `FileSignature`, and `AssetId::for_path(LibraryId, &RelativePathKey)`.

- [ ] **Step 1: Pin the toolchain and create the workspace manifest**

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.97.1"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

```toml
# Cargo.toml
[workspace]
members = ["crates/*"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.97.1"

[workspace.dependencies]
axum = "0.8.9"
blake3 = "1.8.7"
chrono = { version = "0.4.45", features = ["serde"] }
globset = "0.4.20"
http-body-util = "0.1.5"
image = { version = "0.25.10", default-features = false, features = ["jpeg", "png", "tiff", "webp"] }
imagesize = "0.15.0"
kamadak-exif = "0.6.1"
notify = "8.2.0"
quick-xml = { version = "0.41.0", features = ["serialize"] }
rusqlite = { version = "0.40.2", features = ["bundled-full"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
tempfile = "3.27.0"
thiserror = "2.0.20"
tokio = { version = "1.53.1", features = ["full", "test-util"] }
tower = { version = "0.5.3", features = ["util"] }
tower-http = { version = "0.7.0", features = ["catch-panic", "trace"] }
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "fmt"] }
uuid = { version = "1.24.1", features = ["serde", "v4", "v5"] }
walkdir = "2.5.0"
```

- [ ] **Step 2: Write the domain contract test before the types exist**

```rust
// crates/domain/tests/domain_contract.rs
use std::path::Path;
use photo_domain::{AssetId, LibraryId, MediaKind, RelativePathKey};

#[test]
fn asset_identity_is_stable_for_library_and_native_relative_path() {
    let library = LibraryId::from_uuid(uuid::Uuid::from_u128(1));
    let path = RelativePathKey::from_relative_path(Path::new("2026/Trip/a.raw")).unwrap();
    let first = AssetId::for_path(library, &path);
    let second = AssetId::for_path(library, &path);
    assert_eq!(first, second);
}

#[test]
fn relative_path_key_rejects_absolute_paths() {
    let absolute = if cfg!(windows) { r"C:\Photos\a.jpg" } else { "/Photos/a.jpg" };
    assert!(RelativePathKey::from_relative_path(Path::new(absolute)).is_err());
}

#[test]
fn classifies_supported_extensions_case_insensitively() {
    assert_eq!(MediaKind::from_path(Path::new("a.CR3")), Some(MediaKind::Raw));
    assert_eq!(MediaKind::from_path(Path::new("clip.MP4")), Some(MediaKind::Video));
    assert_eq!(MediaKind::from_path(Path::new("notes.txt")), None);
}
```

- [ ] **Step 3: Run the contract test and confirm the missing crate failure**

Run: `cargo test -p photo-domain --test domain_contract`

Expected: FAIL because `crates/domain/Cargo.toml` and `photo_domain` do not exist.

- [ ] **Step 4: Implement the focused domain files**

```toml
# crates/domain/Cargo.toml
[package]
name = "photo-domain"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
chrono.workspace = true
serde.workspace = true
thiserror.workspace = true
uuid.workspace = true
```

```rust
// crates/domain/src/ids.rs
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::RelativePathKey;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub struct $name(Uuid);
        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
            pub const fn from_uuid(value: Uuid) -> Self { Self(value) }
            pub const fn as_uuid(self) -> Uuid { self.0 }
        }
    };
}

id_type!(LibraryId);
id_type!(FolderGroupId);
id_type!(DerivativeId);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AssetId(Uuid);

impl AssetId {
    pub const fn from_uuid(value: Uuid) -> Self { Self(value) }
    pub fn for_path(library: LibraryId, path: &RelativePathKey) -> Self {
        Self(Uuid::new_v5(&library.as_uuid(), path.as_bytes()))
    }
    pub const fn as_uuid(self) -> Uuid { self.0 }
}
```

```rust
// crates/domain/src/path_key.rs
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NativePathKey(Vec<u8>);

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RelativePathKey(NativePathKey);

#[derive(Debug, Error)]
pub enum PathKeyError {
    #[error("path must be relative")]
    Absolute,
    #[error("stored path has invalid native encoding")]
    InvalidEncoding,
}

impl RelativePathKey {
    pub fn from_relative_path(path: &Path) -> Result<Self, PathKeyError> {
        if path.is_absolute() { return Err(PathKeyError::Absolute); }
        Ok(Self(NativePathKey::from_path(path)))
    }
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PathKeyError> {
        let native = NativePathKey::from_bytes(bytes)?;
        if native.to_path_buf()?.is_absolute() { return Err(PathKeyError::Absolute); }
        Ok(Self(native))
    }
    pub fn as_bytes(&self) -> &[u8] { self.0.as_bytes() }
    pub fn to_path_buf(&self) -> Result<PathBuf, PathKeyError> { self.0.to_path_buf() }
}

impl NativePathKey {
    pub fn from_path(path: &Path) -> Self { Self(encode_native(path)) }
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PathKeyError> {
        let value = Self(bytes);
        value.to_path_buf()?;
        Ok(value)
    }
    pub fn as_bytes(&self) -> &[u8] { &self.0 }
    pub fn to_path_buf(&self) -> Result<PathBuf, PathKeyError> { decode_native(&self.0) }
}

#[cfg(unix)]
fn encode_native(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut encoded = vec![b'U'];
    encoded.extend_from_slice(path.as_os_str().as_bytes());
    encoded
}

#[cfg(unix)]
fn decode_native(bytes: &[u8]) -> Result<PathBuf, PathKeyError> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let payload = bytes.strip_prefix(b"U").ok_or(PathKeyError::InvalidEncoding)?;
    Ok(PathBuf::from(OsStr::from_bytes(payload)))
}

#[cfg(windows)]
fn encode_native(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut encoded = vec![b'W'];
    for unit in path.as_os_str().encode_wide() {
        encoded.extend_from_slice(&unit.to_le_bytes());
    }
    encoded
}

#[cfg(windows)]
fn decode_native(bytes: &[u8]) -> Result<PathBuf, PathKeyError> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let payload = bytes.strip_prefix(b"W").ok_or(PathKeyError::InvalidEncoding)?;
    if payload.len() % 2 != 0 { return Err(PathKeyError::InvalidEncoding); }
    let wide = payload.chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}
```

```rust
// crates/domain/src/media.rs
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LibraryKind { Configured, Recent }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Availability { Available, RootOffline, Missing, Unreadable }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MediaKind { Jpeg, Png, Tiff, Heif, Webp, Avif, Raw, Video, Unknown }

impl MediaKind {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "heic" | "heif" => Some(Self::Heif),
            "webp" => Some(Self::Webp),
            "avif" => Some(Self::Avif),
            "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" | "pef" => Some(Self::Raw),
            "mp4" | "mov" | "m4v" | "avi" | "mkv" => Some(Self::Video),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileSignature {
    pub size_bytes: u64,
    pub modified_unix_ns: i128,
    pub sidecar_modified_unix_ns: Option<i128>,
}
```

Export these modules from `crates/domain/src/lib.rs`. Finish the Windows and Unix path codec exactly as described in the comment, then add round-trip tests for Unicode names and, on Unix, a non-UTF-8 filename.

- [ ] **Step 5: Run domain tests and workspace quality checks**

Run: `cargo test -p photo-domain && cargo fmt --all --check && cargo clippy -p photo-domain --all-targets -- -D warnings`

Expected: PASS.

- [ ] **Step 6: Commit the workspace and domain contract**

```bash
git add rust-toolchain.toml Cargo.toml crates/domain Cargo.lock
git commit -m "feat: establish catalog domain model"
```

---

### Task 2: SQLite catalog, migration, and repositories

**Files:**
- Create: `crates/catalog/Cargo.toml`
- Create: `crates/catalog/src/lib.rs`
- Create: `crates/catalog/src/connection.rs`
- Create: `crates/catalog/src/migrate.rs`
- Create: `crates/catalog/src/library_repo.rs`
- Create: `crates/catalog/src/asset_repo.rs`
- Create: `crates/catalog/migrations/0001_catalog.sql`
- Test: `crates/catalog/tests/catalog_round_trip.rs`

**Interfaces:**
- Consumes: Domain IDs, `NativePathKey`, `RelativePathKey`, `LibraryKind`, `Availability`, `MediaKind`, and `FileSignature` from Task 1.
- Produces: `Catalog::open`, `Catalog::open_in_memory`, `Catalog::sqlite_version`, `Catalog::add_library`, `Catalog::list_libraries`, `Catalog::upsert_asset`, `Catalog::find_asset`, `NewLibrary`, `NewAsset`, and `CatalogError`.

- [ ] **Step 1: Write failing persistence tests**

```rust
// crates/catalog/tests/catalog_round_trip.rs
#[test]
fn opens_with_safe_sqlite_and_round_trips_library_and_asset() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    assert!(catalog.sqlite_version().unwrap() >= SqliteVersion::new(3, 51, 3));

    let library = NewLibrary::configured("Pictures", Path::new("/mounted/Pictures"));
    let stored = catalog.add_library(&library).unwrap();
    let rel = RelativePathKey::from_relative_path(Path::new("2026/Trip/a.jpg")).unwrap();
    let asset = NewAsset::minimal(stored.id, rel, "2026/Trip/a.jpg", MediaKind::Jpeg, 42);
    catalog.upsert_asset(&asset).unwrap();

    assert_eq!(catalog.list_libraries().unwrap(), vec![stored.clone()]);
    assert_eq!(catalog.find_asset(asset.id).unwrap().unwrap().display_path, "2026/Trip/a.jpg");
}

#[test]
fn failed_migration_restores_the_pre_migration_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES ('safe'); PRAGMA user_version=0;").unwrap();
    drop(connection);

    assert!(migrate_with(&path, &["BROKEN SQL"]).is_err());
    let restored = rusqlite::Connection::open(&path).unwrap();
    let value: String = restored.query_row("SELECT value FROM sentinel", [], |row| row.get(0)).unwrap();
    assert_eq!(value, "safe");
}
```

Keep `failed_migration_restores_the_pre_migration_database` in `migrate.rs` under `#[cfg(test)]` so it exercises the private `migrate_with` helper without adding a test-only public API.

- [ ] **Step 2: Run the catalog test and verify it fails**

Run: `cargo test -p photo-catalog --test catalog_round_trip`

Expected: FAIL because `photo-catalog` does not exist.

- [ ] **Step 3: Add the initial migration**

```sql
-- crates/catalog/migrations/0001_catalog.sql
CREATE TABLE library_roots (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  kind TEXT NOT NULL CHECK(kind IN ('configured', 'recent')),
  display_name TEXT NOT NULL,
  canonical_root_key BLOB NOT NULL UNIQUE,
  display_path TEXT NOT NULL,
  availability TEXT NOT NULL,
  last_seen_at INTEGER
);

CREATE TABLE folder_groups (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  relative_path_key BLOB NOT NULL,
  display_path TEXT NOT NULL,
  last_viewed_at INTEGER,
  UNIQUE(library_id, relative_path_key)
);

CREATE TABLE library_policies (
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  policy_json TEXT NOT NULL,
  PRIMARY KEY(library_id, position)
);

CREATE TABLE assets (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  folder_group_id BLOB REFERENCES folder_groups(id) ON DELETE SET NULL,
  relative_path_key BLOB NOT NULL,
  display_path TEXT NOT NULL,
  media_kind TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  modified_unix_ns TEXT NOT NULL,
  sidecar_modified_unix_ns TEXT,
  width INTEGER,
  height INTEGER,
  orientation INTEGER,
  representative_rgb INTEGER,
  visible_by_default INTEGER NOT NULL DEFAULT 1,
  availability TEXT NOT NULL,
  captured_at_utc TEXT,
  rating INTEGER CHECK(rating BETWEEN 0 AND 5),
  last_seen_generation INTEGER NOT NULL DEFAULT 0,
  UNIQUE(library_id, relative_path_key)
);

CREATE TABLE metadata_provenance (
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  field_name TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_path_key BLOB,
  raw_value TEXT NOT NULL,
  chosen INTEGER NOT NULL,
  PRIMARY KEY(asset_id, field_name, source_kind, raw_value)
);

CREATE TABLE asset_keywords (
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  normalized TEXT NOT NULL,
  display_value TEXT NOT NULL,
  hierarchy TEXT,
  PRIMARY KEY(asset_id, normalized, hierarchy)
);

CREATE TABLE scan_generations (
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  completed_at INTEGER,
  source_was_online INTEGER NOT NULL,
  PRIMARY KEY(library_id, generation)
);

CREATE TABLE warnings (
  id INTEGER PRIMARY KEY,
  library_id BLOB NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  asset_id BLOB REFERENCES assets(id) ON DELETE CASCADE,
  code TEXT NOT NULL,
  message TEXT NOT NULL,
  occurred_at INTEGER NOT NULL
);

CREATE TABLE derivatives (
  id BLOB PRIMARY KEY CHECK(length(id) = 16),
  asset_id BLOB NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  folder_group_id BLOB NOT NULL REFERENCES folder_groups(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  cache_key TEXT NOT NULL UNIQUE,
  relative_cache_path TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  durable INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX assets_library_sort ON assets(library_id, display_path, id);
CREATE INDEX assets_group ON assets(folder_group_id, id);
CREATE INDEX derivatives_group ON derivatives(folder_group_id, durable, created_at);
PRAGMA user_version = 1;
```

- [ ] **Step 4: Implement safe connection setup and repository methods**

`Catalog::open(path)` must create the parent directory, open rusqlite with bundled SQLite, apply `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, and a 5-second busy timeout, then run migrations in a transaction. `open_in_memory` skips WAL but applies every other pragma. Before changing a nonempty file database, copy it through rusqlite's backup API to `<catalog>.backup-v<old-user-version>`. If a migration fails, close the failed connection, restore that backup atomically, and return `CatalogError::MigrationFailed`.

```rust
pub struct Catalog { connection: rusqlite::Connection }

pub struct NewLibrary {
    pub id: LibraryId,
    pub kind: LibraryKind,
    pub display_name: String,
    pub canonical_root_key: NativePathKey,
    pub display_path: String,
}

impl NewLibrary {
    pub fn configured(display_name: impl Into<String>, canonical_root: &Path) -> Self;
    pub fn recent(display_name: impl Into<String>, canonical_root: &Path) -> Self;
}

pub struct NewAsset {
    pub id: AssetId,
    pub library_id: LibraryId,
    pub relative_path: RelativePathKey,
    pub display_path: String,
    pub media_kind: MediaKind,
    pub signature: FileSignature,
}

impl NewAsset {
    pub fn minimal(library: LibraryId, path: RelativePathKey, display_path: impl Into<String>, kind: MediaKind, size_bytes: u64) -> Self;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SqliteVersion { pub major: u32, pub minor: u32, pub patch: u32 }

impl SqliteVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self { Self { major, minor, patch } }
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self, CatalogError>;
    pub fn open_in_memory() -> Result<Self, CatalogError>;
    pub fn sqlite_version(&self) -> Result<SqliteVersion, CatalogError>;
    pub fn add_library(&mut self, value: &NewLibrary) -> Result<LibraryRootRecord, CatalogError>;
    pub fn list_libraries(&self) -> Result<Vec<LibraryRootRecord>, CatalogError>;
    pub fn upsert_asset(&mut self, value: &NewAsset) -> Result<(), CatalogError>;
    pub fn find_asset(&self, id: AssetId) -> Result<Option<AssetRecord>, CatalogError>;
    pub fn list_assets_page(&self, library: LibraryId, after: Option<(String, AssetId)>, limit: u32) -> Result<Vec<AssetRecord>, CatalogError>;
}
```

Store all UUIDs as 16-byte blobs, nanosecond `i128` values as decimal text, and booleans as `0` or `1`. Refuse startup with `CatalogError::UnsafeSqliteVersion` when the version is older than 3.51.3.

- [ ] **Step 5: Run migration, rollback, and round-trip tests**

Run: `cargo test -p photo-catalog --test catalog_round_trip && cargo clippy -p photo-catalog --all-targets -- -D warnings`

Expected: PASS and the file-backed test reports `journal_mode=wal`.

- [ ] **Step 6: Commit the catalog schema**

```bash
git add crates/catalog Cargo.lock
git commit -m "feat: add local sqlite catalog"
```

---

### Task 3: Library registration, recent sources, and relinking

**Files:**
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/core/src/source_fs.rs`
- Create: `crates/core/src/library_service.rs`
- Modify: `crates/catalog/src/library_repo.rs`
- Test: `crates/core/tests/library_lifecycle.rs`

**Interfaces:**
- Consumes: Task 1 path and ID types; Task 2 library repository.
- Produces: `SourceFs`, `RealSourceFs`, `LibraryService`, `SourceSelection`, `AddLibraryError`, `RelinkError`, `LibraryService::add_configured`, `LibraryService::open_recent`, `LibraryService::promote_recent`, and `LibraryService::relink`.

- [ ] **Step 1: Write failing lifecycle tests using temporary directories**

```rust
#[test]
fn folder_inside_existing_library_becomes_a_selection_not_a_second_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Photos");
    std::fs::create_dir_all(root.join("2026/Trip")).unwrap();
    let mut service = service_for(temp.path());
    let library = service.add_configured(&root, "Photos").unwrap();

    let selected = service.open_recent(&root.join("2026/Trip")).unwrap();
    assert_eq!(selected.library_id, library.id);
    assert_eq!(selected.relative_folder.to_path_buf().unwrap(), PathBuf::from("2026/Trip"));
    assert!(!selected.created_recent_root);
}

#[test]
fn relink_preserves_library_id_after_sample_verification() {
    let temp = tempfile::tempdir().unwrap();
    let old_root = temp.path().join("old");
    let new_root = temp.path().join("new");
    for root in [&old_root, &new_root] {
        std::fs::create_dir_all(root.join("Processed")).unwrap();
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join("Processed/b.jpg"), b"b").unwrap();
    }
    let mut service = service_for(temp.path());
    let library = service.add_configured(&old_root, "Photos").unwrap();
    service.mark_offline(library.id).unwrap();
    let samples = [relative("a.jpg"), relative("Processed/b.jpg")];

    let relinked = service.relink(library.id, &new_root, &samples).unwrap();
    assert_eq!(relinked.id, library.id);
    assert_eq!(relinked.display_path, new_root.to_string_lossy());

    std::fs::remove_file(new_root.join("Processed/b.jpg")).unwrap();
    assert!(matches!(
        service.relink(library.id, &new_root, &samples),
        Err(RelinkError::VerificationFailed { .. })
    ));
}
```

- [ ] **Step 2: Run the lifecycle test and verify the missing service failure**

Run: `cargo test -p photo-core --test library_lifecycle`

Expected: FAIL because `photo-core` does not exist.

- [ ] **Step 3: Implement the filesystem boundary and service signatures**

```rust
pub trait SourceFs: Send + Sync {
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn is_dir(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;
}

pub struct SourceSelection {
    pub library_id: LibraryId,
    pub relative_folder: RelativePathKey,
    pub created_recent_root: bool,
}

impl<F: SourceFs> LibraryService<F> {
    pub fn add_configured(&mut self, root: &Path, name: &str) -> Result<LibraryRootRecord, AddLibraryError>;
    pub fn open_recent(&mut self, folder: &Path) -> Result<SourceSelection, AddLibraryError>;
    pub fn promote_recent(&mut self, id: LibraryId, name: &str) -> Result<LibraryRootRecord, CatalogError>;
    pub fn mark_offline(&mut self, id: LibraryId) -> Result<(), CatalogError>;
    pub fn relink(&mut self, id: LibraryId, replacement: &Path, samples: &[RelativePathKey]) -> Result<LibraryRootRecord, RelinkError>;
}
```

Canonicalize before comparison. Reject duplicate, parent, or child configured roots with `AddLibraryError::Overlaps { existing_id }`. `open_recent` reuses an existing configured or recent root when the selected folder is inside it. It creates a recent root only when no cataloged root contains the folder.

- [ ] **Step 4: Prove source roots remain read-only**

Add a `RecordingSourceFs` test double whose mutating methods do not exist. Search production core code for `File::create`, `write`, `rename`, `remove_file`, and `remove_dir`; the test module may use these only for fixture setup.

Run: `rg -n "File::create|\.write\(|rename\(|remove_file\(|remove_dir" crates/core/src`

Expected: no matches.

- [ ] **Step 5: Run lifecycle tests and commit**

Run: `cargo test -p photo-core --test library_lifecycle && cargo clippy -p photo-core --all-targets -- -D warnings`

```bash
git add crates/core crates/catalog Cargo.lock
git commit -m "feat: manage library source lifecycle"
```

---

### Task 4: Per-library folder policy engine

**Files:**
- Create: `crates/domain/src/folder_policy.rs`
- Create: `crates/core/src/folder_policy.rs`
- Create: `crates/catalog/src/policy_repo.rs`
- Modify: `crates/domain/src/lib.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/catalog/src/lib.rs`
- Test: `crates/core/tests/folder_policy.rs`

**Interfaces:**
- Consumes: Task 1 path types and Task 3 library boundary.
- Produces: `FolderPolicy`, `StructureMatcher`, `PolicyBehavior`, `PathRatingRule`, `FolderPolicyEngine::classify`, and `PolicyDecision`.

- [ ] **Step 1: Write the user's folder structure as failing table tests**

```rust
#[test]
fn processed_policy_flattens_star_folders_and_derives_fallback_rating() {
    let policy = processed_policy();
    let engine = FolderPolicyEngine::new(vec![policy]);
    let structure = snapshot(&["RAW", "Videos", "Processed"]);

    let processed = engine.classify(
        Path::new("2026/Walk/Processed/3 Stars/final.jpg"),
        &structure,
    ).unwrap();
    assert_eq!(processed.group_path, PathBuf::from("2026/Walk"));
    assert!(processed.visible_by_default);
    assert_eq!(processed.path_rating, Some(3));
    assert_eq!(processed.navigation_path, PathBuf::from("2026/Walk"));

    let raw = engine.classify(Path::new("2026/Walk/RAW/source.cr3"), &structure).unwrap();
    assert!(!raw.visible_by_default);
    assert_eq!(raw.path_rating, None);
}

#[test]
fn unmatched_structure_recursively_includes_media() {
    let decision = FolderPolicyEngine::new(vec![processed_policy()])
        .classify(Path::new("2026/Misc/sub/photo.jpg"), &snapshot(&["sub"]))
        .unwrap();
    assert!(decision.visible_by_default);
}
```

- [ ] **Step 2: Run the folder-policy tests and verify missing types**

Run: `cargo test -p photo-core --test folder_policy`

Expected: FAIL with unresolved `FolderPolicyEngine` and policy types.

- [ ] **Step 3: Implement guided policy types and first-match evaluation**

```rust
pub struct FolderPolicy {
    pub name: String,
    pub group_depth: usize,
    pub matcher: StructureMatcher,
    pub behavior: PolicyBehavior,
}

pub struct StructureMatcher { pub required_child_globs: Vec<String> }

pub struct PolicyBehavior {
    pub default_include_globs: Vec<String>,
    pub navigation_hide_globs: Vec<String>,
    pub path_rating: Option<PathRatingRule>,
}

pub struct PathRatingRule { pub segment_suffix: String }

pub struct PolicyDecision {
    pub group_path: PathBuf,
    pub navigation_path: PathBuf,
    pub visible_by_default: bool,
    pub path_rating: Option<u8>,
    pub matched_policy: Option<String>,
}
```

Compile glob patterns when constructing the engine. A path-rating rule for `3 Stars` strips the configured suffix ` Stars`, parses the remaining segment as `u8`, and accepts only `0..=5`. Return the recursive fallback when no policy matches the candidate group's child snapshot. Serialize policies as JSON in `library_policies`, preserving list order; add a save-and-load round-trip test.

- [ ] **Step 4: Add invalid suffix, invalid rating, and policy persistence tests**

Test `Stars`, `6 Stars`, and `3 Starred`; all produce `path_rating=None`. Save two ordered policies for one library, reopen the catalog, and assert the same order and JSON values are returned.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p photo-core --test folder_policy && cargo fmt --all --check`

```bash
git add crates/domain crates/core crates/catalog Cargo.lock
git commit -m "feat: apply per-library folder policies"
```

---

### Task 5: Metadata provenance and field resolution

**Files:**
- Create: `crates/metadata/Cargo.toml`
- Create: `crates/metadata/src/lib.rs`
- Create: `crates/metadata/src/model.rs`
- Create: `crates/metadata/src/resolve.rs`
- Test: `crates/metadata/tests/precedence.rs`

**Interfaces:**
- Consumes: Task 4 path-derived rating candidate.
- Produces: `MetadataSource`, `MetadataCandidate<T>`, `MetadataBundle`, `ResolvedMetadata`, `MetadataResolver::resolve`, `Keyword`, and `ProvenanceRecord`.

- [ ] **Step 1: Write failing precedence and keyword-union tests**

```rust
#[test]
fn sidecar_rating_beats_embedded_and_path_rating() {
    let bundle = MetadataBundle {
        ratings: vec![
            candidate(3, MetadataSource::PathPolicy),
            candidate(4, MetadataSource::EmbeddedXmp),
            candidate(2, MetadataSource::SidecarXmp),
        ],
        ..MetadataBundle::default()
    };
    let resolved = MetadataResolver::resolve(bundle);
    assert_eq!(resolved.rating, Some(2));
    assert_eq!(resolved.provenance.iter().filter(|p| p.field == "rating" && p.chosen).count(), 1);
}

#[test]
fn keywords_union_case_insensitively_and_preserve_hierarchy() {
    let bundle = bundle_with_keywords([
        ("Family", None, MetadataSource::EmbeddedIptc),
        ("family", None, MetadataSource::SidecarXmp),
        ("London", Some("Places|UK|London"), MetadataSource::SidecarXmp),
    ]);
    let resolved = MetadataResolver::resolve(bundle);
    assert_eq!(resolved.keywords.len(), 2);
    assert_eq!(resolved.keywords[1].hierarchy.as_deref(), Some("Places|UK|London"));
}

fn candidate(value: u8, source: MetadataSource) -> MetadataCandidate<u8> {
    MetadataCandidate { value, source, raw_value: value.to_string() }
}

fn bundle_with_keywords<const N: usize>(values: [(&str, Option<&str>, MetadataSource); N]) -> MetadataBundle {
    MetadataBundle {
        keywords: values.into_iter().map(|(value, hierarchy, source)| KeywordCandidate {
            value: value.to_owned(),
            hierarchy: hierarchy.map(str::to_owned),
            source,
        }).collect(),
        ..MetadataBundle::default()
    }
}
```

- [ ] **Step 2: Run the metadata tests and verify the missing crate failure**

Run: `cargo test -p photo-metadata --test precedence`

Expected: FAIL because `photo-metadata` does not exist.

- [ ] **Step 3: Implement typed candidates and field-specific precedence**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataSource {
    SidecarXmp,
    EmbeddedXmp,
    EmbeddedExif,
    EmbeddedIptc,
    Container,
    FilesystemBirth,
    FilesystemModified,
    PathPolicy,
}

pub struct MetadataCandidate<T> {
    pub value: T,
    pub source: MetadataSource,
    pub raw_value: String,
}

#[derive(Default)]
pub struct MetadataBundle {
    pub capture_dates: Vec<MetadataCandidate<chrono::DateTime<chrono::FixedOffset>>>,
    pub ratings: Vec<MetadataCandidate<u8>>,
    pub keywords: Vec<KeywordCandidate>,
    pub orientation: Option<u16>,
    pub warnings: Vec<MetadataWarning>,
}

pub struct KeywordCandidate {
    pub value: String,
    pub hierarchy: Option<String>,
    pub source: MetadataSource,
}

pub struct MetadataWarning {
    pub code: &'static str,
    pub message: String,
}

pub struct Keyword {
    pub normalized: String,
    pub display_value: String,
    pub hierarchy: Option<String>,
}

pub struct ProvenanceRecord {
    pub field: String,
    pub source: MetadataSource,
    pub raw_value: String,
    pub chosen: bool,
}

pub struct ResolvedMetadata {
    pub captured_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub rating: Option<u8>,
    pub keywords: Vec<Keyword>,
    pub provenance: Vec<ProvenanceRecord>,
}
```

Use explicit precedence arrays for capture date and rating. Normalize keyword identity with Unicode lowercase plus trimmed internal whitespace. Keep the first nonempty display spelling and every distinct hierarchy. Emit one provenance row per candidate and mark only selected scalar candidates as chosen; keyword contributors are chosen when they contribute a unique normalized value or hierarchy.

- [ ] **Step 4: Add invalid rating and date fallback tests**

Test that ratings outside `0..=5` become warnings rather than values. Test capture-date order as original capture, other embedded capture, container creation, filesystem birth, then filesystem modified.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p photo-metadata && cargo clippy -p photo-metadata --all-targets -- -D warnings`

```bash
git add crates/metadata Cargo.lock
git commit -m "feat: normalize metadata with provenance"
```

---

### Task 6: Shape probe, EXIF, and XMP sidecar adapters

**Files:**
- Create: `crates/metadata/src/probe.rs`
- Create: `crates/metadata/src/exif_reader.rs`
- Create: `crates/metadata/src/xmp_reader.rs`
- Modify: `crates/metadata/src/lib.rs`
- Test: `crates/metadata/tests/readers.rs`

**Interfaces:**
- Consumes: Task 5 `MetadataBundle` and candidates.
- Produces: `MediaProbe::shape`, `MediaProbe::representative_rgb`, `EmbeddedExifReader::read`, `XmpSidecarReader::read`, `ImageShape`, `RepresentativeRgb`, and `MetadataReadWarning`.

- [ ] **Step 1: Write failing adapter tests with in-test fixtures**

```rust
#[test]
fn reads_rating_and_hierarchical_keywords_from_xmp() {
    let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
      <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
        <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/"
          xmlns:dc="http://purl.org/dc/elements/1.1/"
          xmlns:lr="http://ns.adobe.com/lightroom/1.0/" xmp:Rating="4">
          <dc:subject><rdf:Bag><rdf:li>Family</rdf:li></rdf:Bag></dc:subject>
          <lr:hierarchicalSubject><rdf:Bag><rdf:li>Places|UK|London</rdf:li></rdf:Bag></lr:hierarchicalSubject>
        </rdf:Description>
      </rdf:RDF>
    </x:xmpmeta>"#;
    let bundle = XmpSidecarReader::read(xml.as_bytes()).unwrap();
    assert_eq!(bundle.ratings[0].value, 4);
    assert!(bundle.keywords.iter().any(|k| k.value == "Family"));
    assert!(bundle.keywords.iter().any(|k| k.hierarchy.as_deref() == Some("Places|UK|London")));
}

#[test]
fn reads_orientation_from_minimal_little_endian_tiff() {
    let tiff = [0x49,0x49,0x2a,0x00,0x08,0x00,0x00,0x00,0x01,0x00,
        0x12,0x01,0x03,0x00,0x01,0x00,0x00,0x00,0x06,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00];
    let bundle = EmbeddedExifReader::read_tiff(&tiff).unwrap();
    assert_eq!(bundle.orientation, Some(6));
}

#[test]
fn representative_colour_averages_a_small_rgb_image() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("two-pixels.png");
    let image = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap();
    image.save(&path).unwrap();
    let colour = MediaProbe::representative_rgb(&path).unwrap();
    assert_eq!(colour, RepresentativeRgb { red: 128, green: 0, blue: 128 });
}
```

- [ ] **Step 2: Run adapter tests and confirm unresolved readers**

Run: `cargo test -p photo-metadata --test readers`

Expected: FAIL with unresolved `XmpSidecarReader` and `EmbeddedExifReader`.

- [ ] **Step 3: Implement bounded readers**

Use `quick_xml::Reader` in event mode. Accept `xmp:Rating`, `dc:subject/rdf:li`, and `lr:hierarchicalSubject/rdf:li`; ignore unknown elements and cap text values at 16 KiB each. Use `exif::Reader` for TIFF and supported embedded EXIF streams. Map orientation, DateTimeOriginal, DateTimeDigitized, DateTime, and available XMP-compatible rating tags into Task 5 candidates.

```rust
pub struct ImageShape { pub width: u32, pub height: u32 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepresentativeRgb { pub red: u8, pub green: u8, pub blue: u8 }

impl MediaProbe {
    pub fn shape(path: &Path) -> Result<ImageShape, MetadataReadWarning> {
        let size = imagesize::size(path).map_err(MetadataReadWarning::shape)?;
        Ok(ImageShape { width: size.width as u32, height: size.height as u32 })
    }

    pub fn representative_rgb(path: &Path) -> Result<RepresentativeRgb, MetadataReadWarning> {
        let image = image::ImageReader::open(path)
            .map_err(MetadataReadWarning::open)?
            .with_guessed_format()
            .map_err(MetadataReadWarning::open)?
            .decode()
            .map_err(MetadataReadWarning::decode)?
            .thumbnail(32, 32)
            .to_rgb8();
        let count = u64::from(image.width()) * u64::from(image.height());
        let sums = image.pixels().fold([0_u64; 3], |mut sum, pixel| {
            sum[0] += u64::from(pixel[0]);
            sum[1] += u64::from(pixel[1]);
            sum[2] += u64::from(pixel[2]);
            sum
        });
        Ok(RepresentativeRgb {
            red: ((sums[0] + count / 2) / count) as u8,
            green: ((sums[1] + count / 2) / count) as u8,
            blue: ((sums[2] + count / 2) / count) as u8,
        })
    }
}
```

Readers open sources read-only and enforce a 16 MiB sidecar limit. Representative-colour decoding supports JPEG, PNG, TIFF, and WebP in this slice; other formats return a typed warning and remain indexable. Return typed warnings for malformed or oversized metadata instead of panicking.

- [ ] **Step 4: Add malformed and oversized sidecar tests**

Test truncated XML, a rating of `9`, and a synthetic sidecar one byte over 16 MiB. Expected results are warnings, an absent rating, and no process panic.

- [ ] **Step 5: Run metadata checks and commit**

Run: `cargo test -p photo-metadata && cargo fmt --all --check && cargo clippy -p photo-metadata --all-targets -- -D warnings`

```bash
git add crates/metadata Cargo.lock
git commit -m "feat: read initial image metadata sources"
```

---

### Task 7: Progressive staged indexer and catalog writer

**Files:**
- Create: `crates/indexer/Cargo.toml`
- Create: `crates/indexer/src/lib.rs`
- Create: `crates/indexer/src/events.rs`
- Create: `crates/indexer/src/discover.rs`
- Create: `crates/indexer/src/scanner.rs`
- Create: `crates/indexer/src/catalog_writer.rs`
- Test: `crates/indexer/tests/progressive_scan.rs`

**Interfaces:**
- Consumes: Task 2 asset repository, Task 4 policy decisions, and Task 6 media readers.
- Produces: `Indexer::start`, `ScanRequest`, `ScanHandle`, `IndexEvent`, `MetadataReader`, `CatalogWriter::apply_batch`, and `ScanSummary`.

- [ ] **Step 1: Write a failing test that proves discovery is emitted before metadata completes**

```rust
#[tokio::test]
async fn emits_discovery_before_blocked_metadata_reader_finishes() {
    let fixture = source_with_files(["Processed/1 Star/a.jpg", "Processed/2 Stars/b.jpg"]);
    let (reader, release) = BlockingMetadataReader::new();
    let indexer = Indexer::new(reader, test_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();

    let first = scan.events.recv().await.unwrap();
    assert!(matches!(first, IndexEvent::Discovered { .. }));
    release.send(()).unwrap();
    let summary = scan.join().await.unwrap();
    assert_eq!(summary.discovered, 2);
    assert_eq!(summary.failed, 0);
}
```

- [ ] **Step 2: Run the progressive scan test and verify the missing crate failure**

Run: `cargo test -p photo-indexer --test progressive_scan`

Expected: FAIL because `photo-indexer` does not exist.

- [ ] **Step 3: Implement stage events and the async handle**

```rust
pub enum IndexEvent {
    Discovered { asset: NewAsset },
    Shaped {
        asset_id: AssetId,
        width: u32,
        height: u32,
        orientation: Option<u16>,
        representative_rgb: Option<RepresentativeRgb>,
    },
    MetadataReady { asset_id: AssetId, metadata: ResolvedMetadata },
    Warning { asset_id: Option<AssetId>, code: &'static str, message: String },
    Completed(ScanSummary),
}

pub struct ScanHandle {
    pub events: tokio::sync::mpsc::Receiver<IndexEvent>,
    cancel: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<Result<ScanSummary, IndexError>>,
}

impl ScanHandle {
    pub fn cancel(&self) -> Result<(), IndexError>;
    pub async fn join(self) -> Result<ScanSummary, IndexError>;
}
```

Walk directories in `spawn_blocking`, never follow symlinks by default, and send `Discovered` immediately after stat and policy classification. For each supported media path, probe case-insensitively for `<filename>.xmp` first and `<stem>.xmp` second; read the first match so scalar precedence remains unambiguous. Record its modified time in `FileSignature`. Run shape and metadata work after discovery. Bound the event channel so a slow catalog writer applies backpressure instead of growing memory without limit.

- [ ] **Step 4: Implement transactional batch writes**

`CatalogWriter::apply_batch` must commit up to 500 events or 50 ms of accumulated events in one transaction, whichever occurs first. Upsert by deterministic asset ID. Pack representative RGB as `0xRRGGBB`. Store shape, resolved scalar fields, keywords, provenance, and warnings without deleting earlier catalog rows until reconciliation completes.

- [ ] **Step 5: Add corrupt-file continuation and cancellation tests**

Create one valid shape fixture and one invalid file. Assert the invalid file emits `Warning`, the valid file reaches `MetadataReady`, and the summary reports one failure without aborting. Start a 1,000-file scan, cancel after the first event, and assert a bounded partial summary with no half-written transaction.

- [ ] **Step 6: Run indexer tests and commit**

Run: `cargo test -p photo-indexer && cargo clippy -p photo-indexer --all-targets -- -D warnings`

```bash
git add crates/indexer crates/catalog Cargo.lock
git commit -m "feat: index media in progressive stages"
```

---

### Task 8: User-first scheduling

**Files:**
- Create: `crates/indexer/src/scheduler.rs`
- Modify: `crates/indexer/src/lib.rs`
- Test: `crates/indexer/tests/scheduler_priority.rs`

**Interfaces:**
- Consumes: Task 7 scan jobs.
- Produces: `IndexScheduler`, `IndexJob`, `JobPriority`, `InteractionMode`, `SchedulerConfig`, and `IndexScheduler::set_interaction_mode`.

- [ ] **Step 1: Write failing deterministic priority tests with paused Tokio time**

```rust
#[tokio::test(start_paused = true)]
async fn visible_work_preempts_queued_idle_work() {
    let scheduler = IndexScheduler::new(SchedulerConfig { idle_workers: 4, active_workers: 1 });
    scheduler.enqueue(job("idle-a", JobPriority::IdleLibrary)).await;
    scheduler.enqueue(job("idle-b", JobPriority::IdleLibrary)).await;
    scheduler.enqueue(job("visible", JobPriority::Visible)).await;
    assert_eq!(scheduler.next().await.unwrap().name(), "visible");
}

#[tokio::test(start_paused = true)]
async fn interaction_reduces_new_background_concurrency_to_one() {
    let scheduler = IndexScheduler::new(SchedulerConfig { idle_workers: 4, active_workers: 1 });
    scheduler.set_interaction_mode(InteractionMode::Active).await;
    assert_eq!(scheduler.available_background_permits(), 1);
}
```

- [ ] **Step 2: Run scheduler tests and verify they fail**

Run: `cargo test -p photo-indexer --test scheduler_priority`

Expected: FAIL with unresolved scheduler types.

- [ ] **Step 3: Implement stable priority ordering and cooperative cancellation**

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum JobPriority { IdleLibrary = 0, OpenCollection = 1, NearViewport = 2, Visible = 3 }
```

Use a binary heap keyed by priority then inverse sequence number so equal-priority jobs remain FIFO. Do not kill a decoder in the middle of an atomic output write. Mark queued low-value jobs cancelled immediately; running jobs observe cancellation between safe stages.

- [ ] **Step 4: Add starvation and reprioritization tests**

Assert an idle job eventually runs under a continuous but finite visible queue. Assert an asset already queued for idle indexing is promoted rather than duplicated when it becomes visible.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p photo-indexer --test scheduler_priority`

```bash
git add crates/indexer
git commit -m "feat: prioritize indexing around interaction"
```

---

### Task 9: Reconciliation, watcher hints, and unavailable roots

**Files:**
- Create: `crates/indexer/src/reconcile.rs`
- Create: `crates/indexer/src/watch.rs`
- Modify: `crates/catalog/src/asset_repo.rs`
- Modify: `crates/indexer/src/lib.rs`
- Test: `crates/indexer/tests/reconciliation.rs`

**Interfaces:**
- Consumes: Task 2 scan-generation table, Task 3 source availability, Task 7 catalog writer, and Task 8 scheduler.
- Produces: `Reconciler::run`, `ReconcileOutcome`, `ChangeHint`, `WatchService`, `Catalog::begin_generation`, and `Catalog::complete_generation`.

- [ ] **Step 1: Write the offline-root safety test**

```rust
#[tokio::test]
async fn offline_root_marks_assets_unavailable_without_deleting_them() {
    let mut harness = indexed_library(["a.jpg", "b.jpg"]);
    harness.source.set_offline();
    let outcome = harness.reconciler.run(harness.library_id).await.unwrap();
    assert_eq!(outcome, ReconcileOutcome::RootOffline { retained_assets: 2 });
    assert_eq!(harness.catalog.asset_count(harness.library_id).unwrap(), 2);
    assert!(harness.catalog.assets(harness.library_id).unwrap()
        .iter().all(|a| a.availability == Availability::RootOffline));
}
```

- [ ] **Step 2: Run reconciliation tests and verify missing behavior**

Run: `cargo test -p photo-indexer --test reconciliation`

Expected: FAIL with unresolved `Reconciler`.

- [ ] **Step 3: Implement authoritative scan generations**

Begin a generation only after the root directory can be opened. Stamp every observed asset with the generation inside catalog transactions. On successful completion, mark unseen assets `Missing`; do not delete them. If opening or walking the root fails at root scope, mark the root and its assets `RootOffline`, leave the previous generation authoritative, and return `ReconcileOutcome::RootOffline`.

- [ ] **Step 4: Implement Notify as a hint adapter**

Map notify events into `ChangeHint::{PathChanged, RescanRoot}`. Coalesce repeated hints for the same path over 250 ms and enqueue targeted reconciliation. Any watcher error becomes `RescanRoot`; it never changes catalog availability by itself.

- [ ] **Step 5: Add online missing-file and missed-event tests**

Delete one fixture only after confirming the root remains readable. Assert reconciliation marks that asset `Missing` and leaves its cached rows. Simulate no watcher event, run periodic reconciliation, and assert a new file becomes cataloged.

- [ ] **Step 6: Run tests and commit**

Run: `cargo test -p photo-indexer --test reconciliation && cargo clippy -p photo-indexer --all-targets -- -D warnings`

```bash
git add crates/catalog crates/indexer
git commit -m "feat: reconcile source availability safely"
```

---

### Task 10: Derivative cache primitives and group eviction

**Files:**
- Create: `crates/cache/Cargo.toml`
- Create: `crates/cache/src/lib.rs`
- Create: `crates/cache/src/key.rs`
- Create: `crates/cache/src/writer.rs`
- Create: `crates/cache/src/eviction.rs`
- Modify: `crates/catalog/src/lib.rs`
- Test: `crates/cache/tests/cache_policy.rs`

**Interfaces:**
- Consumes: Task 1 IDs and file signatures; Task 2 derivative and folder-group tables.
- Produces: `DerivativeKind`, `DerivativeSpec`, `DerivativeKey::compute`, `CacheWriter::write_atomic`, `EvictionPlanner::plan`, `EvictionPlan`, and `ProtectedGroups`.

- [ ] **Step 1: Write failing deterministic-key and group-eviction tests**

```rust
#[test]
fn derivative_key_changes_with_source_or_decoder_inputs() {
    let base = spec(DerivativeKind::ScreenPreview, signature(100, 7), "decoder-1", "srgb", DerivativeTarget::LongEdge(2048));
    assert_eq!(DerivativeKey::compute(&base), DerivativeKey::compute(&base));
    assert_ne!(DerivativeKey::compute(&base), DerivativeKey::compute(&spec(DerivativeKind::ScreenPreview, signature(101, 7), "decoder-1", "srgb", DerivativeTarget::LongEdge(2048))));
}

#[test]
fn eviction_removes_oldest_whole_group_and_keeps_durable_thumbnails() {
    let catalog = cache_catalog_with_groups([
        group("old", 10, [large(40), thumbnail(5)]),
        group("new", 20, [large(40), thumbnail(5)]),
    ]);
    let plan = EvictionPlanner::plan(&catalog, 35, &ProtectedGroups::default()).unwrap();
    assert_eq!(plan.groups, vec![group_id("old")]);
    assert_eq!(plan.reclaimable_bytes, 40);
}
```

- [ ] **Step 2: Run cache tests and verify the missing crate failure**

Run: `cargo test -p photo-cache --test cache_policy`

Expected: FAIL because `photo-cache` does not exist.

- [ ] **Step 3: Implement content-addressed derivative keys**

Hash the asset ID, file signature, orientation, derivative kind, decoder version, target dimensions or tile coordinates, and colour conversion with BLAKE3. Render the lowercase hex digest as the key and shard cache paths by the first four hex characters.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivativeKind {
    WallThumbnail,
    ScreenPreview,
    RawDecode,
    DeepZoomTile,
    PosterFrame,
    VideoProxy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivativeTarget {
    LongEdge(u32),
    Tile { level: u16, x: u32, y: u32, size: u16 },
    Original,
}

pub struct DerivativeSpec {
    pub asset_id: AssetId,
    pub signature: FileSignature,
    pub orientation: u16,
    pub kind: DerivativeKind,
    pub decoder_version: String,
    pub colour_space: String,
    pub target: DerivativeTarget,
}
```

- [ ] **Step 4: Implement atomic cache writes**

Write to `<final-name>.partial-<uuid>` in the final directory, flush and `sync_all`, then rename to the final path. Content-addressed outputs are immutable: if the final path already exists, delete the partial file and reuse the existing entry instead of replacing it. Create directories only below the configured cache root. On startup, remove only `.partial-` files below that root and repair catalog rows whose final file is absent.

- [ ] **Step 5: Implement folder-group eviction**

Query non-durable derivative bytes grouped by `folder_group_id`, sorted by nullable `last_viewed_at` oldest first. Skip protected groups and groups with active writes. Add whole groups until reclaimable bytes meet the request. Execute deletion by removing non-durable files first and deleting their catalog rows in one follow-up transaction. Never remove durable thumbnail rows.

- [ ] **Step 6: Add interrupted-write and path-escape tests**

Force the writer callback to fail before rename and assert no final file or catalog row exists. Supply `../outside` as a relative cache path and assert `CacheError::PathEscape` before any write.

- [ ] **Step 7: Run tests and commit**

Run: `cargo test -p photo-cache && cargo clippy -p photo-cache --all-targets -- -D warnings`

```bash
git add crates/cache crates/catalog Cargo.lock
git commit -m "feat: add derivative cache accounting"
```

---

### Task 11: Application assembly and health API

**Files:**
- Create: `crates/server/Cargo.toml`
- Create: `crates/server/src/lib.rs`
- Create: `crates/server/src/main.rs`
- Create: `crates/server/src/config.rs`
- Create: `crates/server/src/health.rs`
- Test: `crates/server/tests/health_api.rs`

**Interfaces:**
- Consumes: Task 2 catalog, Task 3 library service, Task 7 indexer, Task 9 availability, and Task 10 cache state.
- Produces: `AppState`, `HealthReport`, `build_router(AppState) -> axum::Router`, and environment configuration for `PHOTO_VIEWER_DATA_DIR`, `PHOTO_VIEWER_CACHE_DIR`, and `PHOTO_VIEWER_BIND`.

- [ ] **Step 1: Write the failing health endpoint test**

```rust
#[tokio::test]
async fn health_reports_database_cache_and_source_counts_without_paths() {
    let app = build_router(test_state().with_roots(2, 1));
    let response = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["status"], "degraded");
    assert_eq!(value["sources"]["available"], 2);
    assert_eq!(value["sources"]["unavailable"], 1);
    assert!(value.to_string().find("/Volumes").is_none());
}
```

- [ ] **Step 2: Run the server test and verify the missing crate failure**

Run: `cargo test -p photo-server --test health_api`

Expected: FAIL because `photo-server` does not exist.

- [ ] **Step 3: Implement configuration and application state**

Default bind address to `127.0.0.1:8080`. Require explicit data and cache directories, reject either directory when it is inside a configured source root, create them with owner-only permissions where the platform supports that, and open the catalog before binding the socket.

```rust
#[derive(serde::Serialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub database: ComponentHealth,
    pub cache: ComponentHealth,
    pub sources: SourceHealthCounts,
    pub active_warnings: u64,
}
```

Return `200` for healthy or degraded source availability, and `503` only when the database cannot open or the cache root is not writable. Do not return absolute source paths, credentials, or individual filenames.

- [ ] **Step 4: Add panic containment and tracing**

Wrap the router with `tower_http::catch_panic::CatchPanicLayer` and `TraceLayer`. Initialize `tracing_subscriber` from `RUST_LOG`, defaulting to `photo_server=info`. Log library IDs and warning codes, never unrestricted paths.

- [ ] **Step 5: Run API tests and a local smoke command**

Run: `cargo test -p photo-server && PHOTO_VIEWER_DATA_DIR=/tmp/photo-viewer-data PHOTO_VIEWER_CACHE_DIR=/tmp/photo-viewer-cache cargo run -p photo-server`

Expected: server binds only to `127.0.0.1:8080`; `curl -fsS http://127.0.0.1:8080/healthz` returns JSON with `status`, `database`, `cache`, `sources`, and `active_warnings`.

- [ ] **Step 6: Commit the headless service**

```bash
git add crates/server Cargo.lock
git commit -m "feat: expose catalog service health"
```

---

### Task 12: Million-asset benchmark and cross-platform CI

**Files:**
- Create: `crates/catalog-bench/Cargo.toml`
- Create: `crates/catalog-bench/src/main.rs`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`
- Test: `crates/catalog-bench/tests/benchmark_smoke.rs`

**Interfaces:**
- Consumes: All foundation crates.
- Produces: `catalog-bench --assets <count> --output <path>` JSON report and the documented foundation verification commands.

- [ ] **Step 1: Write a failing benchmark smoke test**

```rust
#[test]
fn ten_thousand_asset_smoke_report_has_all_measurements() {
    let report = run_benchmark(BenchmarkConfig { assets: 10_000, batch_size: 500 }).unwrap();
    assert_eq!(report.assets, 10_000);
    assert!(report.insert_ms > 0);
    assert!(report.first_page_ms >= 0.0);
    assert_eq!(report.first_page_rows, 100);
}
```

- [ ] **Step 2: Run the smoke test and verify the missing crate failure**

Run: `cargo test -p catalog-bench --test benchmark_smoke`

Expected: FAIL because `catalog-bench` does not exist.

- [ ] **Step 3: Implement deterministic catalog generation and reporting**

Generate paths as `YYYY/Collection-N/Processed/R Stars/IMG-NNNNNNN.jpg`, distribute media kinds and ratings deterministically, insert in 500-row transactions, then measure opening the catalog, inserting, loading the first 100 rows in natural path order, counting unavailable assets, and selecting the oldest cache groups. Write this schema:

```json
{
  "assets": 1000000,
  "sqlite_version": "3.51.3-or-newer",
  "database_bytes": 0,
  "insert_ms": 0.0,
  "first_page_ms": 0.0,
  "first_page_rows": 100,
  "unavailable_count_ms": 0.0,
  "eviction_plan_ms": 0.0
}
```

The JSON values above define keys and types, not expected measurements. The command exits nonzero on an incorrect row count, SQLite older than 3.51.3, or a query error. It records timings without enforcing hardware-dependent limits in CI.

- [ ] **Step 4: Add the three-platform CI workflow**

```yaml
name: ci
on: [push, pull_request]
jobs:
  rust:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.97.1
          components: rustfmt, clippy
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings
      - run: cargo test --workspace --all-features
      - run: cargo test -p catalog-bench --test benchmark_smoke
```

- [ ] **Step 5: Document setup and foundation commands**

`README.md` must state that the repository currently implements the headless catalog foundation, requires Rust 1.97.1 through rustup, never writes source media, and keeps SQLite/cache local. Include exact commands for the three workspace checks, starting the health server, and running:

```bash
cargo run -p catalog-bench --release -- --assets 1000000 --output target/catalog-benchmark.json
```

- [ ] **Step 6: Run the complete foundation verification**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-smoke.json
```

Expected: every command exits 0 and the smoke report contains 10,000 assets plus every documented measurement key.

- [ ] **Step 7: Commit the benchmark and CI**

```bash
git add crates/catalog-bench .github/workflows/ci.yml README.md Cargo.lock
git commit -m "test: verify catalog foundation across platforms"
```

---

## Final review gate

After Task 12, compare the implementation against this plan and the catalog-foundation portions of the approved spec. Confirm:

- Source directories have no production write path.
- SQLite reports 3.51.3 or newer and file-backed catalogs use WAL.
- An offline root retains its catalog rows.
- Recent sources reuse existing roots and can be promoted.
- The example `Processed/N Stars` policy produces the expected visibility and fallback rating.
- Discovery events reach the catalog before metadata completion.
- Visible jobs outrank queued idle work.
- Cache eviction removes complete physical folder groups and preserves durable thumbnails.
- Health output contains no source paths.
- The full workspace checks and 10,000-asset smoke benchmark pass on the development platform.

Run `git status --short` and require an empty result before handing off the slice for review.
