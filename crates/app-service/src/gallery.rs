use std::collections::{HashMap, VecDeque};
#[cfg(feature = "server-internal-prevalidated-source")]
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{Mutex as TokioMutex, Notify, OwnedSemaphorePermit, Semaphore};

use photo_cache::{
    CacheBudget, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
    ImageDerivativeGenerator,
};
#[cfg(feature = "server-internal-prevalidated-source")]
use photo_catalog::NewLibrary;
use photo_catalog::{Catalog, CatalogError, NewDerivative, NewFolderGroup, WallOrder};
#[cfg(feature = "server-internal-prevalidated-source")]
use photo_core::{AddLibraryError, PrevalidatedSourceKeys, normalize_prevalidated_source_key};
use photo_core::{LibraryService, RealSourceFs};
use photo_domain::{FolderGroupId, GalleryScope, LibraryId, RelativePathKey};
use photo_indexer::{DefaultMetadataReader, IndexEvent, Indexer, MetadataReader, ScanRequest};

use crate::derivative_coordinator::{
    HostedAttemptLease, HostedCancellationDecision, WorkKey, WorkLane, WorkResultReceiver,
    WorkTicket,
};
use crate::hosted_runtime::{
    ScanAdmission, ScanWorkerOwner, SelectionEventSubscription, SelectionRuntime,
    SubscriptionOrigin,
};
use crate::service::{ReaderAdapter, ServiceState};
use crate::{
    AppConfig, AppServiceError, DerivativeClass, DerivativePriority, DerivativeReference,
    DerivativeRequest, InteractionState, OrderState, SourceAvailability, WallPage,
    WallPreviewCounts, WallQueryRequest, WallUpdate,
};

fn canonicalize_for_identity(path: &Path) -> PathBuf {
    let mut unresolved = Vec::new();
    let mut existing = path.to_owned();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        unresolved.push(name.to_owned());
        if !existing.pop() {
            break;
        }
    }
    let mut canonical = std::fs::canonicalize(&existing).unwrap_or(existing);
    for component in unresolved.iter().rev() {
        canonical.push(component);
    }
    canonical
}

const MAX_MANAGED_DERIVATIVE_BYTES: u64 = 64 * 1024 * 1024;
const HOSTED_ASSET_DERIVATIVE_WARNING: &str = "derivative_generation_failed";
const HOSTED_DERIVATIVE_WORKERS: usize = 4;
const HOSTED_VISIBLE_BURST_WORKERS: usize = HOSTED_DERIVATIVE_WORKERS + 1;

fn derivative_spec_for_gallery(
    asset: &photo_catalog::AssetRecord,
    class: DerivativeClass,
) -> DerivativeSpec {
    let (kind, edge) = match class {
        DerivativeClass::WallThumbnail => (DerivativeKind::WallThumbnail, 1024),
        DerivativeClass::ScreenPreview => (DerivativeKind::ScreenPreview, 4096),
    };
    DerivativeSpec {
        asset_id: asset.id,
        signature: asset.signature,
        media_kind: asset.media_kind,
        orientation: asset.orientation.unwrap_or(1),
        kind,
        decoder_version: photo_codec::decoder_fingerprint(asset.media_kind)
            .unwrap_or_default()
            .to_owned(),
        colour_space: "srgb".to_owned(),
        target: DerivativeTarget::LongEdge(edge),
    }
}

fn derivative_kind_name_for_gallery(class: DerivativeClass) -> &'static str {
    match class {
        DerivativeClass::WallThumbnail => "wall_thumbnail",
        DerivativeClass::ScreenPreview => "screen_preview",
    }
}

fn derivative_record_is_valid(root: &Path, record: &photo_catalog::DerivativeRecord) -> bool {
    let Ok(writer) = CacheWriter::new(root) else {
        return false;
    };
    let Ok(mut file) = writer.open_checked(&record.relative_cache_path) else {
        return false;
    };
    validate_managed_jpeg(&mut file, record.size_bytes).unwrap_or(false)
}

/// Validates a bounded JPEG file. The catalog supplies the expected length,
/// while the cache imposes a hard ceiling so a sparse or corrupt file cannot
/// force an unbounded allocation. Full image decoding runs only from callers
/// that have moved this check to controlled blocking work when needed.
fn validate_managed_jpeg(file: &mut std::fs::File, expected_size: u64) -> std::io::Result<bool> {
    use std::io::{Cursor, Read, Seek, SeekFrom};

    let metadata = file.metadata()?;
    if metadata.len() != expected_size
        || !(16..=MAX_MANAGED_DERIVATIVE_BYTES).contains(&expected_size)
    {
        return Ok(false);
    }
    let expected_size = usize::try_from(expected_size).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "derivative is too large")
    })?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(expected_size);
    file.take(expected_size as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() != expected_size || !jpeg_header_is_well_formed(&bytes) {
        return Ok(false);
    }
    // Marker framing does not validate the entropy stream. Decode the bounded
    // bytes with image's allocation limit so same-size, well-framed garbage
    // cannot be treated as an immutable cache hit.
    let mut reader = image::ImageReader::new(Cursor::new(&bytes));
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(65_536);
    limits.max_image_height = Some(65_536);
    limits.max_alloc = Some(MAX_MANAGED_DERIVATIVE_BYTES);
    reader.limits(limits);
    let reader = reader.with_guessed_format()?;
    if reader.format() != Some(image::ImageFormat::Jpeg) || reader.decode().is_err() {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(bytes.ends_with(&[0xff, 0xd9]))
}

fn jpeg_header_is_well_formed(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[..2] != [0xff, 0xd8] {
        return false;
    }
    let mut offset = 2;
    let mut frame = false;
    while offset + 1 < bytes.len() {
        if bytes[offset] != 0xff {
            return false;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let Some(&marker) = bytes.get(offset) else {
            return false;
        };
        offset += 1;
        if marker == 0xda {
            let Some(length_bytes) = bytes.get(offset..offset + 2) else {
                return false;
            };
            let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
            if length < 2 || offset + length > bytes.len() {
                return false;
            }
            return frame;
        }
        if marker == 0xd9 {
            return frame;
        }
        if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let Some(length_bytes) = bytes.get(offset..offset + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 || offset + length > bytes.len() {
            return false;
        }
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            frame = true;
        }
        offset += length;
    }
    false
}

struct ProtectedGroupGuard {
    groups: photo_cache::ProtectedGroups,
    group: FolderGroupId,
}

struct HostedDriverOwner {
    coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    generation: u64,
    engine: GalleryEngine,
    runtime: Arc<SelectionRuntime>,
    active_ticket: Arc<Mutex<Option<WorkTicket>>>,
    active_attempt: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    active_attempt_lease: Arc<Mutex<Option<HostedAttemptLease>>>,
    kind: HostedDriverKind,
}

#[derive(Clone, Copy)]
enum HostedDriverKind {
    Durable,
    VisibleBurst,
}

struct HostedAttemptTaskOwner {
    attempt: HostedAttemptLease,
}

impl Drop for HostedAttemptTaskOwner {
    fn drop(&mut self) {
        self.attempt.finish_task();
    }
}

#[cfg(debug_assertions)]
struct HostedEncodeTestGate {
    entered: Arc<Notify>,
    release: Arc<AtomicBool>,
}

#[cfg(debug_assertions)]
#[derive(Clone)]
struct HostedCancellationTransitionTestGate {
    entered: Arc<Notify>,
    release: Arc<AtomicBool>,
    applied: Arc<Notify>,
    completed: Arc<Notify>,
}

#[cfg(debug_assertions)]
struct HostedEncodeRegistrationTestGate {
    entered: Arc<Notify>,
    release: Arc<AtomicBool>,
    registered: Arc<Notify>,
    rejected: Arc<Notify>,
}

#[cfg(debug_assertions)]
type HostedAsyncTestGate = Arc<TokioMutex<Option<(Arc<Notify>, Arc<Notify>)>>>;

impl Drop for HostedDriverOwner {
    fn drop(&mut self) {
        let coordinator = self.coordinator.clone();
        let generation = self.generation;
        let engine = self.engine.clone();
        let runtime = self.runtime.clone();
        let self_kind = self.kind;
        let active_ticket = self.active_ticket.clone();
        let active_attempt = self.active_attempt.clone();
        let active_attempt_lease = self.active_attempt_lease.clone();
        let attempt = active_attempt
            .lock()
            .expect("hosted active attempt poisoned")
            .take();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            #[cfg(debug_assertions)]
            let cancellation_gate = engine
                .hosted_cancellation_transition_test_gate
                .lock()
                .expect("hosted cancellation transition test gate poisoned")
                .take();
            #[cfg(debug_assertions)]
            if let Some(gate) = cancellation_gate.as_ref() {
                gate.entered.notify_one();
                while !gate.release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
            let attempt_lease = active_attempt_lease
                .lock()
                .expect("hosted active attempt lease poisoned")
                .take();
            let cancellation = attempt_lease
                .as_ref()
                .map(HostedAttemptLease::cancel)
                .unwrap_or(HostedCancellationDecision::CancelAttempt);
            if cancellation == HostedCancellationDecision::CancelAttempt
                && let Some(attempt) = attempt.as_ref()
            {
                attempt.abort();
            }
            #[cfg(debug_assertions)]
            if let Some(gate) = cancellation_gate.as_ref() {
                gate.applied.notify_one();
            }
            handle.spawn(async move {
                let ticket = active_ticket
                    .lock()
                    .expect("hosted active ticket poisoned")
                    .take();
                if cancellation == HostedCancellationDecision::CancelAttempt
                    && let Some(ticket) = ticket
                {
                    coordinator.abort_hosted_attempt(ticket).await;
                }
                if let Some(attempt_lease) = attempt_lease {
                    attempt_lease.wait_finished().await;
                }
                if let Some(ticket) = ticket {
                    coordinator
                        .recover_hosted_publication_as_failure(ticket)
                        .await;
                }
                match self_kind {
                    HostedDriverKind::Durable => {
                        coordinator.release_driver_owner(generation).await;
                    }
                    HostedDriverKind::VisibleBurst => {
                        coordinator.release_visible_driver_owner(generation).await;
                    }
                }
                if coordinator.pending_job_count().await > 0 {
                    engine.start_hosted_derivative_driver(runtime.clone()).await;
                }
                if coordinator.has_queued_visible_wall_work().await {
                    engine.start_hosted_visible_driver(runtime).await;
                }
                #[cfg(debug_assertions)]
                if let Some(gate) = cancellation_gate {
                    gate.completed.notify_one();
                }
            });
        }
    }
}

struct HostedAdmission {
    permit: Option<OwnedSemaphorePermit>,
    active: Arc<AtomicUsize>,
    wake: Arc<Notify>,
}

impl Drop for HostedAdmission {
    fn drop(&mut self) {
        self.permit.take();
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.wake.notify_waiters();
    }
}

enum HostedDerivativePrepared {
    Wall(photo_cache::EncodedScreenPreview),
    Screen(photo_cache::EncodedScreenPreview),
}

#[derive(Clone, Copy)]
enum HostedDerivativeRequestMode {
    AwaitCompletion,
    AdmitOnly,
}

struct HostedDerivativeBatch<'a> {
    selection: &'a GallerySelection,
    scope: GalleryScope,
    priority: DerivativePriority,
    class: DerivativeClass,
    assets: &'a [photo_catalog::AssetRecord],
    mode: HostedDerivativeRequestMode,
}

impl ProtectedGroupGuard {
    fn new(
        groups: photo_cache::ProtectedGroups,
        group: FolderGroupId,
    ) -> Result<Self, AppServiceError> {
        groups.protect(group)?;
        Ok(Self { groups, group })
    }
}

impl Drop for ProtectedGroupGuard {
    fn drop(&mut self) {
        let _ = self.groups.unprotect(self.group);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GallerySelection {
    pub(crate) id: String,
    pub(crate) library_id: LibraryId,
    pub(crate) group_id: FolderGroupId,
    pub(crate) relative_folder: RelativePathKey,
    pub(crate) epoch: u64,
}

impl GallerySelection {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn library_id(&self) -> LibraryId {
        self.library_id
    }
    pub fn group_id(&self) -> FolderGroupId {
        self.group_id
    }
    pub fn relative_folder(&self) -> &RelativePathKey {
        &self.relative_folder
    }
    pub(crate) fn token(&self) -> crate::service::SelectionToken {
        crate::service::SelectionToken {
            library_id: self.library_id,
            group_id: self.group_id,
            epoch: self.epoch,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSummary {
    pub folder_id: String,
    pub path: String,
    pub id: String,
    pub source_id: String,
    pub display_name: String,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub availability: SourceAvailability,
}

pub struct ManagedDerivative {
    pub file: std::fs::File,
    pub content_type: &'static str,
    pub content_length: u64,
    pub etag: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBreadcrumb {
    pub name: String,
    pub path: String,
}

#[derive(Clone)]
pub struct GalleryEngine {
    pub(crate) state: Arc<Mutex<ServiceState>>,
    pub(crate) scheduler: Arc<photo_indexer::IndexScheduler>,
    pub(crate) folder_jobs: Arc<crate::folder_jobs::FolderJobRegistry>,
    pub(crate) metadata_reader: ReaderAdapter,
    hosted_library_id: Option<LibraryId>,
    folder_access: crate::FolderAccessCoordinator,
    pub(crate) shared_coordinator:
        Option<Arc<crate::derivative_coordinator::DerivativeCoordinator>>,
    desktop_coordinator_claimed: Arc<AtomicBool>,
    pub(crate) cache_root: PathBuf,
    cache_writer: CacheWriter,
    pub(crate) catalog_path: PathBuf,
    derivative_admission: Arc<Semaphore>,
    derivative_admission_wake: Arc<Notify>,
    hosted_active: Arc<AtomicUsize>,
    #[cfg(debug_assertions)]
    hosted_active_peak: Arc<AtomicUsize>,
    #[cfg(debug_assertions)]
    hosted_admission_limit_override: Arc<AtomicUsize>,
    hosted_admission_owner: Arc<TokioMutex<()>>,
    hosted_publication_fence: Arc<TokioMutex<()>>,
    protected_groups: photo_cache::ProtectedGroups,
    #[cfg(debug_assertions)]
    hosted_attempts: Arc<AtomicUsize>,
    #[cfg(test)]
    fail_next_start: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_source_check: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_batch: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_join: Arc<AtomicBool>,
    #[cfg(debug_assertions)]
    hosted_commit_publication_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_commit_post_publication_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_authorized_publication_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_driver_abort_handle: Arc<TokioMutex<Option<tokio::task::AbortHandle>>>,
    #[cfg(debug_assertions)]
    hosted_cancellation_transition_test_gate:
        Arc<Mutex<Option<HostedCancellationTransitionTestGate>>>,
    #[cfg(debug_assertions)]
    hosted_panic_before_admission: Arc<TokioMutex<bool>>,
    #[cfg(debug_assertions)]
    hosted_post_authorization_fault: Arc<AtomicUsize>,
    #[cfg(debug_assertions)]
    hosted_encode_test_gates: Arc<TokioMutex<VecDeque<HostedEncodeTestGate>>>,
    #[cfg(debug_assertions)]
    hosted_encode_registration_test_gate: Arc<TokioMutex<Option<HostedEncodeRegistrationTestGate>>>,
    #[cfg(debug_assertions)]
    hosted_empty_authorization_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_post_validation_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_pre_enqueue_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_scan_admission_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_root_outage_snapshot_test_gate: HostedAsyncTestGate,
    #[cfg(debug_assertions)]
    hosted_link_removal_failure_test_hook: Arc<AtomicBool>,
}

/// Server-internal capability proving that the hosted source was pinned and
/// identity-validated at the startup boundary. The owned directory descriptor
/// prevents this from becoming a path-only trust bypass.
#[cfg(feature = "server-internal-prevalidated-source")]
#[doc(hidden)]
pub struct PrevalidatedHostedSource {
    validated_directory: File,
    validation_lifetime: Arc<()>,
    operational_path: PathBuf,
    canonical_key: PathBuf,
}

#[cfg(feature = "server-internal-prevalidated-source")]
impl PrevalidatedHostedSource {
    /// Constructs a hosted-source capability at the server's pinned startup
    /// validation boundary.
    ///
    /// # Safety
    ///
    /// The caller must ensure `validated_directory`, `operational_path`, and
    /// `canonical_key` describe the same directory and were validated together
    /// without a pathname reopen window. Runtime checks below establish that
    /// the descriptor is a directory and both keys are normalized absolute
    /// paths, but cannot prove that three-way identity relationship.
    #[doc(hidden)]
    pub unsafe fn from_server_validated_directory(
        validated_directory: File,
        validation_lifetime: Arc<()>,
        operational_path: PathBuf,
        canonical_key: PathBuf,
    ) -> std::io::Result<Self> {
        if !validated_directory.metadata()?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "prevalidated source descriptor is not a directory",
            ));
        }
        for key in [&operational_path, &canonical_key] {
            if !key.is_absolute() || normalize_prevalidated_source_key(key)? != *key {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "prevalidated source key is invalid",
                ));
            }
        }
        Ok(Self {
            validated_directory,
            validation_lifetime,
            operational_path,
            canonical_key,
        })
    }
}

impl GalleryEngine {
    pub fn open(config: AppConfig, source_root: PathBuf) -> Result<Self, AppServiceError> {
        Self::open_with_reader(config, source_root, Arc::new(DefaultMetadataReader))
    }

    pub fn open_with_reader(
        config: AppConfig,
        source_root: PathBuf,
        reader: Arc<dyn MetadataReader>,
    ) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let mut libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;
        let current_canonical = std::fs::canonicalize(&source_root).ok();
        let canonical = current_canonical
            .clone()
            .unwrap_or_else(|| canonicalize_for_identity(&source_root));
        let hosted_library_id = if let Some(library) = libraries
            .catalog()
            .list_libraries()?
            .into_iter()
            .find(|library| {
                library.canonical_root_key.to_path_buf().ok().as_deref()
                    == Some(canonical.as_path())
                    || current_canonical.is_none()
                        && Path::new(&library.display_path) == source_root.as_path()
            }) {
            library.id
        } else {
            if !canonical.is_dir() {
                return Err(AppServiceError::OpenRecent(
                    photo_core::AddLibraryError::NotDirectory(canonical),
                ));
            }
            libraries
                .add_configured(
                    &canonical,
                    source_root
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .as_deref()
                        .unwrap_or("Hosted photos"),
                )?
                .id
        };
        Self::finish_open(config, reader, libraries, hosted_library_id)
    }

    #[cfg(feature = "server-internal-prevalidated-source")]
    #[doc(hidden)]
    pub fn open_prevalidated_hosted(
        config: AppConfig,
        source: PrevalidatedHostedSource,
    ) -> Result<Self, AppServiceError> {
        let PrevalidatedHostedSource {
            validated_directory,
            validation_lifetime,
            operational_path,
            canonical_key,
        } = source;
        let mut cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        cataloged_roots.push(canonical_key.clone());
        // SAFETY: the configured key is tied to the owned server-validated
        // descriptor. Remaining keys came from the catalog's typed canonical
        // root field and are used only for no-I/O lexical overlap checks.
        let source_keys =
            unsafe { PrevalidatedSourceKeys::from_validated_identity_keys(cataloged_roots) }
                .map_err(AppServiceError::LibrarySetup)?;
        config.validate_prevalidated_source_keys(&source_keys)?;
        config.prepare_prevalidated_source_keys(&source_keys)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let mut libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;

        let mut hosted_library_id = None;
        for existing in libraries.catalog().list_libraries()? {
            let existing_key = existing
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| AddLibraryError::InvalidCatalogPath(error.to_string()))?;
            let existing_key =
                normalize_prevalidated_source_key(&existing_key).map_err(AddLibraryError::Io)?;
            if existing_key == canonical_key {
                hosted_library_id = Some(existing.id);
                break;
            }
            if existing_key.starts_with(&canonical_key) || canonical_key.starts_with(&existing_key)
            {
                return Err(AppServiceError::OpenRecent(AddLibraryError::Overlaps {
                    existing_id: existing.id,
                }));
            }
        }
        let hosted_library_id = match hosted_library_id {
            Some(id) => id,
            None => {
                let display_name = operational_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Hosted photos".to_owned());
                let mut library = NewLibrary::configured(display_name, &canonical_key);
                library.display_path = operational_path.to_string_lossy().into_owned();
                libraries.catalog_mut().add_library(&library)?.id
            }
        };
        let result = Self::finish_open(
            config,
            Arc::new(DefaultMetadataReader),
            libraries,
            hosted_library_id,
        );
        drop(validated_directory);
        drop(validation_lifetime);
        result
    }

    fn finish_open(
        config: AppConfig,
        reader: Arc<dyn MetadataReader>,
        libraries: LibraryService<RealSourceFs>,
        hosted_library_id: LibraryId,
    ) -> Result<Self, AppServiceError> {
        let cache_root = config.cache_dir().to_owned();
        let cache_writer = CacheWriter::new(&cache_root)?;
        let catalog_path = config.catalog_path();
        let scheduler = Arc::new(photo_indexer::IndexScheduler::new(Default::default()));
        Ok(Self {
            state: Arc::new(Mutex::new(ServiceState {
                libraries,
                folder_status: HashMap::new(),
                folder_revision: 0,
                active_scan: None,
                protected_group: None,
                selection_epoch: 0,
                published_wall_cache_warning: None,
                published_screen_cache_warning: None,
            })),
            folder_jobs: Arc::new(crate::folder_jobs::FolderJobRegistry::new(
                scheduler.clone(),
            )),
            scheduler,
            metadata_reader: ReaderAdapter::new(reader),
            hosted_library_id: Some(hosted_library_id),
            folder_access: crate::FolderAccessCoordinator::default(),
            shared_coordinator: None,
            desktop_coordinator_claimed: Arc::new(AtomicBool::new(false)),
            cache_root,
            cache_writer,
            catalog_path,
            derivative_admission: Arc::new(Semaphore::new(HOSTED_VISIBLE_BURST_WORKERS)),
            derivative_admission_wake: Arc::new(Notify::new()),
            hosted_active: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_active_peak: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_admission_limit_override: Arc::new(AtomicUsize::new(0)),
            hosted_admission_owner: Arc::new(TokioMutex::new(())),
            hosted_publication_fence: Arc::new(TokioMutex::new(())),
            protected_groups: photo_cache::ProtectedGroups::default(),
            #[cfg(debug_assertions)]
            hosted_attempts: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
            #[cfg(debug_assertions)]
            hosted_commit_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_commit_post_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_authorized_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_driver_abort_handle: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_cancellation_transition_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_panic_before_admission: Arc::new(TokioMutex::new(false)),
            #[cfg(debug_assertions)]
            hosted_post_authorization_fault: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_encode_test_gates: Arc::new(TokioMutex::new(VecDeque::new())),
            #[cfg(debug_assertions)]
            hosted_encode_registration_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_empty_authorization_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_post_validation_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_pre_enqueue_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_scan_admission_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_root_outage_snapshot_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_link_removal_failure_test_hook: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn from_shared_state(
        state: Arc<Mutex<ServiceState>>,
        scheduler: Arc<photo_indexer::IndexScheduler>,
        metadata_reader: ReaderAdapter,
        shared_coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
        cache_root: PathBuf,
        catalog_path: PathBuf,
    ) -> Self {
        let cache_writer = CacheWriter::new(&cache_root).expect("managed cache root exists");
        Self {
            state,
            folder_jobs: Arc::new(crate::folder_jobs::FolderJobRegistry::new(
                scheduler.clone(),
            )),
            scheduler,
            metadata_reader,
            hosted_library_id: None,
            folder_access: crate::FolderAccessCoordinator::default(),
            shared_coordinator: Some(shared_coordinator),
            desktop_coordinator_claimed: Arc::new(AtomicBool::new(false)),
            cache_root,
            cache_writer,
            catalog_path,
            derivative_admission: Arc::new(Semaphore::new(HOSTED_VISIBLE_BURST_WORKERS)),
            derivative_admission_wake: Arc::new(Notify::new()),
            hosted_active: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_active_peak: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_admission_limit_override: Arc::new(AtomicUsize::new(0)),
            hosted_admission_owner: Arc::new(TokioMutex::new(())),
            hosted_publication_fence: Arc::new(TokioMutex::new(())),
            protected_groups: photo_cache::ProtectedGroups::default(),
            #[cfg(debug_assertions)]
            hosted_attempts: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            fail_next_start: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_source_check: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_batch: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_join: Arc::new(AtomicBool::new(false)),
            #[cfg(debug_assertions)]
            hosted_commit_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_commit_post_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_authorized_publication_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_driver_abort_handle: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_cancellation_transition_test_gate: Arc::new(Mutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_panic_before_admission: Arc::new(TokioMutex::new(false)),
            #[cfg(debug_assertions)]
            hosted_post_authorization_fault: Arc::new(AtomicUsize::new(0)),
            #[cfg(debug_assertions)]
            hosted_encode_test_gates: Arc::new(TokioMutex::new(VecDeque::new())),
            #[cfg(debug_assertions)]
            hosted_encode_registration_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_empty_authorization_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_post_validation_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_pre_enqueue_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_scan_admission_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_root_outage_snapshot_test_gate: Arc::new(TokioMutex::new(None)),
            #[cfg(debug_assertions)]
            hosted_link_removal_failure_test_hook: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn for_photo_picks(&self, protected_groups: photo_cache::ProtectedGroups) -> Self {
        // Desktop wall work has one active coordinator. Picks need stable
        // per-group coordinators and runtimes so cross-folder requests cannot
        // reset or be rejected by the active wall's selection epoch.
        Self {
            folder_jobs: Arc::new(crate::folder_jobs::FolderJobRegistry::new(
                self.scheduler.clone(),
            )),
            shared_coordinator: None,
            protected_groups,
            ..self.clone()
        }
    }

    pub fn root_id(&self) -> Option<String> {
        self.hosted_library_id.map(|id| id.as_uuid().to_string())
    }

    pub async fn check_relative(
        &self,
        relative: &Path,
    ) -> Result<crate::AccessReply, AppServiceError> {
        if relative.is_absolute()
            || relative.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(photo_core::AddLibraryError::InvalidSelection.into());
        }
        let target = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library_id = self
                .hosted_library_id
                .ok_or(AppServiceError::UnknownAsset)?;
            let library = state
                .libraries
                .catalog()
                .find_library(library_id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            crate::FolderAccessTarget {
                key: crate::FolderAccessKey {
                    library_id,
                    relative: RelativePathKey::from_relative_path(relative)
                        .map_err(|_| photo_core::AddLibraryError::InvalidSelection)?,
                },
                root: library
                    .canonical_root_key
                    .to_path_buf()
                    .map_err(|e| CatalogError::InvalidData(e.to_string()))?,
            }
        };
        Ok(self.folder_access.check_with_root(target).await)
    }

    pub async fn select_relative(
        &self,
        relative: &Path,
    ) -> Result<SelectionSummary, AppServiceError> {
        let reply = self.check_relative(relative).await?;
        match reply {
            crate::AccessReply::Complete {
                outcome: crate::FolderProbeOutcome::Available(proof),
                ..
            } => self.select_validated(&proof),
            crate::AccessReply::Complete {
                outcome: crate::FolderProbeOutcome::Invalid,
                ..
            } => Err(photo_core::AddLibraryError::InvalidSelection.into()),
            _ => Err(photo_core::AddLibraryError::NotDirectory(relative.to_path_buf()).into()),
        }
    }

    pub fn select_validated(
        &self,
        proof: &crate::ValidatedFolder,
    ) -> Result<SelectionSummary, AppServiceError> {
        if self.hosted_library_id != Some(proof.key().library_id) {
            return Err(AppServiceError::UnknownAsset);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let group = state
            .libraries
            .catalog_mut()
            .upsert_folder_group(&NewFolderGroup {
                id: FolderGroupId::new(),
                library_id: proof.key().library_id,
                relative_path: proof.key().relative.clone(),
                display_path: proof
                    .key()
                    .relative
                    .to_path_buf()
                    .map_err(|e| CatalogError::InvalidData(e.to_string()))?
                    .to_string_lossy()
                    .into_owned(),
                last_viewed_at: Some(crate::service::unix_timestamp()),
            })?;
        let selection = GallerySelection {
            id: format!("selection-{}", group.as_uuid()),
            library_id: proof.key().library_id,
            group_id: group,
            relative_folder: proof.key().relative.clone(),
            epoch: 0,
        };
        drop(state);
        self.selection_summary(&selection).map(|mut summary| {
            summary.availability = SourceAvailability::Available;
            summary
        })
    }

    pub fn resolve_selection(&self, id: &str) -> Result<GallerySelection, AppServiceError> {
        let uuid = id
            .strip_prefix("selection-")
            .ok_or(AppServiceError::UnknownAsset)
            .and_then(|s| uuid::Uuid::parse_str(s).map_err(|_| AppServiceError::UnknownAsset))?;
        let canonical_id = format!("selection-{}", uuid.hyphenated());
        if id != canonical_id {
            return Err(AppServiceError::UnknownAsset);
        }
        let group = FolderGroupId::from_uuid(uuid);
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let record = state
            .libraries
            .catalog()
            .folder_group(group)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if self
            .hosted_library_id
            .is_some_and(|library_id| record.library_id != library_id)
        {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(GallerySelection {
            id: id.to_owned(),
            library_id: record.library_id,
            group_id: record.id,
            relative_folder: record.relative_path,
            epoch: 0,
        })
    }

    pub(crate) fn selection_from_token(
        &self,
        token: crate::service::SelectionToken,
    ) -> Result<GallerySelection, AppServiceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let record = state
            .libraries
            .catalog()
            .folder_group(token.group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if record.library_id != token.library_id {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(GallerySelection {
            id: token.selection_id(),
            library_id: record.library_id,
            group_id: record.id,
            relative_folder: record.relative_path,
            epoch: token.epoch,
        })
    }

    pub fn selection_summary(
        &self,
        selection: &GallerySelection,
    ) -> Result<SelectionSummary, AppServiceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let group = state
            .libraries
            .catalog()
            .folder_group(selection.group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if self
            .hosted_library_id
            .is_some_and(|library_id| group.library_id != library_id)
            || group.library_id != selection.library_id
            || group.relative_path != selection.relative_folder
        {
            return Err(AppServiceError::UnknownAsset);
        }
        let library = state
            .libraries
            .catalog()
            .find_library(group.library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let relative = group
            .relative_path
            .to_path_buf()
            .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
        let mut breadcrumbs = Vec::new();
        let mut path = PathBuf::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            path.push(name);
            breadcrumbs.push(FolderBreadcrumb {
                name: name.to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
            });
        }
        Ok(SelectionSummary {
            folder_id: group.id.as_uuid().to_string(),
            path: relative.to_string_lossy().into_owned(),
            id: selection.id.clone(),
            source_id: group.library_id.as_uuid().hyphenated().to_string(),
            display_name: relative
                .file_name()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or(library.display_name),
            breadcrumbs,
            availability: crate::service::map_availability(library.availability),
        })
    }

    fn runtime(&self, selection: &GallerySelection) -> Arc<SelectionRuntime> {
        self.folder_jobs
            .ensure((selection.library_id, selection.group_id), || {
                // Cache-only derivative requests inherit durable settlement. Explicit
                // desktop scan admission below separately requests a refresh.
                let (settled, reconciled_recovery_token) = {
                    self.state
                        .lock()
                        .ok()
                        .and_then(|state| {
                            let catalog = state.libraries.catalog();
                            let settled = catalog
                                .has_completed_generation_for_group(
                                    selection.library_id,
                                    selection.group_id,
                                )
                                .ok()?
                                && !catalog
                                    .heif_metadata_refresh_required(
                                        selection.library_id,
                                        selection.group_id,
                                    )
                                    .ok()?;
                            let reconciled = if self.hosted_library_id == Some(selection.library_id)
                            {
                                catalog
                                    .folder_group_recovery_state(
                                        selection.library_id,
                                        selection.group_id,
                                    )
                                    .ok()?
                                    .reconciled
                            } else {
                                0
                            };
                            Some((settled, reconciled))
                        })
                        .unwrap_or((false, 0))
                };
                let initial_coordinator = self.shared_coordinator.as_ref().filter(|_| {
                    !self
                        .desktop_coordinator_claimed
                        .swap(true, Ordering::AcqRel)
                });
                let runtime = match initial_coordinator {
                    Some(coordinator) => SelectionRuntime::new_with_coordinator_at_recovery(
                        selection.clone(),
                        coordinator.clone(),
                        settled,
                        reconciled_recovery_token,
                    ),
                    None => SelectionRuntime::new_at_recovery(
                        selection.clone(),
                        self.scheduler.clone(),
                        settled,
                        reconciled_recovery_token,
                    ),
                };
                if self.shared_coordinator.is_some()
                    && let Ok(state) = self.state.lock()
                    && let Ok(stored) = state.libraries.catalog().load_app_state()
                {
                    *runtime
                        .desktop_scope
                        .lock()
                        .expect("desktop scope poisoned") = stored.gallery_scope;
                }
                runtime
            })
    }

    /// Captures ownership once. Selection changes can only affect publication,
    /// never redirect this adapter's catalog reads or derivative commits.
    pub(crate) fn bind_desktop_derivatives(
        &self,
        service: &crate::AppService,
        selection: crate::service::SelectionToken,
    ) -> Result<crate::AppService, AppServiceError> {
        let runtime = self.runtime(&self.selection_from_token(selection)?);
        if !self.folder_jobs.admits_binding(&runtime) {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        let controls = runtime.desktop_controls.get_or_init(|| {
            let first = Arc::ptr_eq(&runtime.coordinator, &service.coordinator);
            crate::hosted_runtime::DesktopDerivativeControls {
                worker: if first {
                    service.derivative_driver.clone()
                } else {
                    Arc::new(TokioMutex::new(()))
                },
                collection: if first {
                    service.collection_driver.clone()
                } else {
                    Arc::new(crate::service::CollectionDriverControl::new())
                },
            }
        });
        let mut bound = service.clone();
        bound.coordinator = runtime.coordinator.clone();
        bound.derivative_driver = controls.worker.clone();
        bound.collection_driver = controls.collection.clone();
        bound.derivative_runtime = Some(runtime);
        Ok(bound)
    }

    pub(crate) fn remove_runtime_if_dead(
        &self,
        group_id: FolderGroupId,
        runtime: &Arc<SelectionRuntime>,
    ) {
        self.folder_jobs
            .finish((runtime.selection.library_id, group_id), runtime);
    }

    /// Requests cancellation of the admitted scan for a desktop selection.
    /// The runtime remains responsible for draining and persisting any events
    /// already in flight; this only replaces the old AppService-owned sender.
    pub(crate) fn cancel_runtime_scan(&self, group_id: FolderGroupId) {
        let runtime = self.folder_jobs.runtimes.lock().ok().and_then(|runtimes| {
            runtimes
                .iter()
                .find(|(key, _)| key.1 == group_id)
                .and_then(|(_, runtime)| Weak::upgrade(runtime))
        });
        let Some(runtime) = runtime else {
            return;
        };
        runtime.request_cancel();
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn cancel_runtime_scan_for_test(&self, selection: &GallerySelection) {
        self.cancel_runtime_scan(selection.group_id);
    }

    pub async fn ensure_running(
        &self,
        selection: &GallerySelection,
    ) -> Result<(), AppServiceError> {
        if self.folder_jobs.is_shutdown() {
            return Ok(());
        }
        let runtime = self.runtime(selection);
        if self.shared_coordinator.is_some() && runtime.prepare_desktop_refresh(selection.epoch) {
            runtime.coordinator.invalidate_background().await;
            runtime
                .coordinator
                .reset_collection_for_scope(runtime.selection.token())
                .await;
        }
        runtime.begin_scan(self.clone()).await
    }

    pub(crate) fn runtime_recovery_state(
        &self,
        runtime: &SelectionRuntime,
    ) -> Result<Option<photo_catalog::FolderGroupRecoveryState>, AppServiceError> {
        if self.hosted_library_id != Some(runtime.selection.library_id) {
            return Ok(None);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let catalog = state.libraries.catalog();
        let library = catalog
            .find_library(runtime.selection.library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if library.availability != photo_domain::Availability::Available {
            return Ok(None);
        }
        catalog
            .folder_group_recovery_state(runtime.selection.library_id, runtime.selection.group_id)
            .map(Some)
            .map_err(Into::into)
    }

    /// Resolves a catalog identity only within the configured hosted library.
    /// The caller must open the relative key from its pinned source descriptor.
    pub fn resolve_hosted_original(
        &self,
        asset_id: photo_domain::AssetId,
    ) -> Result<(RelativePathKey, String, photo_domain::MediaKind), AppServiceError> {
        let library_id = self
            .hosted_library_id
            .ok_or(AppServiceError::ForeignAsset)?;
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let catalog = state.libraries.catalog();
        let asset = catalog
            .find_asset(asset_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if asset.library_id != library_id {
            return Err(AppServiceError::ForeignAsset);
        }
        let library = catalog
            .find_library(library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if asset.availability != photo_domain::Availability::Available
            || library.availability != photo_domain::Availability::Available
        {
            return Err(AppServiceError::UnknownAsset);
        }
        let path = asset
            .relative_path
            .to_path_buf()
            .map_err(|_| AppServiceError::UnknownAsset)?;
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(AppServiceError::UnknownAsset);
        }
        let filename = path
            .file_name()
            .ok_or(AppServiceError::UnknownAsset)?
            .to_string_lossy()
            .into_owned();
        Ok((asset.relative_path, filename, asset.media_kind))
    }

    /// Resolves a bounded pick batch without opening a source or starting a scan.
    pub fn resolve_assets(
        &self,
        selection: &GallerySelection,
        asset_ids: &[String],
    ) -> Result<Vec<Option<crate::WallAsset>>, AppServiceError> {
        let ids = crate::picks::parse_pick_asset_ids(asset_ids)?;
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let catalog = state.libraries.catalog();
        let group = catalog
            .folder_group(selection.group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if group.library_id != selection.library_id
            || group.relative_path != selection.relative_folder
            || self
                .hosted_library_id
                .is_some_and(|library| library != group.library_id)
        {
            return Err(AppServiceError::ForeignAsset);
        }
        let records = catalog.wall_records_for_assets_scoped(
            selection.group_id,
            GalleryScope::IncludeSubfolders,
            &ids,
        )?;
        for id in &ids {
            if let Some(asset) = catalog.find_asset(*id)?
                && (asset.library_id != selection.library_id
                    || !records.iter().any(|record| record.id == *id))
            {
                return Err(AppServiceError::ForeignAsset);
            }
        }
        let order = if catalog
            .has_completed_generation_for_group(selection.library_id, selection.group_id)?
        {
            OrderState::Settled
        } else {
            OrderState::Provisional
        };
        let assets = crate::service::wall_assets_with_derivatives(catalog, &records, order)?
            .into_iter()
            .map(|asset| (asset.id.clone(), asset))
            .collect::<HashMap<_, _>>();
        Ok(ids
            .iter()
            .map(|id| assets.get(&id.as_uuid().to_string()).cloned())
            .collect())
    }

    pub async fn query_wall(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: WallQueryRequest,
    ) -> Result<WallPage, AppServiceError> {
        if !(1..=250).contains(&request.limit) {
            return Err(AppServiceError::InvalidLimit);
        }
        let inventory_total = self.runtime(selection).inventory_total(scope);
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let settled = state
            .libraries
            .catalog()
            .has_completed_generation_for_group(selection.library_id, selection.group_id)?;
        let order = if settled {
            match request.direction {
                crate::SortDirection::OldestFirst => WallOrder::CapturedAscending,
                crate::SortDirection::NewestFirst => WallOrder::CapturedDescending,
            }
        } else {
            WallOrder::Provisional
        };
        let cursor = request
            .cursor
            .as_deref()
            .map(|value| {
                crate::wall::decode_cursor(value, request.direction, scope, selection, order)
            })
            .transpose()?;
        let page = state.libraries.catalog().wall_page_scoped(
            selection.group_id,
            scope,
            order,
            cursor,
            request.limit,
        )?;
        let catalog_total_count = state
            .libraries
            .catalog()
            .wall_photo_count_scoped(selection.group_id, scope)?;
        let preview_counts = state
            .libraries
            .catalog()
            .wall_preview_counts_scoped(selection.group_id, scope)?;
        let total_count = if settled {
            catalog_total_count
        } else {
            inventory_total.unwrap_or(catalog_total_count)
        };
        let items = crate::service::wall_assets_with_derivatives(
            state.libraries.catalog(),
            &page.items,
            if settled {
                OrderState::Settled
            } else {
                OrderState::Provisional
            },
        )?;
        let next_cursor = page
            .next
            .as_ref()
            .map(|key| crate::wall::encode_cursor(request.direction, scope, selection, key))
            .transpose()?;
        Ok(WallPage {
            items,
            next_cursor,
            order_state: if settled {
                OrderState::Settled
            } else {
                OrderState::Provisional
            },
            source_warnings: state
                .libraries
                .catalog()
                .source_warning_summaries(selection.library_id)?
                .into_iter()
                .map(|w| crate::service::map_source_warning_code(&w.code))
                .collect(),
            total_count,
            indexed_count: catalog_total_count,
            preview_counts: WallPreviewCounts {
                wall_ready: preview_counts.wall_ready,
                screen_ready: preview_counts.screen_ready,
            },
        })
    }

    /// Generates only the derivatives explicitly requested by this selection.
    /// Membership is checked before any source file is opened. Work is then
    /// admitted through the selection runtime's coordinator, so duplicate
    /// callers share one queued attempt and inherit its cancellation and lane
    /// semantics.
    pub async fn request_derivatives(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        self.run_derivative_request(
            selection,
            scope,
            request,
            HostedDerivativeRequestMode::AwaitCompletion,
        )
        .await
    }

    /// Validates, reuses, or queues hosted derivative work before returning,
    /// without retaining one completion task per HTTP request. The coordinator
    /// keeps a compact authorization marker per scope and publishes completion
    /// through the selection event stream.
    pub async fn admit_derivatives(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        self.run_derivative_request(
            selection,
            scope,
            request,
            HostedDerivativeRequestMode::AdmitOnly,
        )
        .await
    }

    async fn run_derivative_request(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: DerivativeRequest,
        mode: HostedDerivativeRequestMode,
    ) -> Result<(), AppServiceError> {
        if self.folder_jobs.is_shutdown() {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        let assets = self.validate_derivative_request(selection, scope, &request)?;
        #[cfg(debug_assertions)]
        if let Some((entered, release)) = self.hosted_post_validation_test_gate.lock().await.take()
        {
            let notified = release.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            entered.notify_one();
            notified.await;
        }

        let runtime = self.runtime(selection);
        if !self.folder_jobs.admits_binding(&runtime) {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        let _protected =
            ProtectedGroupGuard::new(self.protected_groups.clone(), selection.group_id)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
        let classes = if request.kind == DerivativeClass::ScreenPreview {
            vec![
                DerivativeClass::WallThumbnail,
                DerivativeClass::ScreenPreview,
            ]
        } else {
            vec![DerivativeClass::WallThumbnail]
        };
        for class in classes {
            let (receivers, queued, capacity_exceeded) = self
                .queue_requested_derivative_class(
                    &runtime,
                    HostedDerivativeBatch {
                        selection,
                        scope,
                        priority: request.priority,
                        class,
                        assets: &assets,
                        mode,
                    },
                )
                .await?;
            if queued {
                self.start_hosted_derivative_driver(runtime.clone()).await;
                if matches!(mode, HostedDerivativeRequestMode::AdmitOnly)
                    && request.priority == DerivativePriority::Visible
                    && class == DerivativeClass::WallThumbnail
                {
                    self.start_hosted_visible_driver(runtime.clone()).await;
                }
            }
            if capacity_exceeded {
                return Err(AppServiceError::DerivativeUnavailable);
            }
            for receiver in receivers {
                match receiver.await {
                    Ok(crate::derivative_coordinator::DerivativeResult::Ready(_)) => {}
                    Ok(crate::derivative_coordinator::DerivativeResult::Failed) => {
                        return Err(AppServiceError::DerivativeFailed);
                    }
                    Ok(crate::derivative_coordinator::DerivativeResult::Unavailable) | Err(_) => {
                        return Err(AppServiceError::DerivativeUnavailable);
                    }
                }
            }
        }
        Ok(())
    }

    async fn queue_requested_derivative_class(
        &self,
        runtime: &Arc<SelectionRuntime>,
        batch: HostedDerivativeBatch<'_>,
    ) -> Result<(Vec<WorkResultReceiver>, bool, bool), AppServiceError> {
        let mut receivers = Vec::new();
        let mut queued = false;
        let mut capacity_exceeded = false;
        for asset in batch.assets {
            if !asset.media_kind.is_wall_viewable() {
                continue;
            }
            let spec = derivative_spec_for_gallery(asset, batch.class);
            let key = DerivativeKey::compute(&spec);
            let existing = async {
                let _publication = self.hosted_publication_fence.lock().await;
                let existing = {
                    let state = self
                        .state
                        .lock()
                        .map_err(|_| AppServiceError::StatePoisoned)?;
                    state.libraries.catalog().find_derivative(
                        asset.id,
                        derivative_kind_name_for_gallery(batch.class),
                        key.as_str(),
                    )?
                };
                let existing_is_valid = match existing.as_ref() {
                    Some(record) => self.derivative_record_is_valid_async(record).await,
                    None => false,
                };
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| AppServiceError::StatePoisoned)?;
                let result = if let Some(record) = existing.as_ref() {
                    if existing_is_valid {
                        let authorized = state.libraries.catalog().wall_records_for_assets_scoped(
                            batch.selection.group_id,
                            batch.scope,
                            &[asset.id],
                        )?;
                        if authorized.is_empty() {
                            return Err(AppServiceError::DerivativeUnavailable);
                        }
                        state
                            .libraries
                            .catalog_mut()
                            .link_derivative_group(record.id, batch.selection.group_id)?;
                        Some(record.cache_key.clone())
                    } else {
                        self.remove_requester_derivative_link(
                            state.libraries.catalog_mut(),
                            record,
                            batch.selection.group_id,
                        )?;
                        None
                    }
                } else {
                    None
                };
                Ok::<_, AppServiceError>(result)
            }
            .await
            .map_err(|_| AppServiceError::DerivativeFailed)?;
            if let Some(key) = existing {
                self.clear_hosted_derivative_warning(runtime, runtime.selection.token(), asset.id)
                    .await;
                runtime
                    .publish(WallUpdate::DerivativesReady {
                        selection_id: batch.selection.id.clone(),
                        derivatives: vec![DerivativeReference {
                            asset_id: asset.id.as_uuid().hyphenated().to_string(),
                            kind: batch.class,
                            key,
                        }],
                        preview_counts: None,
                    })
                    .await;
                continue;
            }
            let lane = match (batch.class, batch.priority) {
                (DerivativeClass::WallThumbnail, DerivativePriority::Visible) => {
                    WorkLane::VisibleWall
                }
                (DerivativeClass::WallThumbnail, DerivativePriority::NearViewport) => {
                    WorkLane::NearWall
                }
                (DerivativeClass::ScreenPreview, DerivativePriority::Visible) => {
                    WorkLane::ViewerPreview
                }
                (DerivativeClass::ScreenPreview, DerivativePriority::NearViewport) => {
                    WorkLane::IdlePreview
                }
            };
            let prerequisite = (batch.class == DerivativeClass::ScreenPreview).then(|| {
                DerivativeKey::compute(&derivative_spec_for_gallery(
                    asset,
                    DerivativeClass::WallThumbnail,
                ))
                .as_str()
                .to_owned()
            });
            let work_key = WorkKey {
                selection: runtime.selection.token(),
                asset_id: asset.id,
                class: batch.class,
                cache_key: key.as_str().to_owned(),
                availability: asset.availability,
                scope: batch.scope,
            };
            runtime.coordinator.invalidate_completed(&work_key).await;
            #[cfg(debug_assertions)]
            if let Some((entered, release)) = self.hosted_pre_enqueue_test_gate.lock().await.take()
            {
                let notified = release.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                entered.notify_one();
                notified.await;
            }
            match batch.mode {
                HostedDerivativeRequestMode::AwaitCompletion => {
                    receivers.push(
                        runtime
                            .coordinator
                            .enqueue_hosted_with_prerequisite_observed(
                                work_key,
                                lane,
                                prerequisite,
                                None,
                                self.hosted_scope_authorizer(),
                            )
                            .await,
                    );
                }
                HostedDerivativeRequestMode::AdmitOnly => {
                    let admitted = runtime
                        .coordinator
                        .admit_hosted_with_prerequisite_observed(
                            work_key,
                            lane,
                            prerequisite,
                            None,
                            self.hosted_scope_authorizer(),
                        )
                        .await;
                    if !admitted {
                        capacity_exceeded = true;
                        break;
                    }
                }
            }
            queued = true;
        }
        Ok((receivers, queued, capacity_exceeded))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn hosted_derivative_attempts_for_test(&self) -> usize {
        self.hosted_attempts.load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn hosted_derivative_active_count_for_test(&self) -> usize {
        self.hosted_active.load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn hosted_derivative_peak_count_for_test(&self) -> usize {
        self.hosted_active_peak.load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn hosted_derivative_admission_cap_for_test(&self) -> usize {
        self.hosted_admission_limit()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn set_hosted_derivative_admission_cap_for_test(&self, limit: usize) {
        self.hosted_admission_limit_override
            .store(limit.clamp(1, HOSTED_DERIVATIVE_WORKERS), Ordering::Release);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn hosted_waiter_count_for_test(&self, selection: &GallerySelection) -> usize {
        self.runtime(selection).coordinator.waiter_count().await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn hosted_pending_job_count_for_test(&self, selection: &GallerySelection) -> usize {
        self.runtime(selection)
            .coordinator
            .pending_job_count()
            .await
    }

    fn try_hosted_slot(&self) -> bool {
        self.try_hosted_slot_with_limit(self.hosted_admission_limit())
    }

    fn try_hosted_visible_slot(&self) -> bool {
        self.try_hosted_slot_with_limit(HOSTED_VISIBLE_BURST_WORKERS)
    }

    fn try_hosted_slot_with_limit(&self, limit: usize) -> bool {
        let mut active = self.hosted_active.load(Ordering::Acquire);
        loop {
            if active >= limit {
                return false;
            }
            match self.hosted_active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    #[cfg(debug_assertions)]
                    self.hosted_active_peak
                        .fetch_max(active + 1, Ordering::AcqRel);
                    return true;
                }
                Err(next) => active = next,
            }
        }
    }

    fn hosted_admission_limit(&self) -> usize {
        #[cfg(debug_assertions)]
        {
            let forced = self.hosted_admission_limit_override.load(Ordering::Acquire);
            if forced > 0 {
                return forced;
            }
        }
        self.scheduler
            .available_background_permits()
            .clamp(1, HOSTED_DERIVATIVE_WORKERS)
    }

    fn hosted_scope_authorizer(&self) -> crate::derivative_coordinator::ScopeAuthorizer {
        let engine = self.clone();
        Arc::new(move |key, scope| engine.hosted_scope_is_current(key, scope))
    }

    fn hosted_scope_is_current(&self, key: &WorkKey, scope: GalleryScope) -> bool {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .libraries
                    .catalog()
                    .wall_records_for_assets_scoped(key.selection.group_id, scope, &[key.asset_id])
                    .ok()
            })
            .is_some_and(|records| !records.is_empty())
    }

    /// Reserves one hosted capacity slot and dequeues a job while the shared
    /// admission owner is held. This makes selection drivers contenders for
    /// one central admission decision instead of letting each driver consume
    /// capacity before discovering that another selection has better work.
    async fn try_admit_hosted_work(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Option<(WorkTicket, HostedAdmission)> {
        let _owner = self.hosted_admission_owner.lock().await;
        let permit = self.derivative_admission.clone().try_acquire_owned().ok()?;
        if !self.try_hosted_slot() {
            drop(permit);
            return None;
        }
        let admission = HostedAdmission {
            permit: Some(permit),
            active: self.hosted_active.clone(),
            wake: self.derivative_admission_wake.clone(),
        };
        let Some(ticket) = runtime.coordinator.next_work().await else {
            drop(admission);
            return None;
        };
        Some((ticket, admission))
    }

    async fn try_admit_hosted_visible_work(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Option<(WorkTicket, HostedAdmission)> {
        let _owner = self.hosted_admission_owner.lock().await;
        let visible_priority =
            crate::derivative_coordinator::scheduler_priority(WorkLane::VisibleWall);
        if self
            .scheduler
            .highest_priority_in_family(crate::derivative_coordinator::SCHEDULER_OWNER_PREFIX)
            .await
            != Some(visible_priority)
        {
            return None;
        }
        let permit = self.derivative_admission.clone().try_acquire_owned().ok()?;
        if !self.try_hosted_visible_slot() {
            drop(permit);
            return None;
        }
        let admission = HostedAdmission {
            permit: Some(permit),
            active: self.hosted_active.clone(),
            wake: self.derivative_admission_wake.clone(),
        };
        let Some(ticket) = runtime.coordinator.next_work().await else {
            drop(admission);
            return None;
        };
        if runtime.coordinator.lane(ticket).await != WorkLane::VisibleWall {
            runtime.coordinator.requeue(ticket).await;
            drop(admission);
            return None;
        }
        Some((ticket, admission))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_commit_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_commit_publication_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_commit_publication_test_gate(&self) {
        let gate = self.hosted_commit_publication_test_gate.lock().await.take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_commit_post_publication_test_gate(&self) {
        let gate = self
            .hosted_commit_post_publication_test_gate
            .lock()
            .await
            .take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_authorized_publication_test_gate(&self) {
        let gate = self
            .hosted_authorized_publication_test_gate
            .lock()
            .await
            .take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    /// Validates derivative identifiers and selection scope without starting
    /// a scan or touching source media. HTTP callers use this boundary before
    /// admitting work for a selection.
    pub fn validate_derivative_request(
        &self,
        selection: &GallerySelection,
        scope: GalleryScope,
        request: &DerivativeRequest,
    ) -> Result<Vec<photo_catalog::AssetRecord>, AppServiceError> {
        if !(1..=250).contains(&request.asset_ids.len()) {
            return Err(AppServiceError::InvalidLimit);
        }
        let ids = request
            .asset_ids
            .iter()
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map(photo_domain::AssetId::from_uuid)
                    .map_err(|_| AppServiceError::InvalidAssetId)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let state = self
            .state
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let allowed = state.libraries.catalog().wall_records_for_assets_scoped(
            selection.group_id,
            scope,
            &ids,
        )?;
        if allowed.len() != ids.len() {
            return Err(AppServiceError::ForeignAsset);
        }
        ids.iter()
            .map(|id| {
                let asset = state
                    .libraries
                    .catalog()
                    .find_asset(*id)?
                    .ok_or(AppServiceError::UnknownAsset)?;
                if asset.library_id != selection.library_id {
                    return Err(AppServiceError::ForeignAsset);
                }
                Ok(asset)
            })
            .collect::<Result<Vec<_>, AppServiceError>>()
    }

    fn persist_hosted_derivative_warning(&self, key: &WorkKey) {
        let _ = self.state.lock().ok().and_then(|mut state| {
            state
                .libraries
                .catalog_mut()
                .record_warning_once(&photo_catalog::CatalogWarningRecord {
                    library_id: key.selection.library_id,
                    asset_id: Some(key.asset_id),
                    code: HOSTED_ASSET_DERIVATIVE_WARNING.to_owned(),
                    message: "A cached preview could not be generated. The app can retry it."
                        .to_owned(),
                })
                .ok()
        });
    }

    async fn emit_hosted_derivative_warning(&self, runtime: &SelectionRuntime, key: &WorkKey) {
        runtime
            .publish(WallUpdate::Warning {
                selection_id: runtime.selection.id.clone(),
                source_id: key.selection.library_id.as_uuid().hyphenated().to_string(),
                asset_id: Some(key.asset_id.as_uuid().hyphenated().to_string()),
                warning: crate::WallWarningState {
                    code: "derivativeUnavailable".to_owned(),
                    retryable: true,
                },
            })
            .await;
    }

    fn hosted_work_remains_in_scope(&self, key: &WorkKey) -> bool {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                let asset = state.libraries.catalog().find_asset(key.asset_id).ok()??;
                if !asset.media_kind.is_wall_viewable()
                    || asset.availability != key.availability
                    || DerivativeKey::compute(&derivative_spec_for_gallery(&asset, key.class))
                        .as_str()
                        != key.cache_key
                {
                    return None;
                }
                state
                    .libraries
                    .catalog()
                    .wall_records_for_assets_scoped(
                        key.selection.group_id,
                        key.scope,
                        &[key.asset_id],
                    )
                    .ok()
            })
            .is_some_and(|records| !records.is_empty())
    }

    async fn clear_hosted_derivative_warning(
        &self,
        runtime: &SelectionRuntime,
        selection: crate::service::SelectionToken,
        asset_id: photo_domain::AssetId,
    ) {
        let removed = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| {
                state
                    .libraries
                    .catalog_mut()
                    .clear_warning(
                        selection.library_id,
                        Some(asset_id),
                        HOSTED_ASSET_DERIVATIVE_WARNING,
                    )
                    .ok()
            })
            .unwrap_or(0);
        if removed > 0 {
            runtime
                .publish(WallUpdate::WarningCleared {
                    selection_id: runtime.selection.id.clone(),
                    source_id: selection.library_id.as_uuid().hyphenated().to_string(),
                    asset_id: Some(asset_id.as_uuid().hyphenated().to_string()),
                    code: "derivativeUnavailable".to_owned(),
                })
                .await;
        }
    }

    async fn start_hosted_derivative_driver(&self, runtime: Arc<SelectionRuntime>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let Some(generation) = runtime.coordinator.claim_driver_owner().await else {
            return;
        };
        self.spawn_hosted_derivative_driver(runtime, generation, HostedDriverKind::Durable)
            .await;
    }

    async fn start_hosted_visible_driver(&self, runtime: Arc<SelectionRuntime>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let Some(generation) = runtime.coordinator.claim_visible_driver_owner().await else {
            return;
        };
        self.spawn_hosted_derivative_driver(runtime, generation, HostedDriverKind::VisibleBurst)
            .await;
    }

    async fn spawn_hosted_derivative_driver(
        &self,
        runtime: Arc<SelectionRuntime>,
        generation: u64,
        kind: HostedDriverKind,
    ) {
        let engine = self.clone();
        let active_ticket = Arc::new(Mutex::new(None));
        let active_attempt = Arc::new(Mutex::new(None));
        let active_attempt_lease = Arc::new(Mutex::new(None));
        let driver = tokio::spawn(async move {
            let _owner = HostedDriverOwner {
                coordinator: runtime.coordinator.clone(),
                generation,
                engine: engine.clone(),
                runtime: runtime.clone(),
                active_ticket: active_ticket.clone(),
                active_attempt: active_attempt.clone(),
                active_attempt_lease: active_attempt_lease.clone(),
                kind,
            };
            loop {
                let coordinator = runtime.coordinator.clone();
                let observed_coordinator = coordinator.change_generation();
                let observed_scheduler = engine.scheduler.change_generation();
                let next = match kind {
                    HostedDriverKind::Durable => engine.try_admit_hosted_work(&runtime).await,
                    HostedDriverKind::VisibleBurst => {
                        engine.try_admit_hosted_visible_work(&runtime).await
                    }
                };
                let Some((ticket, admission)) = next else {
                    let should_exit = match kind {
                        HostedDriverKind::Durable => {
                            coordinator.pending_job_count().await == 0
                                && coordinator.release_driver_owner_if_idle(generation).await
                        }
                        HostedDriverKind::VisibleBurst => {
                            if coordinator.has_queued_visible_wall_work().await {
                                false
                            } else {
                                coordinator.release_visible_driver_owner(generation).await;
                                true
                            }
                        }
                    };
                    if should_exit {
                        break;
                    }
                    let coordinator_wake = coordinator.wait_for_change_since(observed_coordinator);
                    let scheduler_wake = engine.scheduler.wait_for_change_since(observed_scheduler);
                    let admission_wake = engine.derivative_admission_wake.notified();
                    tokio::pin!(coordinator_wake);
                    tokio::pin!(scheduler_wake);
                    tokio::pin!(admission_wake);
                    tokio::select! {
                        _ = &mut coordinator_wake => {}
                        _ = &mut scheduler_wake => {}
                        _ = &mut admission_wake => {}
                    }
                    continue;
                };
                let lane = coordinator.lane(ticket).await;
                if engine
                    .scheduler
                    .highest_priority_in_family(
                        crate::derivative_coordinator::SCHEDULER_OWNER_PREFIX,
                    )
                    .await
                    .is_some_and(|priority| {
                        priority > crate::derivative_coordinator::scheduler_priority(lane)
                    })
                {
                    let observed = coordinator.change_generation();
                    let observed_scheduler = engine.scheduler.change_generation();
                    coordinator.requeue(ticket).await;
                    drop(admission);
                    let coordinator_wake = coordinator.wait_for_change_since(observed);
                    let scheduler_wake = engine.scheduler.wait_for_change_since(observed_scheduler);
                    tokio::pin!(coordinator_wake);
                    tokio::pin!(scheduler_wake);
                    tokio::select! {
                        _ = &mut coordinator_wake => {}
                        _ = &mut scheduler_wake => {}
                    }
                    continue;
                }
                let Some(key) = coordinator.work_key(ticket).await else {
                    drop(admission);
                    continue;
                };
                *active_ticket.lock().expect("hosted active ticket poisoned") = Some(ticket);
                let Some(attempt_lease) = coordinator.claim_hosted_attempt(ticket).await else {
                    coordinator.discard(ticket).await;
                    *active_ticket.lock().expect("hosted active ticket poisoned") = None;
                    drop(admission);
                    continue;
                };
                *active_attempt_lease
                    .lock()
                    .expect("hosted active attempt lease poisoned") = Some(attempt_lease.clone());
                let warning_key = key.clone();
                let attempt = tokio::spawn({
                    let engine = engine.clone();
                    let runtime = runtime.clone();
                    let attempt_lease = attempt_lease.clone();
                    async move {
                        let _attempt_owner = HostedAttemptTaskOwner {
                            attempt: attempt_lease.clone(),
                        };
                        engine
                            .process_hosted_derivative(
                                runtime,
                                ticket,
                                key,
                                admission,
                                attempt_lease,
                            )
                            .await;
                    }
                });
                *active_attempt
                    .lock()
                    .expect("hosted active attempt poisoned") = Some(attempt.abort_handle());
                if attempt.await.is_err() {
                    engine.persist_hosted_derivative_warning(&warning_key);
                    runtime
                        .coordinator
                        .abort_hosted_attempt_as_failure(ticket)
                        .await;
                    engine
                        .emit_hosted_derivative_warning(&runtime, &warning_key)
                        .await;
                } else {
                    runtime
                        .coordinator
                        .recover_hosted_publication_as_failure(ticket)
                        .await;
                }
                *active_attempt
                    .lock()
                    .expect("hosted active attempt poisoned") = None;
                *active_attempt_lease
                    .lock()
                    .expect("hosted active attempt lease poisoned") = None;
                *active_ticket.lock().expect("hosted active ticket poisoned") = None;
            }
        });
        #[cfg(debug_assertions)]
        {
            if matches!(kind, HostedDriverKind::Durable) {
                *self.hosted_driver_abort_handle.lock().await = Some(driver.abort_handle());
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = driver;
    }

    async fn process_hosted_derivative(
        &self,
        runtime: Arc<SelectionRuntime>,
        ticket: WorkTicket,
        key: WorkKey,
        admission: HostedAdmission,
        attempt_lease: HostedAttemptLease,
    ) {
        let Some((asset, source, spec)) = self.pending_hosted_derivative(&key).await else {
            let warn = self.hosted_work_remains_in_scope(&key);
            if warn {
                self.persist_hosted_derivative_warning(&key);
            }
            runtime.coordinator.discard(ticket).await;
            if warn {
                self.emit_hosted_derivative_warning(&runtime, &key).await;
            }
            return;
        };
        #[cfg(debug_assertions)]
        if self.consume_hosted_panic_before_admission_test_hook().await {
            panic!("injected hosted derivative panic before commit admission");
        }
        #[cfg(debug_assertions)]
        self.hosted_attempts.fetch_add(1, Ordering::AcqRel);
        let prerequisite = (key.class == DerivativeClass::ScreenPreview).then(|| {
            DerivativeKey::compute(&derivative_spec_for_gallery(
                &asset,
                DerivativeClass::WallThumbnail,
            ))
            .as_str()
            .to_owned()
        });
        let Some(permit) = runtime
            .coordinator
            .admit_commit(ticket, key.selection, prerequisite.as_deref())
            .await
        else {
            runtime.coordinator.discard(ticket).await;
            return;
        };
        // Every hosted commit has exactly one live owner. The owner Drop path
        // can claim an abort while the attempt is encoding, and all later
        // publication boundaries reject the cancelled permit.
        if !permit.claim_supervisor() {
            return;
        }
        let group = key.selection.group_id;
        let Ok(_protected) = ProtectedGroupGuard::new(self.protected_groups.clone(), group) else {
            self.persist_hosted_derivative_warning(&key);
            runtime.coordinator.fail_commit_with_error(permit).await;
            self.emit_hosted_derivative_warning(&runtime, &key).await;
            return;
        };
        let cache_root = self.cache_root.clone();
        let catalog_path = self.catalog_path.clone();
        let protected = self.protected_groups.clone();
        let asset_id = key.asset_id;
        let class = key.class;
        let generation_spec = spec.clone();
        let generation_cache_root = cache_root.clone();
        #[cfg(debug_assertions)]
        let encode_test_gate = self.hosted_encode_test_gates.lock().await.pop_front();
        #[cfg(debug_assertions)]
        let encode_registration_gate = self
            .hosted_encode_registration_test_gate
            .lock()
            .await
            .take();
        #[cfg(debug_assertions)]
        if let Some(gate) = encode_registration_gate.as_ref() {
            gate.entered.notify_one();
            while !gate.release.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
        let Some(encode_guard) = attempt_lease.begin_encode() else {
            #[cfg(debug_assertions)]
            if let Some(gate) = encode_registration_gate.as_ref() {
                gate.rejected.notify_one();
            }
            runtime.coordinator.fail_commit(permit).await;
            return;
        };
        #[cfg(debug_assertions)]
        if let Some(gate) = encode_registration_gate.as_ref() {
            gate.registered.notify_one();
        }
        let prepared = tokio::task::spawn_blocking(move || {
            let _encode_guard = encode_guard;
            let generator = ImageDerivativeGenerator::new(&generation_cache_root)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
            #[cfg(debug_assertions)]
            if let Some(gate) = encode_test_gate {
                gate.entered.notify_one();
                while !gate.release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
            let prepared = if class == DerivativeClass::ScreenPreview {
                generator
                    .encode_screen_preview(&source, &generation_spec)
                    .map(HostedDerivativePrepared::Screen)
                    .map_err(|_| AppServiceError::DerivativeFailed)
            } else {
                let encoded = generator
                    .encode_wall_thumbnail(&source, &generation_spec)
                    .map_err(|_| AppServiceError::DerivativeFailed)?;
                Ok(HostedDerivativePrepared::Wall(encoded))
            }?;
            Ok::<_, AppServiceError>((prepared, admission))
        })
        .await;
        let (prepared, _admission) = match prepared {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(_)) | Err(_) => {
                self.persist_hosted_derivative_warning(&key);
                runtime.coordinator.fail_commit_with_error(permit).await;
                self.emit_hosted_derivative_warning(&runtime, &key).await;
                return;
            }
        };
        if permit.was_cancelled() {
            runtime.coordinator.fail_commit_with_error(permit).await;
            return;
        }
        if self.pending_hosted_derivative(&key).await.is_none() {
            let warn = self.hosted_work_remains_in_scope(&key);
            if warn {
                self.persist_hosted_derivative_warning(&key);
            }
            runtime.coordinator.fail_commit(permit).await;
            if warn {
                self.emit_hosted_derivative_warning(&runtime, &key).await;
            }
            return;
        }
        #[cfg(debug_assertions)]
        self.wait_hosted_commit_publication_test_gate().await;
        // The test gate also represents the last externally observable fence:
        // re-read after it so a deterministic membership/signature mutation
        // rejects the staged bytes before either cache or catalog publication.
        // Acquire only after this cancellable staging boundary. Once held,
        // owner cleanup lets an authorized publisher retain terminal
        // settlement ownership rather than dropping its captured waiters.
        let _publication = self.hosted_publication_fence.lock().await;
        if !attempt_lease.begin_publication() {
            runtime.coordinator.fail_commit(permit).await;
            return;
        }
        if permit.was_cancelled() {
            runtime.coordinator.fail_commit_with_error(permit).await;
            return;
        }
        if self.pending_hosted_derivative(&key).await.is_none() {
            let warn = self.hosted_work_remains_in_scope(&key);
            if warn {
                self.persist_hosted_derivative_warning(&key);
            }
            runtime.coordinator.fail_commit(permit).await;
            if warn {
                self.emit_hosted_derivative_warning(&runtime, &key).await;
            }
            return;
        }
        let authorizer = self.hosted_scope_authorizer();
        #[cfg(debug_assertions)]
        if runtime
            .coordinator
            .authorized_waiter_scopes(ticket, &authorizer)
            .await
            .is_empty()
            && let Some((entered, release)) = self
                .hosted_empty_authorization_test_gate
                .lock()
                .await
                .take()
        {
            let notified = release.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            entered.notify_one();
            notified.await;
        }
        let Some(publication) = runtime
            .coordinator
            .authorize_hosted_publication(permit, authorizer)
            .await
        else {
            return;
        };
        #[cfg(debug_assertions)]
        self.wait_hosted_authorized_publication_test_gate().await;
        #[cfg(debug_assertions)]
        match self
            .hosted_post_authorization_fault
            .swap(0, Ordering::AcqRel)
        {
            1 => panic!("injected hosted derivative panic after publication authorization"),
            2 => {
                drop(publication);
                return;
            }
            _ => {}
        }
        // The publication fence makes this short catalog/cache transaction an
        // indivisible supervisor boundary. Keep it in the owned attempt task:
        // aborting the supervisor must never detach a blocking publisher that
        // can outlive the terminal failure and write stale bytes afterward.
        let result = (|| {
            let generator = ImageDerivativeGenerator::new(&cache_root)
                .map_err(|_| AppServiceError::DerivativeFailed)?;
            let mut catalog = Catalog::open(&catalog_path).map_err(AppServiceError::Catalog)?;
            match prepared {
                HostedDerivativePrepared::Screen(encoded) => generator
                    .commit_screen_preview_repairing(
                        encoded,
                        &spec,
                        group,
                        &mut catalog,
                        CacheBudget::automatic(&cache_root).map_err(AppServiceError::Cache)?,
                        &protected,
                    )
                    .map_err(|_| AppServiceError::DerivativeFailed),
                HostedDerivativePrepared::Wall(encoded) => {
                    let generated = generator
                        .commit_wall_thumbnail(encoded, &spec)
                        .map_err(|_| AppServiceError::DerivativeFailed)?;
                    catalog.upsert_derivative(&NewDerivative {
                        id: photo_domain::DerivativeId::new(),
                        asset_id,
                        folder_group_id: group,
                        kind: derivative_kind_name_for_gallery(class).to_owned(),
                        cache_key: generated.key.as_str().to_owned(),
                        relative_cache_path: generated.relative_path.clone(),
                        size_bytes: generated.size_bytes,
                        durable: true,
                        created_at: crate::service::unix_timestamp(),
                    })?;
                    Ok(generated)
                }
            }
        })();
        let result = result.ok();
        if let Some(generated) = result {
            if publication.was_cancelled() {
                let _ = self.rollback_hosted_publication(&generated, group);
                runtime
                    .coordinator
                    .finish_hosted_publication(
                        publication,
                        crate::derivative_coordinator::DerivativeResult::Unavailable,
                    )
                    .await;
                return;
            }
            #[cfg(debug_assertions)]
            self.wait_hosted_commit_post_publication_test_gate().await;
            // Keep a final defensive fence for mutations made through a
            // separate Catalog connection. If it trips, leave immutable
            // shared state intact and suppress this request's publication.
            if self.pending_hosted_derivative(&key).await.is_none() {
                let warn = self.hosted_work_remains_in_scope(&key);
                if warn {
                    self.persist_hosted_derivative_warning(&key);
                }
                let _ = self.rollback_hosted_publication(&generated, group);
                runtime
                    .coordinator
                    .finish_hosted_publication(
                        publication,
                        crate::derivative_coordinator::DerivativeResult::Unavailable,
                    )
                    .await;
                if warn {
                    self.emit_hosted_derivative_warning(&runtime, &key).await;
                }
                return;
            }
            let reference = DerivativeReference {
                asset_id: asset_id.as_uuid().hyphenated().to_string(),
                kind: class,
                key: generated.key.as_str().to_owned(),
            };
            self.clear_hosted_derivative_warning(&runtime, key.selection, asset_id)
                .await;
            runtime
                .publish(WallUpdate::DerivativesReady {
                    selection_id: runtime.selection.id.clone(),
                    derivatives: vec![reference.clone()],
                    preview_counts: None,
                })
                .await;
            runtime
                .coordinator
                .finish_hosted_publication(
                    publication,
                    crate::derivative_coordinator::DerivativeResult::Ready(reference),
                )
                .await;
        } else {
            self.persist_hosted_derivative_warning(&key);
            runtime
                .coordinator
                .finish_hosted_publication(
                    publication,
                    crate::derivative_coordinator::DerivativeResult::Failed,
                )
                .await;
            self.emit_hosted_derivative_warning(&runtime, &key).await;
        }
    }

    async fn pending_hosted_derivative(
        &self,
        key: &WorkKey,
    ) -> Option<(
        photo_catalog::AssetRecord,
        PathBuf,
        photo_cache::DerivativeSpec,
    )> {
        let (asset, source, spec, wall_record) = {
            let state = self.state.lock().ok()?;
            let asset = state.libraries.catalog().find_asset(key.asset_id).ok()??;
            if !asset.media_kind.is_wall_viewable()
                || asset.availability != photo_domain::Availability::Available
                || asset.availability != key.availability
            {
                return None;
            }
            let spec = derivative_spec_for_gallery(&asset, key.class);
            if DerivativeKey::compute(&spec).as_str() != key.cache_key {
                return None;
            }
            let wall_record = if key.class == DerivativeClass::ScreenPreview {
                let wall = derivative_spec_for_gallery(&asset, DerivativeClass::WallThumbnail);
                let wall_key = DerivativeKey::compute(&wall);
                Some(
                    state
                        .libraries
                        .catalog()
                        .find_derivative(asset.id, "wall_thumbnail", wall_key.as_str())
                        .ok()??,
                )
            } else {
                None
            };
            let library = state
                .libraries
                .catalog()
                .find_library(key.selection.library_id)
                .ok()??;
            let root = library.canonical_root_key.to_path_buf().ok()?;
            let relative = asset.relative_path.to_path_buf().ok()?;
            (asset, root.join(relative), spec, wall_record)
        };
        if let Some(wall_record) = wall_record
            && !self.derivative_record_is_valid_async(&wall_record).await
        {
            return None;
        }
        if !std::fs::symlink_metadata(&source).ok()?.is_file() {
            return None;
        }
        Some((asset, source, spec))
    }

    async fn derivative_record_is_valid_async(
        &self,
        record: &photo_catalog::DerivativeRecord,
    ) -> bool {
        let root = self.cache_root.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || derivative_record_is_valid(&root, &record))
            .await
            .unwrap_or(false)
    }

    /// Removes only this requester's link when a post-publication fence finds
    /// that the staged result is no longer authorized. Shared immutable bytes
    /// remain available to any other group link. When this is the final link,
    /// physical cleanup happens first and the bytes are restored if catalog
    /// link removal fails.
    fn rollback_hosted_publication(
        &self,
        generated: &photo_cache::GeneratedDerivative,
        group: FolderGroupId,
    ) -> Result<(), AppServiceError> {
        let mut catalog = Catalog::open(&self.catalog_path)?;
        let Some(record) = catalog.find_derivative_by_cache_key(generated.key.as_str())? else {
            return Ok(());
        };
        self.remove_requester_derivative_link(&mut catalog, &record, group)
    }

    fn remove_requester_derivative_link(
        &self,
        catalog: &mut Catalog,
        record: &photo_catalog::DerivativeRecord,
        group: FolderGroupId,
    ) -> Result<(), AppServiceError> {
        let removal = catalog.begin_derivative_group_link_removal(record.id, group)?;
        #[cfg(debug_assertions)]
        if self
            .hosted_link_removal_failure_test_hook
            .swap(false, Ordering::AcqRel)
        {
            return Err(AppServiceError::DerivativeFailed);
        }
        let remove_file = removal.removed() && removal.remaining_links() == 0;
        let old_bytes = if remove_file {
            match self
                .cache_writer
                .read_checked_limited(&record.relative_cache_path, MAX_MANAGED_DERIVATIVE_BYTES)
            {
                Ok(bytes) => Some(bytes),
                Err(photo_cache::CacheError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        if remove_file {
            self.cache_writer
                .remove_checked(&record.relative_cache_path)?;
        }
        if let Err(error) = removal.commit() {
            if let Some(old_bytes) = old_bytes
                && self
                    .cache_writer
                    .replace_atomic(record.relative_cache_path.clone(), |file| {
                        std::io::Write::write_all(file, &old_bytes)
                    })
                    .is_err()
            {
                return Err(AppServiceError::DerivativeFailed);
            }
            return Err(error.into());
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    async fn consume_hosted_panic_before_admission_test_hook(&self) -> bool {
        let mut hook = self.hosted_panic_before_admission.lock().await;
        let installed = *hook;
        *hook = false;
        installed
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_derivative_panic_before_admission_test_hook(&self) {
        *self.hosted_panic_before_admission.lock().await = true;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn install_hosted_derivative_panic_after_authorization_test_hook(&self) {
        self.hosted_post_authorization_fault
            .store(1, Ordering::Release);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn install_hosted_derivative_drop_after_authorization_test_hook(&self) {
        self.hosted_post_authorization_fault
            .store(2, Ordering::Release);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_encode_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<AtomicBool>,
    ) {
        self.hosted_encode_test_gates
            .lock()
            .await
            .push_back(HostedEncodeTestGate { entered, release });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_encode_registration_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<AtomicBool>,
        registered: Arc<Notify>,
        rejected: Arc<Notify>,
    ) {
        *self.hosted_encode_registration_test_gate.lock().await =
            Some(HostedEncodeRegistrationTestGate {
                entered,
                release,
                registered,
                rejected,
            });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn install_hosted_cancellation_transition_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<AtomicBool>,
        applied: Arc<Notify>,
        completed: Arc<Notify>,
    ) {
        *self
            .hosted_cancellation_transition_test_gate
            .lock()
            .expect("hosted cancellation transition test gate poisoned") =
            Some(HostedCancellationTransitionTestGate {
                entered,
                release,
                applied,
                completed,
            });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_empty_authorization_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_empty_authorization_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_pre_enqueue_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_pre_enqueue_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_post_validation_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_post_validation_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_scan_admission_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_scan_admission_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_scan_admission_test_gate(&self) {
        let gate = self.hosted_scan_admission_test_gate.lock().await.take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_root_outage_snapshot_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_root_outage_snapshot_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    async fn wait_hosted_root_outage_snapshot_test_gate(&self) {
        let gate = self
            .hosted_root_outage_snapshot_test_gate
            .lock()
            .await
            .take();
        let Some((entered, release)) = gate else {
            return;
        };
        let notified = release.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        entered.notify_one();
        notified.await;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn fail_next_hosted_cleanup_for_test(&self) {
        self.cache_writer.fail_next_cleanup_for_test();
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn fail_next_hosted_link_removal_for_test(&self) {
        self.hosted_link_removal_failure_test_hook
            .store(true, Ordering::Release);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_commit_post_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_commit_post_publication_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_hosted_authorized_publication_test_gate(
        &self,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    ) {
        *self.hosted_authorized_publication_test_gate.lock().await = Some((entered, release));
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn abort_hosted_derivative_driver_for_test(&self) {
        if let Some(handle) = self.hosted_driver_abort_handle.lock().await.take() {
            handle.abort();
        }
    }

    pub fn open_derivative(&self, opaque_id: &str) -> Result<ManagedDerivative, AppServiceError> {
        if opaque_id.is_empty() || opaque_id.len() > 512 || !opaque_id.is_ascii() {
            return Err(AppServiceError::UnknownAsset);
        }
        let record = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            state
                .libraries
                .catalog()
                .find_derivative_by_cache_key(opaque_id)?
                .ok_or(AppServiceError::UnknownAsset)?
        };
        if !matches!(record.kind.as_str(), "wall_thumbnail" | "screen_preview") {
            return Err(AppServiceError::UnknownAsset);
        }
        let writer = CacheWriter::new(&self.cache_root)?;
        let mut file = writer.open_checked(&record.relative_cache_path)?;
        if !validate_managed_jpeg(&mut file, record.size_bytes)
            .map_err(|error| AppServiceError::Cache(error.into()))?
        {
            return Err(AppServiceError::UnknownAsset);
        }
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0))
            .map_err(|error| AppServiceError::Cache(error.into()))?;
        let content_length = file
            .metadata()
            .map_err(|error| AppServiceError::Cache(error.into()))?
            .len();
        if content_length != record.size_bytes {
            return Err(AppServiceError::UnknownAsset);
        }
        Ok(ManagedDerivative {
            file,
            content_type: "image/jpeg",
            content_length,
            etag: format!("\"{}\"", record.cache_key),
        })
    }

    /// Performs catalog lookup and bounded cache validation on a blocking
    /// worker. The returned descriptor is ready for async chunk streaming.
    pub async fn open_derivative_async(
        &self,
        opaque_id: &str,
    ) -> Result<ManagedDerivative, AppServiceError> {
        let engine = self.clone();
        let opaque_id = opaque_id.to_owned();
        tokio::task::spawn_blocking(move || engine.open_derivative(&opaque_id))
            .await
            .map_err(|_| AppServiceError::DerivativeUnavailable)?
    }

    pub fn subscribe(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_with_origin(
            selection,
            client_id,
            scope,
            after_event_id,
            SubscriptionOrigin::Hosted,
        )
    }

    pub(crate) fn subscribe_desktop(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_with_origin(
            selection,
            client_id,
            scope,
            after_event_id,
            SubscriptionOrigin::Desktop,
        )
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn subscribe_desktop_for_test(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
    ) -> SelectionEventSubscription {
        self.subscribe_desktop(selection, client_id, scope, after_event_id)
    }

    fn subscribe_with_origin(
        &self,
        selection: &GallerySelection,
        client_id: String,
        scope: GalleryScope,
        after_event_id: Option<u64>,
        origin: SubscriptionOrigin,
    ) -> SelectionEventSubscription {
        let runtime = self.runtime(selection);
        let _publication = runtime
            .publication
            .lock()
            .expect("runtime publication poisoned");
        let receiver = runtime.updates.subscribe();
        let (client_token, scope_receiver) = runtime.register(client_id.clone(), scope, origin);
        let lifecycle = runtime.lifecycle_receiver();
        let history = runtime
            .history
            .lock()
            .expect("runtime history poisoned")
            .clone();
        let head = runtime.next_event_id.load(Ordering::Acquire);
        let oldest = history.front().map(|event| event.id);
        let (backlog, lagged, resync_after) = match after_event_id {
            None => (history, false, None),
            Some(after)
                if after > head || oldest.is_some_and(|first| after.saturating_add(1) < first) =>
            {
                (VecDeque::new(), true, Some(head))
            }
            Some(after) => (
                history
                    .into_iter()
                    .filter(|event| event.id > after)
                    .collect(),
                false,
                None,
            ),
        };
        drop(_publication);
        if origin == SubscriptionOrigin::Hosted {
            runtime.start_lease_reaper(self.clone());
        }
        SelectionEventSubscription {
            backlog,
            receiver,
            runtime,
            client_id,
            client_token,
            scope: scope_receiver,
            lifecycle,
            lagged,
            resync_after,
            resume_after: None,
            engine: self.clone(),
            terminal: None,
            terminal_event_id: None,
            last_seen_event_id: after_event_id
                .filter(|after| *after <= head)
                .unwrap_or(head),
        }
    }

    pub async fn update_client_interaction(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_with_origin(
            selection,
            client_id,
            scope,
            interaction,
            SubscriptionOrigin::Hosted,
        )
        .await
    }

    pub(crate) async fn update_client_interaction_desktop(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_with_origin(
            selection,
            client_id,
            scope,
            interaction,
            SubscriptionOrigin::Desktop,
        )
        .await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn update_client_interaction_desktop_for_test(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
    ) -> Result<bool, AppServiceError> {
        self.update_client_interaction_desktop(selection, client_id, scope, interaction)
            .await
    }

    async fn update_client_interaction_with_origin(
        &self,
        selection: &GallerySelection,
        client_id: &str,
        scope: GalleryScope,
        interaction: InteractionState,
        origin: SubscriptionOrigin,
    ) -> Result<bool, AppServiceError> {
        let runtime = self.runtime(selection);
        let found = {
            let mut demand = runtime
                .client_demand
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let token = demand
                .iter()
                .filter(|(_, value)| value.client_id == client_id && value.origin == origin)
                .map(|(token, _)| *token)
                .max();
            let Some(token) = token else {
                return Ok(false);
            };
            let value = demand.get_mut(&token).expect("demand token was present");
            value.scope = scope;
            let _ = value.scope_sender.send(scope);
            value.interaction = interaction;
            value.lease_until = if origin == SubscriptionOrigin::Hosted
                && interaction == InteractionState::Active
            {
                tokio::time::Instant::now() + std::time::Duration::from_secs(30)
            } else {
                tokio::time::Instant::now()
            };
            true
        };
        if !found {
            return Ok(false);
        }
        if origin == SubscriptionOrigin::Hosted {
            self.refresh_scheduler_interaction().await;
        }
        runtime.lease_wake.notify_waiters();
        Ok(true)
    }

    pub(crate) async fn refresh_scheduler_interaction(&self) {
        let active = self
            .folder_jobs
            .runtimes
            .lock()
            .ok()
            .map(|runtimes| {
                runtimes
                    .values()
                    .filter_map(Weak::upgrade)
                    .any(|runtime| runtime.has_active_lease())
            })
            .unwrap_or(false);
        self.scheduler
            .set_interaction_mode(if active {
                photo_indexer::InteractionMode::Active
            } else {
                photo_indexer::InteractionMode::Idle
            })
            .await;
    }

    pub(crate) async fn start_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
        admission: ScanAdmission,
    ) -> Result<(), AppServiceError> {
        if runtime.cancellation_requested() {
            runtime.finish_unowned_scan(admission, crate::hosted_runtime::ScanLifecycle::Cancelled);
            return Ok(());
        }
        if !self.ensure_runtime_source(&runtime).await? {
            return Ok(());
        }
        let (root, selected) = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::StatePoisoned)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let selected = runtime
                .selection
                .relative_folder
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            (root, selected)
        };
        // An unavailable source is a normal cached-browsing state. Check
        // lexical existence before canonicalization so a missing root can
        // still publish SourceUnavailable and leave cached wall rows readable.
        if !root.is_dir() || !root.join(&selected).is_dir() {
            self.mark_runtime_source_unavailable(&runtime).await?;
            return Ok(());
        }
        if runtime.cancellation_requested() {
            runtime.finish_unowned_scan(admission, crate::hosted_runtime::ScanLifecycle::Cancelled);
            return Ok(());
        }
        let indexer = Indexer::with_scheduler(
            self.metadata_reader.clone(),
            photo_core::FolderPolicyEngine::new(Vec::new())
                .map_err(|e| AppServiceError::LibrarySetup(std::io::Error::other(e.to_string())))?,
            self.scheduler.clone(),
        );
        #[cfg(debug_assertions)]
        self.wait_hosted_scan_admission_test_gate().await;
        let _publication = self.hosted_publication_fence.lock().await;
        if runtime.cancellation_requested() {
            runtime.finish_unowned_scan(admission, crate::hosted_runtime::ScanLifecycle::Cancelled);
            return Ok(());
        }
        let library_available = self.hosted_library_id != Some(runtime.selection.library_id) || {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::StatePoisoned)?
                .availability
                == photo_domain::Availability::Available
        };
        if !library_available {
            runtime.request_source_unavailable();
            let should_publish = !runtime
                .source_unavailable_reported
                .swap(true, Ordering::AcqRel);
            drop(_publication);
            if should_publish {
                runtime
                    .publish(WallUpdate::SourceUnavailable {
                        selection_id: runtime.selection.id().to_owned(),
                        source_id: runtime
                            .selection
                            .library_id
                            .as_uuid()
                            .hyphenated()
                            .to_string(),
                    })
                    .await;
            }
            return Ok(());
        }
        let Some(owner) = runtime.claim_scan_worker(admission) else {
            runtime.finish_unowned_scan(admission, crate::hosted_runtime::ScanLifecycle::Cancelled);
            return Ok(());
        };
        let generation = match (|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let selection_root = root.join(&selected);
            let canonical_root = std::fs::canonicalize(&root).map_err(|_| {
                AppServiceError::OpenRecent(photo_core::AddLibraryError::NotDirectory(root.clone()))
            })?;
            let canonical_selection_root =
                std::fs::canonicalize(&selection_root).map_err(|_| {
                    AppServiceError::OpenRecent(photo_core::AddLibraryError::NotDirectory(
                        selection_root.clone(),
                    ))
                })?;
            if !canonical_selection_root.starts_with(&canonical_root) {
                return Err(AppServiceError::OpenRecent(
                    photo_core::AddLibraryError::InvalidSelection,
                ));
            }
            state
                .libraries
                .catalog_mut()
                .begin_generation_for_group_at_recovery(
                    runtime.selection.library_id,
                    runtime.selection.group_id,
                    admission.recovery_token,
                )
                .map_err(AppServiceError::from)
        })() {
            Ok(generation) => generation,
            Err(error) => {
                runtime.finish_owned_scan(owner, crate::hosted_runtime::ScanLifecycle::Failed);
                return Err(error);
            }
        };
        drop(_publication);
        let selection_root = root.join(&selected);
        if runtime.cancellation_requested() {
            runtime.finish_owned_scan(owner, crate::hosted_runtime::ScanLifecycle::Cancelled);
            return Ok(());
        }
        #[cfg(test)]
        if self.fail_next_start.swap(false, Ordering::AcqRel) {
            runtime.finish_owned_scan(owner, crate::hosted_runtime::ScanLifecycle::Failed);
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test startup failure".to_owned(),
            )));
        }
        let handle = match indexer.start(
            ScanRequest::new(selection_root.clone())
                .for_library(runtime.selection.library_id)
                .roots(root, selection_root)
                .for_folder_group(runtime.selection.group_id),
        ) {
            Ok(handle) => handle,
            Err(error) => {
                runtime.finish_owned_scan(owner, crate::hosted_runtime::ScanLifecycle::Failed);
                return Err(AppServiceError::LibrarySetup(std::io::Error::other(
                    error.to_string(),
                )));
            }
        };
        runtime.install_scan_sender(&owner, handle.cancellation_sender());
        let engine = self.clone();
        tokio::spawn(async move {
            engine
                .drain_runtime_scan(runtime, handle, generation, admission, owner)
                .await;
        });
        Ok(())
    }

    /// Checks source availability even when a completed catalog means no scan
    /// is needed. This is intentionally a cheap filesystem check so cached
    /// wall rows remain queryable while an offline source is reported once.
    pub(crate) async fn ensure_runtime_source(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Result<bool, AppServiceError> {
        #[cfg(test)]
        if self.fail_next_source_check.swap(false, Ordering::AcqRel) {
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test source check failure".to_owned(),
            )));
        }
        let available = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let selected = runtime
                .selection
                .relative_folder
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            root.is_dir() && root.join(selected).is_dir()
        };
        if available {
            runtime
                .source_unavailable_reported
                .store(false, Ordering::Release);
            return Ok(true);
        }
        self.mark_runtime_source_unavailable(runtime).await?;
        Ok(false)
    }

    async fn mark_runtime_source_unavailable(
        &self,
        runtime: &Arc<SelectionRuntime>,
    ) -> Result<(), AppServiceError> {
        let root = {
            let state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            state
                .libraries
                .catalog()
                .find_library(runtime.selection.library_id)?
                .ok_or(AppServiceError::UnknownAsset)?
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?
        };
        if !root.is_dir()
            && self
                .mark_hosted_library_root_unavailable(runtime.selection.library_id)
                .await?
        {
            return Ok(());
        }
        runtime.request_source_unavailable();
        let should_publish = {
            let _publication = self.hosted_publication_fence.lock().await;
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            if root.is_dir() {
                state
                    .libraries
                    .catalog_mut()
                    .mark_group_offline(runtime.selection.library_id, runtime.selection.group_id)?;
            } else {
                state
                    .libraries
                    .catalog_mut()
                    .mark_root_offline(runtime.selection.library_id)?;
            }
            !runtime
                .source_unavailable_reported
                .swap(true, Ordering::AcqRel)
        };
        if should_publish {
            runtime
                .publish(WallUpdate::SourceUnavailable {
                    selection_id: runtime.selection.id().to_owned(),
                    source_id: runtime
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                })
                .await;
        }
        self.remove_runtime_if_dead(runtime.selection.group_id, runtime);
        Ok(())
    }

    /// Persists an authoritative hosted root outage behind the same fence as
    /// scan batches. Every active selection for the library is cancelled
    /// before the durable transition, so no scope can publish an old batch
    /// after the root is recorded offline.
    pub async fn mark_hosted_library_root_unavailable(
        &self,
        library_id: LibraryId,
    ) -> Result<bool, AppServiceError> {
        if self.hosted_library_id != Some(library_id) {
            return Ok(false);
        }
        let runtimes = {
            let _publication = self.hosted_publication_fence.lock().await;
            let runtimes = {
                let runtimes = self
                    .folder_jobs
                    .runtimes
                    .lock()
                    .map_err(|_| AppServiceError::StatePoisoned)?;
                runtimes
                    .values()
                    .filter_map(Weak::upgrade)
                    .filter(|runtime| runtime.selection.library_id == library_id)
                    .collect::<Vec<_>>()
            };
            #[cfg(debug_assertions)]
            self.wait_hosted_root_outage_snapshot_test_gate().await;
            for runtime in &runtimes {
                runtime.request_source_unavailable();
            }
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            state
                .libraries
                .catalog_mut()
                .mark_root_offline(library_id)?;
            runtimes
        };
        for runtime in runtimes {
            if !runtime
                .source_unavailable_reported
                .swap(true, Ordering::AcqRel)
            {
                runtime
                    .publish(WallUpdate::SourceUnavailable {
                        selection_id: runtime.selection.id().to_owned(),
                        source_id: library_id.as_uuid().hyphenated().to_string(),
                    })
                    .await;
            }
            self.remove_runtime_if_dead(runtime.selection.group_id, &runtime);
        }
        Ok(true)
    }

    #[doc(hidden)]
    pub fn runtime_count_for_test(&self) -> usize {
        self.folder_jobs
            .runtimes
            .lock()
            .expect("runtime registry poisoned")
            .values()
            .filter(|runtime| runtime.upgrade().is_some())
            .count()
    }

    pub(crate) fn current_event_id(&self, selection: &GallerySelection) -> u64 {
        self.folder_jobs
            .runtimes
            .lock()
            .ok()
            .and_then(|runtimes| {
                runtimes
                    .get(&(selection.library_id, selection.group_id))
                    .and_then(Weak::upgrade)
            })
            .map_or(0, |runtime| runtime.next_event_id.load(Ordering::Acquire))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn current_event_id_for_test(&self, selection: &GallerySelection) -> u64 {
        self.current_event_id(selection)
    }

    pub(crate) fn runtime_scan_state(
        &self,
        selection: &GallerySelection,
    ) -> Option<tokio::sync::watch::Receiver<crate::hosted_runtime::ScanLifecycle>> {
        self.folder_jobs
            .runtimes
            .lock()
            .ok()
            .and_then(|runtimes| {
                runtimes
                    .get(&(selection.library_id, selection.group_id))
                    .and_then(Weak::upgrade)
            })
            .map(|runtime| runtime.lifecycle_receiver())
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn publish_update_for_test(
        &self,
        selection: &GallerySelection,
        update: WallUpdate,
    ) -> crate::SequencedWallUpdate {
        self.runtime(selection).publish(update).await
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn set_interaction_for_test(&self, state: InteractionState) {
        self.scheduler
            .set_interaction_mode(match state {
                InteractionState::Idle => photo_indexer::InteractionMode::Idle,
                InteractionState::Active => photo_indexer::InteractionMode::Active,
            })
            .await;
    }

    #[doc(hidden)]
    pub async fn aggregate_scope_for_test(&self, selection: &GallerySelection) -> GalleryScope {
        self.runtime(selection).aggregate_scope()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn scheduler_permits_for_test(&self) -> usize {
        self.scheduler.available_background_permits()
    }

    #[cfg(test)]
    pub(crate) fn fail_next_start_for_test(&self) {
        self.fail_next_start.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_source_check_for_test(&self) {
        self.fail_next_source_check.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_batch_for_test(&self) {
        self.fail_next_batch.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_join_for_test(&self) {
        self.fail_next_join.store(true, Ordering::Release);
    }

    async fn drain_runtime_scan(
        &self,
        runtime: Arc<SelectionRuntime>,
        mut handle: photo_indexer::ScanHandle,
        generation: u64,
        admission: ScanAdmission,
        owner: ScanWorkerOwner,
    ) {
        let mut batch = Vec::new();
        let mut persistence_failed = false;
        let mut stream_open = true;
        while stream_open {
            let Some(first) = handle.events.recv().await else {
                break;
            };
            batch.push(first);
            let deadline = tokio::time::sleep(std::time::Duration::from_millis(50));
            tokio::pin!(deadline);
            while batch.len() < 200 {
                tokio::select! {
                    _ = &mut deadline => break,
                    event = handle.events.recv() => match event {
                        Some(event) => batch.push(event),
                        None => { stream_open = false; break; }
                    }
                }
            }
            if runtime.cancellation_requested() {
                break;
            }
            if self
                .apply_runtime_batch(&runtime, &owner, generation, &batch)
                .await
                .is_err()
            {
                persistence_failed = true;
                let _ = handle.cancel();
                break;
            }
            batch.clear();
        }
        let successful = handle
            .join()
            .await
            .map(|summary| !summary.cancelled)
            .unwrap_or(false);
        #[cfg(test)]
        let successful = if self.fail_next_join.swap(false, Ordering::AcqRel) {
            false
        } else {
            successful
        };
        let mut completed = false;
        if successful && !persistence_failed && !runtime.cancellation_requested() {
            let _publication = self.hosted_publication_fence.lock().await;
            completed = if runtime.worker_may_publish(&owner)
                && let Ok(mut state) = self.state.lock()
            {
                state
                    .libraries
                    .catalog_mut()
                    .complete_generation_for_group_at_recovery(
                        runtime.selection.library_id,
                        runtime.selection.group_id,
                        generation,
                        admission.recovery_token,
                    )
                    .is_ok()
            } else {
                false
            };
            if completed && runtime.worker_may_publish(&owner) {
                runtime
                    .settled
                    .store(true, std::sync::atomic::Ordering::Release);
                let _ = runtime
                    .publish(WallUpdate::MetadataSettled {
                        selection_id: runtime.selection.id().to_owned(),
                        source_id: runtime
                            .selection
                            .library_id
                            .as_uuid()
                            .hyphenated()
                            .to_string(),
                        generation,
                    })
                    .await;
            } else {
                persistence_failed = true;
            }
        }
        let terminal = if runtime.cancellation_requested() {
            crate::hosted_runtime::ScanLifecycle::Cancelled
        } else if persistence_failed {
            crate::hosted_runtime::ScanLifecycle::Failed
        } else if completed {
            crate::hosted_runtime::ScanLifecycle::Completed
        } else {
            crate::hosted_runtime::ScanLifecycle::Failed
        };
        runtime.finish_owned_scan(owner, terminal);
        self.remove_runtime_if_dead(runtime.selection.group_id, &runtime);
    }

    async fn apply_runtime_batch(
        &self,
        runtime: &Arc<SelectionRuntime>,
        owner: &ScanWorkerOwner,
        generation: u64,
        events: &[IndexEvent],
    ) -> Result<(), AppServiceError> {
        #[cfg(test)]
        if self.fail_next_batch.swap(false, Ordering::AcqRel) {
            return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                "test persistence failure".to_owned(),
            )));
        }
        if runtime.cancellation_requested() {
            return Ok(());
        }
        let progress = events
            .iter()
            .filter_map(|e| {
                if let IndexEvent::Progress(progress) = e {
                    Some(*progress)
                } else {
                    None
                }
            })
            .next_back();
        if let Some(progress) = progress {
            runtime.remember_inventory_totals(progress.direct_total, progress.total);
        }
        let shaped = events
            .iter()
            .filter_map(|e| match e {
                IndexEvent::ShapeReady { asset_id, .. }
                | IndexEvent::ShapeFallback { asset_id, .. } => Some(*asset_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let _publication = self.hosted_publication_fence.lock().await;
        if !runtime.worker_may_publish(owner) {
            return Ok(());
        }
        let updates = {
            let mut updates = Vec::new();
            let mut state = self
                .state
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let mut writer = photo_indexer::CatalogWriter::new(
                state.libraries.catalog_mut(),
                runtime.selection.library_id,
                generation,
            );
            if let Err(error) = writer.apply_batch(events) {
                return Err(AppServiceError::Catalog(CatalogError::InvalidData(
                    format!("hosted scan batch could not be persisted: {error}"),
                )));
            }
            let catalog = state.libraries.catalog();
            let mut published_progress =
                progress.map(crate::scan::progress_dto).unwrap_or_default();
            published_progress.direct_indexed_count = Some(catalog.wall_photo_count_scoped(
                runtime.selection.group_id,
                GalleryScope::CurrentFolder,
            )?);
            published_progress.indexed_count = Some(catalog.wall_photo_count_scoped(
                runtime.selection.group_id,
                GalleryScope::IncludeSubfolders,
            )?);
            published_progress.direct_total = published_progress
                .direct_total
                .or_else(|| runtime.inventory_total(GalleryScope::CurrentFolder));
            published_progress.total = published_progress
                .total
                .or_else(|| runtime.inventory_total(GalleryScope::IncludeSubfolders));
            let progress_update = progress.map(|_| WallUpdate::Progress {
                selection_id: runtime.selection.id().to_owned(),
                generation,
                progress: published_progress,
            });
            if let Some(update) = progress_update {
                updates.push(update);
            }
            for event in events {
                if let IndexEvent::Warning {
                    asset_id: Some(asset_id),
                    code,
                    ..
                } = event
                    && matches!(*code, "source_missing" | "source_unreadable")
                {
                    updates.push(WallUpdate::Warning {
                        selection_id: runtime.selection.id().to_owned(),
                        source_id: runtime.selection.library_id.as_uuid().to_string(),
                        asset_id: Some(asset_id.as_uuid().to_string()),
                        warning: crate::WallWarningState {
                            code: (*code).to_owned(),
                            retryable: true,
                        },
                    });
                }
            }
            if let Ok(records) = state.libraries.catalog().wall_records_for_assets_scoped(
                runtime.selection.group_id,
                GalleryScope::IncludeSubfolders,
                &shaped,
            ) && let Ok(assets) = crate::service::wall_assets_with_derivatives(
                state.libraries.catalog(),
                &records,
                OrderState::Provisional,
            ) && !assets.is_empty()
            {
                updates.push(WallUpdate::CatalogBatch {
                    selection_id: runtime.selection.id().to_owned(),
                    assets,
                    order_state: OrderState::Provisional,
                    generation,
                    progress: published_progress,
                });
            }
            updates
        };
        for update in updates {
            if !runtime.worker_may_publish(owner) {
                break;
            }
            let _ = runtime.publish(update).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod managed_derivative_validation_tests {
    use std::io::Write;

    use super::validate_managed_jpeg;

    #[test]
    fn oversized_sparse_derivative_is_rejected_without_reading_the_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut handle = file.reopen().unwrap();
        handle.write_all(&[0xff, 0xd8, 0xff]).unwrap();
        handle.set_len(128 * 1024 * 1024).unwrap();
        assert!(!validate_managed_jpeg(&mut handle, 128 * 1024 * 1024).unwrap());
    }

    #[test]
    fn same_prefix_garbage_is_rejected_by_bounded_jpeg_framing() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&[0xff, 0xd8, 0xff, 0x00, 0x00, 0xff, 0xd9])
            .unwrap();
        file.as_file_mut().flush().unwrap();
        assert!(!validate_managed_jpeg(file.as_file_mut(), 7).unwrap());
    }
}

#[cfg(all(test, feature = "server-internal-prevalidated-source"))]
mod prevalidated_hosted_source_tests {
    use std::sync::Arc;

    use super::PrevalidatedHostedSource;

    #[test]
    fn arbitrary_regular_file_cannot_fabricate_a_prevalidated_hosted_source() {
        let temp = tempfile::tempdir().unwrap();
        let regular_file_path = temp.path().join("not-a-directory");
        std::fs::write(&regular_file_path, b"source sentinel").unwrap();
        let regular_file = std::fs::File::open(&regular_file_path).unwrap();
        // SAFETY: deliberately violates the constructor contract to prove the
        // runtime descriptor-type check still rejects the misuse.
        let result = unsafe {
            PrevalidatedHostedSource::from_server_validated_directory(
                regular_file,
                Arc::new(()),
                regular_file_path.clone(),
                regular_file_path,
            )
        };

        let error = match result {
            Ok(_) => panic!("regular-file descriptor was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            !error
                .to_string()
                .contains(temp.path().to_string_lossy().as_ref())
        );
    }

    #[cfg(unix)]
    #[test]
    fn prevalidated_hosted_source_requires_normalized_absolute_keys() {
        let temp = tempfile::tempdir().unwrap();
        let directory = std::fs::File::open(temp.path()).unwrap();
        let non_normalized = temp.path().join("unused").join("..");

        // SAFETY: deliberately violates the constructor contract to exercise
        // its runtime key validation.
        let result = unsafe {
            PrevalidatedHostedSource::from_server_validated_directory(
                directory,
                Arc::new(()),
                non_normalized,
                temp.path().to_owned(),
            )
        };

        let error = match result {
            Ok(_) => panic!("non-normalized operational key was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "prevalidated source key is invalid");
    }
}
