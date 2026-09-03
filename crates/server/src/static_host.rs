use std::future::Future;
use std::io;
#[cfg(target_os = "linux")]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::path::Component;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};
use tower::ServiceExt;
use tower_http::services::ServeDir;
use tower_http::services::fs::{Backend, File as TowerFile};
use tower_http::set_header::SetResponseHeaderLayer;

const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const HTML_CACHE: &str = "no-cache";
const MAX_STATIC_RAW_PATH_BYTES: usize = 4096;
const MAX_STATIC_DECODED_PATH_BYTES: usize = 4096;
const MAX_STATIC_COMPONENTS: usize = 64;
const MAX_STATIC_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Debug)]
pub struct StaticWebRoot {
    path: Arc<PathBuf>,
    backend: SecureBackend,
}

impl StaticWebRoot {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        StaticWebRootValidation::capture(path)?.pin()
    }

    /// Returns an informational path for diagnostics. Static access and
    /// containment use the retained directory descriptor, never this path.
    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    /// Builds the fail-closed backend used on unsupported platforms without
    /// weakening the descriptor-backed production path on Linux or macOS.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn unavailable_for_test(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
            backend: SecureBackend::unavailable(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct StaticWebRootValidation {
    #[cfg(unix)]
    pinned: PinnedDirectory,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl StaticWebRootValidation {
    pub(crate) fn capture(path: PathBuf) -> io::Result<Self> {
        Self::capture_with_identity_hook_inner(path, || {})
    }

    #[cfg(test)]
    fn capture_with_identity_hook<F>(path: PathBuf, after_identity_capture: F) -> io::Result<Self>
    where
        F: FnOnce(),
    {
        Self::capture_with_identity_hook_inner(path, after_identity_capture)
    }

    fn capture_with_identity_hook_inner<F>(
        path: PathBuf,
        after_identity_capture: F,
    ) -> io::Result<Self>
    where
        F: FnOnce(),
    {
        #[cfg(unix)]
        {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let pinned = PinnedDirectory::open_static_with_identity_hook(
                &path,
                MountIdentityProvider::system(),
                after_identity_capture,
            )?;
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            let pinned = PinnedDirectory::open_with_identity_hook(&path, after_identity_capture)?;
            Ok(Self { pinned })
        }
        #[cfg(not(unix))]
        {
            let path = path.canonicalize()?;
            after_identity_capture();
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "static root is not a directory",
                ));
            }
            Ok(Self { path })
        }
    }

    pub(crate) fn path(&self) -> &Path {
        #[cfg(unix)]
        {
            self.pinned.display_path()
        }
        #[cfg(not(unix))]
        {
            &self.path
        }
    }

    #[cfg(unix)]
    pub(crate) fn pinned_directory(&self) -> &PinnedDirectory {
        &self.pinned
    }

    pub(crate) fn pin(self) -> io::Result<StaticWebRoot> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let path = Arc::new(self.pinned.display_path().to_owned());
            Ok(StaticWebRoot {
                path,
                backend: SecureBackend::from_root(self.pinned),
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            #[cfg(unix)]
            let path = self.pinned.display_path().to_owned();
            #[cfg(not(unix))]
            let path = self.path;
            Ok(StaticWebRoot {
                path: Arc::new(path),
                backend: SecureBackend::unavailable(),
            })
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum MountIdentity {
    #[cfg(target_os = "linux")]
    Linux(u64),
    #[cfg(target_os = "macos")]
    MacOs {
        fsid: Vec<u8>,
        filesystem_type: Vec<u8>,
        mounted_on: Vec<u8>,
        mounted_from: Vec<u8>,
    },
    #[cfg(test)]
    Synthetic(u64),
}

#[cfg(unix)]
impl MountIdentity {
    #[cfg(test)]
    const fn synthetic(value: u64) -> Self {
        Self::Synthetic(value)
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct MountIdentityProvider(Arc<MountIdentityResolver>);

#[cfg(unix)]
type MountIdentityResolver = dyn Fn(&std::fs::File) -> io::Result<MountIdentity> + Send + Sync;

#[cfg(unix)]
impl std::fmt::Debug for MountIdentityProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MountIdentityProvider")
    }
}

#[cfg(unix)]
impl MountIdentityProvider {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn system() -> Self {
        Self(Arc::new(system_mount_identity))
    }

    #[cfg(test)]
    fn for_test<F>(provider: F) -> Self
    where
        F: Fn(&std::fs::File) -> io::Result<MountIdentity> + Send + Sync + 'static,
    {
        Self(Arc::new(provider))
    }

    fn identify(&self, file: &std::fs::File) -> io::Result<MountIdentity> {
        (self.0)(file)
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct MountPolicy {
    identity: MountIdentity,
    provider: MountIdentityProvider,
}

#[cfg(unix)]
impl MountPolicy {
    fn capture(file: &std::fs::File, provider: MountIdentityProvider) -> io::Result<Self> {
        let identity = provider
            .identify(file)
            .map_err(|_| mount_transition_error())?;
        Ok(Self { identity, provider })
    }

    fn validate(&self, file: &std::fs::File) -> io::Result<()> {
        let actual = self
            .provider
            .identify(file)
            .map_err(|_| mount_transition_error())?;
        validate_mount_chain(&self.identity, [&actual])
    }

    fn identify(&self, file: &std::fs::File) -> io::Result<MountIdentity> {
        self.provider
            .identify(file)
            .map_err(|_| mount_transition_error())
    }
}

#[cfg(unix)]
fn validate_mount_chain<'a>(
    expected: &MountIdentity,
    chain: impl IntoIterator<Item = &'a MountIdentity>,
) -> io::Result<()> {
    if chain.into_iter().all(|actual| actual == expected) {
        Ok(())
    } else {
        Err(mount_transition_error())
    }
}

#[cfg(unix)]
fn mount_transition_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "static mount transition is not permitted",
    )
}

#[cfg(unix)]
impl DirectoryIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct PinnedDirectory {
    directory: std::fs::File,
    identity: DirectoryIdentity,
    display_path: PathBuf,
    mount_policy: Option<MountPolicy>,
}

#[cfg(unix)]
impl PinnedDirectory {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Self::open_with_identity_hook(path, || {})
    }

    fn open_with_identity_hook<F>(path: &Path, after_identity_capture: F) -> io::Result<Self>
    where
        F: FnOnce(),
    {
        use std::os::unix::fs::OpenOptionsExt;

        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = directory.metadata()?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "configured path is not a directory",
            ));
        }
        let identity = DirectoryIdentity::from_metadata(&metadata);
        after_identity_capture();
        let display_path = informational_display_path(path, identity)?;
        Ok(Self {
            directory,
            identity,
            display_path,
            mount_policy: None,
        })
    }

    #[cfg(test)]
    fn open_with_mount_provider(path: &Path, provider: MountIdentityProvider) -> io::Result<Self> {
        Self::open_static_with_identity_hook(path, provider, || {})
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    fn open_static_with_identity_hook<F>(
        path: &Path,
        provider: MountIdentityProvider,
        after_identity_capture: F,
    ) -> io::Result<Self>
    where
        F: FnOnce(),
    {
        let mut pinned = Self::open_with_identity_hook(path, after_identity_capture)?;
        pinned.mount_policy = Some(MountPolicy::capture(&pinned.directory, provider)?);
        pinned.validate_static_mount_ancestry()?;
        Ok(pinned)
    }

    /// This path is for diagnostics and path-based defense in depth only.
    /// Security decisions use the retained descriptor and its ancestry.
    pub(crate) fn display_path(&self) -> &Path {
        &self.display_path
    }

    pub(crate) fn same_object_as(&self, other: &Self) -> bool {
        self.identity == other.identity
    }

    pub(crate) fn into_file(self) -> std::fs::File {
        self.directory
    }

    pub(crate) fn overlaps(&self, other: &Self) -> io::Result<bool> {
        if self.identity == other.identity {
            return Ok(true);
        }
        let self_ancestry = self.ancestry()?;
        if self_ancestry.contains(&other.identity) {
            return Ok(true);
        }
        let other_ancestry = other.ancestry()?;
        Ok(other_ancestry.contains(&self.identity))
    }

    pub(crate) fn identity_is_in_ancestry_of(&self, other: &Self) -> io::Result<bool> {
        if self.identity == other.identity {
            return Ok(true);
        }
        Ok(other.ancestry()?.contains(&self.identity))
    }

    fn ancestry(&self) -> io::Result<Vec<DirectoryIdentity>> {
        let mut identities = vec![self.identity];
        let mut current = self.directory.try_clone()?;
        loop {
            let parent = openat(
                current.as_raw_fd(),
                std::ffi::OsStr::new(".."),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )?;
            let parent_identity = DirectoryIdentity::from_metadata(&parent.metadata()?);
            let current_identity = *identities.last().expect("ancestry contains the root");
            if parent_identity == current_identity {
                return Ok(identities);
            }
            if identities.contains(&parent_identity) || identities.len() >= 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory ancestry did not reach a stable root",
                ));
            }
            identities.push(parent_identity);
            current = parent;
        }
    }

    fn validate_static_mount_ancestry(&self) -> io::Result<()> {
        let policy = self.mount_policy.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "static mount policy is unavailable",
            )
        })?;
        let mut identities = vec![self.identity];
        let mut current = self.directory.try_clone()?;
        let mut current_mount = policy.identify(&current)?;
        validate_mount_chain(&policy.identity, [&current_mount])?;
        loop {
            let parent = openat(
                current.as_raw_fd(),
                std::ffi::OsStr::new(".."),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )?;
            let parent_mount = policy.identify(&parent)?;
            if parent_mount != current_mount
                && !allowed_platform_ancestry_transition(&current_mount, &parent_mount)
            {
                return Err(mount_transition_error());
            }
            let parent_identity = DirectoryIdentity::from_metadata(&parent.metadata()?);
            let current_identity = *identities.last().expect("ancestry contains the root");
            if parent_identity == current_identity {
                return Ok(());
            }
            if identities.contains(&parent_identity) || identities.len() >= 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory ancestry did not reach a stable root",
                ));
            }
            identities.push(parent_identity);
            current = parent;
            current_mount = parent_mount;
        }
    }
}

#[cfg(target_os = "linux")]
fn allowed_platform_ancestry_transition(_child: &MountIdentity, _parent: &MountIdentity) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn allowed_platform_ancestry_transition(child: &MountIdentity, parent: &MountIdentity) -> bool {
    is_macos_apfs_data_to_system_firmlink(child, parent)
}

#[cfg(target_os = "macos")]
fn is_macos_apfs_data_to_system_firmlink(child: &MountIdentity, parent: &MountIdentity) -> bool {
    let (
        Some((child_type, child_mounted_on, child_mounted_from)),
        Some((parent_type, parent_mounted_on, parent_mounted_from)),
    ) = (macos_mount_fields(child), macos_mount_fields(parent))
    else {
        return false;
    };

    child_type == b"apfs"
        && parent_type == b"apfs"
        && child_mounted_on == b"/System/Volumes/Data"
        && parent_mounted_on == b"/"
        && apfs_physical_store(child_mounted_from).is_some_and(|child_store| {
            apfs_physical_store(parent_mounted_from)
                .is_some_and(|parent_store| child_store == parent_store)
        })
}

#[cfg(target_os = "macos")]
fn macos_mount_fields(identity: &MountIdentity) -> Option<(&[u8], &[u8], &[u8])> {
    #[cfg(not(test))]
    let MountIdentity::MacOs {
        filesystem_type,
        mounted_on,
        mounted_from,
        ..
    } = identity;
    #[cfg(test)]
    let MountIdentity::MacOs {
        filesystem_type,
        mounted_on,
        mounted_from,
        ..
    } = identity
    else {
        return None;
    };
    Some((filesystem_type, mounted_on, mounted_from))
}

#[cfg(target_os = "macos")]
fn apfs_physical_store(source: &[u8]) -> Option<&[u8]> {
    let suffix = source.strip_prefix(b"/dev/disk")?;
    let disk_digits = suffix
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if disk_digits == 0 || suffix.get(disk_digits) != Some(&b's') {
        return None;
    }
    let mut remainder = &suffix[disk_digits..];
    while !remainder.is_empty() {
        remainder = remainder.strip_prefix(b"s")?;
        let digits = remainder
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        remainder = &remainder[digits..];
    }
    Some(&suffix[..disk_digits])
}

#[cfg(target_os = "linux")]
fn system_mount_identity(file: &std::fs::File) -> io::Result<MountIdentity> {
    use std::mem::MaybeUninit;

    let mut stat = MaybeUninit::<libc::statx>::zeroed();
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_NO_AUTOMOUNT,
            libc::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    };
    if result == 0 {
        let stat = unsafe { stat.assume_init() };
        if stat.stx_mask & libc::STATX_MNT_ID != 0 {
            return Ok(MountIdentity::Linux(stat.stx_mnt_id));
        }
    }

    let proc_path = format!("/proc/self/fdinfo/{}", file.as_raw_fd());
    let mut fdinfo = std::fs::File::open(proc_path)?;
    let mut contents = String::new();
    fdinfo
        .by_ref()
        .take(16 * 1024)
        .read_to_string(&mut contents)?;
    if contents.len() == 16 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "descriptor mount metadata exceeded its fixed bound",
        ));
    }
    parse_fdinfo_mount_id(&contents).map(MountIdentity::Linux)
}

#[cfg(any(target_os = "linux", test))]
fn parse_fdinfo_mount_id(contents: &str) -> io::Result<u64> {
    let mut mount_id = None;
    for line in contents.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name != "mnt_id" {
            continue;
        }
        let value = value.trim();
        if mount_id.is_some()
            || value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid_fdinfo());
        }
        mount_id = Some(value.parse().map_err(|_| invalid_fdinfo())?);
    }
    mount_id.ok_or_else(invalid_fdinfo)
}

#[cfg(any(target_os = "linux", test))]
fn invalid_fdinfo() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor mount metadata is unavailable",
    )
}

#[cfg(target_os = "macos")]
fn system_mount_identity(file: &std::fs::File) -> io::Result<MountIdentity> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let fsid = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!(stat.f_fsid).cast::<u8>(),
            std::mem::size_of_val(&stat.f_fsid),
        )
        .to_vec()
    };
    Ok(MountIdentity::MacOs {
        fsid,
        filesystem_type: bounded_c_array(&stat.f_fstypename)?,
        mounted_on: bounded_c_array(&stat.f_mntonname)?,
        mounted_from: bounded_c_array(&stat.f_mntfromname)?,
    })
}

#[cfg(target_os = "macos")]
fn bounded_c_array<const N: usize>(value: &[libc::c_char; N]) -> io::Result<Vec<u8>> {
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), N) };
    let length = bytes.iter().position(|byte| *byte == 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "mount metadata is not terminated",
        )
    })?;
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mount metadata is incomplete",
        ));
    }
    Ok(bytes[..length].to_vec())
}

#[cfg(unix)]
fn informational_display_path(path: &Path, expected: DirectoryIdentity) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let Ok(canonical) = path.canonicalize() else {
        return Ok(absolute);
    };
    let Ok(metadata) = std::fs::metadata(&canonical) else {
        return Ok(absolute);
    };
    if DirectoryIdentity::from_metadata(&metadata) == expected {
        Ok(canonical)
    } else {
        Ok(absolute)
    }
}

#[derive(Clone)]
struct StaticHost {
    backend: SecureBackend,
}

pub(crate) fn router(web_root: StaticWebRoot) -> Router {
    Router::new()
        .fallback(any(serve))
        .with_state(StaticHost {
            backend: web_root.backend,
        })
        .layer(SetResponseHeaderLayer::overriding(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

async fn serve(State(host): State<StaticHost>, request: Request<Body>) -> Response<Body> {
    let normalized = match NormalizedPath::parse(request.uri().path()) {
        Ok(path) => path,
        Err(StaticPathError::Invalid) => return empty_response(StatusCode::BAD_REQUEST),
        Err(StaticPathError::TooLong) => return empty_response(StatusCode::URI_TOO_LONG),
    };
    if normalized.is_reserved_namespace() {
        return crate::api::route_not_found().await.into_response();
    }
    let method = request.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return empty_response(StatusCode::METHOD_NOT_ALLOWED);
    }
    let headers = request.headers().clone();
    let mut request = request;
    *request.uri_mut() = normalized.uri();
    let response = static_service(host.backend.clone())
        .oneshot(request)
        .await
        .expect("static file service is infallible");
    if response.status() != StatusCode::NOT_FOUND {
        return cache_static(response, normalized.is_asset(), false);
    }
    if !normalized.is_interface_route() {
        return response.map(Body::new);
    }

    let index_request = request_for_index(method, headers);
    let response = static_service(host.backend)
        .oneshot(index_request)
        .await
        .expect("index file service is infallible");
    cache_static(response, false, true)
}

fn static_service(
    backend: SecureBackend,
) -> ServeDir<tower_http::services::fs::DefaultServeDirFallback, SecureBackend> {
    ServeDir::with_backend(".", backend)
        .append_index_html_on_directories(false)
        .precompressed_br()
        .precompressed_gzip()
}

fn request_for_index(method: Method, headers: HeaderMap) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/index.html")
        .body(Body::empty())
        .expect("index request is valid");
    *request.headers_mut() = headers;
    request
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .expect("static status response is valid")
}

fn cache_static<B>(response: Response<B>, asset: bool, interface_fallback: bool) -> Response<Body>
where
    B: axum::body::HttpBody<Data = axum::body::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    let mut response = response.map(Body::new);
    let cache = if !interface_fallback && asset {
        HeaderValue::from_static(IMMUTABLE_CACHE)
    } else {
        HeaderValue::from_static(HTML_CACHE)
    };
    response.headers_mut().insert(CACHE_CONTROL, cache);
    response
}

#[derive(Clone, Debug)]
struct NormalizedPath {
    components: Vec<String>,
    uri: Uri,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StaticPathError {
    Invalid,
    TooLong,
}

impl NormalizedPath {
    fn parse(raw: &str) -> Result<Self, StaticPathError> {
        if raw.len() > MAX_STATIC_RAW_PATH_BYTES {
            return Err(StaticPathError::TooLong);
        }
        if !raw.starts_with('/') {
            return Err(StaticPathError::Invalid);
        }
        let raw = raw.as_bytes();
        let mut decoded = Vec::with_capacity(raw.len());
        let mut index = 0;
        while index < raw.len() {
            match raw[index] {
                b'%' if index + 2 < raw.len() => {
                    let high = hex(raw[index + 1]).ok_or(StaticPathError::Invalid)?;
                    let low = hex(raw[index + 2]).ok_or(StaticPathError::Invalid)?;
                    let byte = high << 4 | low;
                    if matches!(byte, b'.' | b'/' | b'\\' | 0) {
                        return Err(StaticPathError::Invalid);
                    }
                    decoded.push(byte);
                    index += 3;
                }
                b'%' => return Err(StaticPathError::Invalid),
                b'\\' | 0 => return Err(StaticPathError::Invalid),
                byte => {
                    decoded.push(byte);
                    index += 1;
                }
            }
            if decoded.len() > MAX_STATIC_DECODED_PATH_BYTES {
                return Err(StaticPathError::TooLong);
            }
        }
        let decoded = String::from_utf8(decoded).map_err(|_| StaticPathError::Invalid)?;
        let relative = decoded.strip_prefix('/').ok_or(StaticPathError::Invalid)?;
        let mut components = Vec::new();
        if !relative.is_empty() {
            let relative = relative.strip_suffix('/').unwrap_or(relative);
            if relative.is_empty() {
                return Err(StaticPathError::Invalid);
            }
            for component in relative.split('/') {
                if component.is_empty() || component == "." || component == ".." {
                    return Err(StaticPathError::Invalid);
                }
                if component.len() > MAX_STATIC_COMPONENT_BYTES {
                    return Err(StaticPathError::TooLong);
                }
                if components.len() == MAX_STATIC_COMPONENTS {
                    return Err(StaticPathError::TooLong);
                }
                components.push(component.to_owned());
            }
        }
        let canonical = canonical_uri_path(&components);
        let uri = Uri::from_str(&canonical).map_err(|_| StaticPathError::Invalid)?;
        Ok(Self { components, uri })
    }

    fn uri(&self) -> Uri {
        self.uri.clone()
    }

    fn is_asset(&self) -> bool {
        self.components.len() > 1
            && self
                .components
                .first()
                .is_some_and(|value| value == "assets")
    }

    fn is_reserved_namespace(&self) -> bool {
        self.components
            .first()
            .is_some_and(|value| value == "api" || value == "healthz")
    }

    fn is_interface_route(&self) -> bool {
        self.components
            .last()
            .is_none_or(|name| Path::new(name).extension().is_none())
    }
}

fn canonical_uri_path(components: &[String]) -> String {
    let mut path = String::from("/");
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            path.push('/');
        }
        for byte in component.as_bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                path.push(char::from(*byte));
            } else {
                path.push('%');
                path.push(hex_digit(byte >> 4));
                path.push(hex_digit(byte & 0x0f));
            }
        }
    }
    path
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_digit(value: u8) -> char {
    char::from(if value < 10 {
        b'0' + value
    } else {
        b'A' + value - 10
    })
}

#[derive(Clone, Debug)]
struct SecureBackend {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: Option<Arc<PinnedDirectory>>,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    root: Option<Arc<SecureRoot>>,
    #[cfg(test)]
    filesystem_calls: Arc<AtomicUsize>,
}

impl SecureBackend {
    #[cfg(test)]
    fn new(path: &Path) -> io::Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            Ok(Self::from_root(PinnedDirectory::open_with_mount_provider(
                path,
                MountIdentityProvider::system(),
            )?))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Ok(Self::from_root(SecureRoot::open(path)?))
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn from_root(root: PinnedDirectory) -> Self {
        Self {
            root: Some(Arc::new(root)),
            #[cfg(test)]
            filesystem_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(all(test, not(any(target_os = "linux", target_os = "macos"))))]
    fn from_root(root: SecureRoot) -> Self {
        Self {
            root: Some(Arc::new(root)),
            #[cfg(test)]
            filesystem_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(any(not(any(target_os = "linux", target_os = "macos")), debug_assertions))]
    fn unavailable() -> Self {
        Self {
            root: None,
            #[cfg(test)]
            filesystem_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    fn filesystem_calls(&self) -> Arc<AtomicUsize> {
        self.filesystem_calls.clone()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[derive(Debug)]
struct SecureRoot {}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl SecureRoot {
    #[cfg(test)]
    fn open(_path: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure descriptor-relative static serving is unavailable on this platform",
        ))
    }

    fn open_file(&self, _path: &Path) -> io::Result<std::fs::File> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "static serving is unavailable",
        ))
    }

    fn metadata(&self, _path: &Path) -> io::Result<std::fs::Metadata> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "static serving is unavailable",
        ))
    }
}

#[cfg(unix)]
impl PinnedDirectory {
    fn open_file(&self, path: &Path) -> io::Result<std::fs::File> {
        self.open_file_with_hook(path, || {})
    }

    fn open_file_with_hook<F>(&self, path: &Path, before_final_open: F) -> io::Result<std::fs::File>
    where
        F: FnOnce(),
    {
        self.open_relative_with_hook(path, true, before_final_open)
    }

    fn metadata(&self, path: &Path) -> io::Result<std::fs::Metadata> {
        self.open_relative_with_hook(path, false, || {})?.metadata()
    }

    fn open_relative_with_hook<F>(
        &self,
        path: &Path,
        require_regular_file: bool,
        before_final_open: F,
    ) -> io::Result<std::fs::File>
    where
        F: FnOnce(),
    {
        let components = secure_components(path)?;
        let mut directory = self.directory.try_clone()?;
        self.validate_open_mount(&directory)?;
        if components.is_empty() {
            if require_regular_file {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "not a regular file",
                ));
            }
            return Ok(directory);
        }
        for component in &components[..components.len() - 1] {
            directory = openat(
                directory.as_raw_fd(),
                component,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )?;
            self.validate_open_mount(&directory)?;
        }
        before_final_open();
        let file = openat(
            directory.as_raw_fd(),
            components.last().expect("components are non-empty"),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )?;
        self.validate_open_mount(&file)?;
        let metadata = file.metadata()?;
        let valid = if require_regular_file {
            metadata.is_file()
        } else {
            metadata.is_file() || metadata.is_dir()
        };
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unsupported file type",
            ));
        }
        Ok(file)
    }

    fn validate_open_mount(&self, file: &std::fs::File) -> io::Result<()> {
        if let Some(policy) = &self.mount_policy {
            policy.validate(file)
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
fn secure_components(path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(value) => Some(Ok(value.to_owned())),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => Some(Err(
                io::Error::new(io::ErrorKind::NotFound, "invalid static path"),
            )),
        })
        .collect()
}

#[cfg(unix)]
fn openat(
    directory: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
    flags: i32,
) -> io::Result<std::fs::File> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "invalid static path"))?;
    let descriptor = unsafe { libc::openat(directory, name.as_ptr(), flags) };
    if descriptor == -1 {
        return Err(safe_open_error(io::Error::last_os_error()));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn safe_open_error(error: io::Error) -> io::Error {
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP)
            | Some(libc::ENOTDIR)
            | Some(libc::EACCES)
            | Some(libc::EPERM)
            | Some(libc::ENAMETOOLONG)
    ) {
        io::Error::new(io::ErrorKind::NotFound, "static file is unavailable")
    } else {
        error
    }
}

#[derive(Debug)]
struct SecureFile(tokio::fs::File);

impl AsyncRead for SecureFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncSeek for SecureFile {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        Pin::new(&mut self.0).start_seek(position)
    }

    fn poll_complete(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<u64>> {
        Pin::new(&mut self.0).poll_complete(context)
    }
}

impl TowerFile for SecureFile {
    type Metadata = std::fs::Metadata;
    type MetadataFuture<'a> =
        Pin<Box<dyn Future<Output = io::Result<std::fs::Metadata>> + Send + 'a>>;

    fn metadata(&self) -> Self::MetadataFuture<'_> {
        Box::pin(async move { self.0.metadata().await })
    }
}

impl Backend for SecureBackend {
    type File = SecureFile;
    type Metadata = std::fs::Metadata;
    type OpenFuture = Pin<Box<dyn Future<Output = io::Result<SecureFile>> + Send>>;
    type MetadataFuture = Pin<Box<dyn Future<Output = io::Result<std::fs::Metadata>> + Send>>;

    fn open(&self, path: PathBuf) -> Self::OpenFuture {
        #[cfg(test)]
        self.filesystem_calls.fetch_add(1, Ordering::Relaxed);
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(static_unavailable)?;
            let file = tokio::task::spawn_blocking(move || root.open_file(&path))
                .await
                .map_err(io::Error::other)??;
            Ok(SecureFile(tokio::fs::File::from_std(file)))
        })
    }

    fn metadata(&self, path: PathBuf) -> Self::MetadataFuture {
        #[cfg(test)]
        self.filesystem_calls.fetch_add(1, Ordering::Relaxed);
        let root = self.root.clone();
        Box::pin(async move {
            let root = root.ok_or_else(static_unavailable)?;
            tokio::task::spawn_blocking(move || root.metadata(&path))
                .await
                .map_err(io::Error::other)?
        })
    }
}

fn static_unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "static serving is unavailable")
}

#[cfg(test)]
mod path_limit_tests {
    use std::sync::atomic::Ordering;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{Request, StatusCode};

    use super::{SecureBackend, StaticHost, serve};

    #[tokio::test]
    async fn every_static_path_bound_rejects_before_a_filesystem_call() {
        let temp = tempfile::tempdir().unwrap();
        let backend = SecureBackend::new(temp.path()).unwrap();
        let calls = backend.filesystem_calls();
        let oversized_total = format!("/{}", vec!["a".repeat(240); 18].join("/"));
        let too_many_components = format!("/{}", vec!["a"; 65].join("/"));
        let oversized_component = format!("/{}", "%61".repeat(256));

        for uri in [oversized_total, too_many_components, oversized_component] {
            let response = serve(
                State(StaticHost {
                    backend: backend.clone(),
                }),
                Request::get(uri).body(Body::empty()).unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::URI_TOO_LONG);
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        }
    }
}

#[cfg(all(test, not(unix)))]
mod unsupported_platform_tests {
    use super::SecureBackend;

    #[test]
    fn static_backend_fails_closed_without_descriptor_relative_open() {
        let temp = tempfile::tempdir().unwrap();
        assert!(SecureBackend::new(temp.path()).is_err());
    }
}

#[cfg(all(test, unix))]
mod secure_open_tests {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::symlink;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[cfg(target_os = "macos")]
    use super::is_macos_apfs_data_to_system_firmlink;
    use super::{
        DirectoryIdentity, MountIdentity, MountIdentityProvider, PinnedDirectory, SecureBackend,
        StaticWebRoot, StaticWebRootValidation, openat, parse_fdinfo_mount_id, router,
        safe_open_error, validate_mount_chain,
    };

    fn read(mut file: std::fs::File) -> Vec<u8> {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn ancestor_swap_stays_on_the_pinned_directory() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        let assets = web.join("assets");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(assets.join("app.js"), b"SAFE ASSET").unwrap();
        std::fs::write(outside.join("app.js"), b"SOURCE SENTINEL").unwrap();
        let root = PinnedDirectory::open(&web).unwrap();

        let file = root
            .open_file_with_hook(std::path::Path::new("assets/app.js"), || {
                std::fs::rename(&assets, web.join("assets-pinned")).unwrap();
                symlink(&outside, &assets).unwrap();
            })
            .unwrap();

        assert_eq!(read(file), b"SAFE ASSET");
    }

    #[test]
    fn identity_capture_cannot_be_redirected_after_path_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        let source = temp.path().join("photos");
        let moved_web = temp.path().join("validated-web-moved");
        std::fs::create_dir(&web).unwrap();
        std::fs::create_dir(&source).unwrap();
        std::fs::write(web.join("index.html"), b"SAFE WEB ROOT").unwrap();
        std::fs::write(source.join("index.html"), b"SOURCE ROOT SENTINEL").unwrap();

        let validation = StaticWebRootValidation::capture_with_identity_hook(web.clone(), || {
            std::fs::rename(&web, &moved_web).unwrap();
            std::fs::rename(&source, &web).unwrap();
        })
        .unwrap();
        let pinned = validation.pin().unwrap();
        let file = pinned
            .backend
            .root
            .as_ref()
            .unwrap()
            .open_file(std::path::Path::new("index.html"))
            .unwrap();

        assert_eq!(read(file), b"SAFE WEB ROOT");
        assert_eq!(
            std::fs::read(web.join("index.html")).unwrap(),
            b"SOURCE ROOT SENTINEL"
        );
    }

    #[test]
    fn pinned_root_survives_same_inode_rename_during_display_path_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        let moved_web = temp.path().join("renamed-web");
        std::fs::create_dir(&web).unwrap();
        std::fs::write(web.join("index.html"), b"SAFE WEB ROOT").unwrap();

        let validation = StaticWebRootValidation::capture_with_identity_hook(web.clone(), || {
            std::fs::rename(&web, &moved_web).unwrap();
        })
        .unwrap();
        let pinned = validation.pin().unwrap();
        let file = pinned
            .backend
            .root
            .as_ref()
            .unwrap()
            .open_file(std::path::Path::new("index.html"))
            .unwrap();

        assert_eq!(read(file), b"SAFE WEB ROOT");
    }

    #[test]
    fn held_descriptor_alias_has_the_same_security_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir(&source).unwrap();
        let pinned = PinnedDirectory::open(&source).unwrap();
        let aliased_directory = openat(
            pinned.directory.as_raw_fd(),
            std::ffi::OsStr::new("."),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
        .unwrap();
        let aliased = PinnedDirectory {
            directory: aliased_directory,
            identity: pinned.identity,
            display_path: temp.path().join("different-informational-label"),
            mount_policy: None,
        };

        assert_ne!(aliased.display_path(), pinned.display_path());
        assert_eq!(aliased.identity, pinned.identity);
        assert!(aliased.overlaps(&pinned).unwrap());
    }

    #[test]
    fn descriptor_ancestry_detects_parent_child_overlap_in_both_directions() {
        let temp = tempfile::tempdir().unwrap();
        let parent_path = temp.path().join("photos");
        let child_path = parent_path.join("nested");
        std::fs::create_dir_all(&child_path).unwrap();
        let parent = PinnedDirectory::open(&parent_path).unwrap();
        let child = PinnedDirectory::open(&child_path).unwrap();

        assert!(parent.overlaps(&child).unwrap());
        assert!(child.overlaps(&parent).unwrap());
    }

    #[test]
    fn synthetic_bind_mounts_fail_closed_even_when_directory_identity_matches() {
        let root_mount = MountIdentity::synthetic(703);
        let bind_mount = MountIdentity::synthetic(416);

        assert!(validate_mount_chain(&root_mount, [&root_mount, &root_mount]).is_ok());
        assert!(validate_mount_chain(&root_mount, [&root_mount, &bind_mount]).is_err());
        assert!(validate_mount_chain(&bind_mount, [&bind_mount, &root_mount]).is_err());
    }

    #[test]
    fn linux_fdinfo_mount_id_parser_is_strict_and_path_free() {
        assert_eq!(
            parse_fdinfo_mount_id("pos:\t0\nflags:\t0100000\nmnt_id:\t416\nino:\t3\n").unwrap(),
            416
        );
        for malformed in [
            "pos:\t0\n",
            "mnt_id:\tnot-a-number\n",
            "mnt_id:\t416\nmnt_id:\t703\n",
            "mnt_id: 416 trailing\n",
        ] {
            let error = parse_fdinfo_mount_id(malformed).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
            assert!(!error.to_string().contains(malformed));
        }
    }

    #[test]
    fn mount_identity_provider_failure_is_path_free_and_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let provider = MountIdentityProvider::for_test(|_| {
            Err(std::io::Error::other("SOURCE SENTINEL PROVIDER DETAIL"))
        });

        let error = PinnedDirectory::open_with_mount_provider(temp.path(), provider).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!error.to_string().contains("SOURCE SENTINEL"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_exact_apfs_data_to_system_firmlink_transition_is_allowed() {
        fn mount(filesystem: &[u8], on: &[u8], from: &[u8], fsid: u8) -> MountIdentity {
            MountIdentity::MacOs {
                fsid: vec![fsid],
                filesystem_type: filesystem.to_vec(),
                mounted_on: on.to_vec(),
                mounted_from: from.to_vec(),
            }
        }

        let data = mount(b"apfs", b"/System/Volumes/Data", b"/dev/disk3s5", 17);
        let system = mount(b"apfs", b"/", b"/dev/disk3s1s1", 19);
        assert!(is_macos_apfs_data_to_system_firmlink(&data, &system));

        for near_miss in [
            mount(b"apfs", b"/other", b"/dev/disk3s5", 17),
            mount(b"nullfs", b"/System/Volumes/Data", b"/dev/disk3s5", 17),
            mount(b"apfs", b"/System/Volumes/Data", b"/dev/disk4s5", 17),
            mount(b"apfs", b"/System/Volumes/Data", b"disk3s5", 17),
        ] {
            assert!(!is_macos_apfs_data_to_system_firmlink(&near_miss, &system));
        }
        for near_miss in [
            mount(b"apfs", b"/System", b"/dev/disk3s1s1", 19),
            mount(b"nullfs", b"/", b"/dev/disk3s1s1", 19),
            mount(b"apfs", b"/", b"/dev/disk4s1s1", 19),
            mount(b"apfs", b"/", b"/dev/disk3", 19),
        ] {
            assert!(!is_macos_apfs_data_to_system_firmlink(&data, &near_miss));
        }
    }

    #[tokio::test]
    async fn every_static_open_rejects_a_mount_transition_with_an_empty_response() {
        for (target, encoding) in [
            ("assets", None),
            ("assets/app.js", None),
            ("assets/app.js.br", Some("br")),
            ("assets/app.js.gz", Some("gzip")),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let web = temp.path().join("web");
            let assets = web.join("assets");
            std::fs::create_dir_all(&assets).unwrap();
            std::fs::write(web.join("index.html"), b"SAFE INTERFACE").unwrap();
            std::fs::write(assets.join("app.js"), b"SAFE ASSET").unwrap();
            std::fs::write(assets.join("app.js.br"), b"SOURCE BR SENTINEL").unwrap();
            std::fs::write(assets.join("app.js.gz"), b"SOURCE GZIP SENTINEL").unwrap();
            let mismatched =
                DirectoryIdentity::from_metadata(&std::fs::metadata(web.join(target)).unwrap());
            let provider = MountIdentityProvider::for_test(move |file| {
                let identity = DirectoryIdentity::from_metadata(&file.metadata()?);
                Ok(if identity == mismatched {
                    MountIdentity::synthetic(416)
                } else {
                    MountIdentity::synthetic(703)
                })
            });
            let pinned = PinnedDirectory::open_with_mount_provider(&web, provider).unwrap();
            let app = router(StaticWebRoot {
                path: Arc::new(web),
                backend: SecureBackend::from_root(pinned),
            });
            let mut request = Request::get("/assets/app.js").body(Body::empty()).unwrap();
            if let Some(encoding) = encoding {
                request
                    .headers_mut()
                    .insert(header::ACCEPT_ENCODING, encoding.parse().unwrap());
            }

            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{target}");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(body.is_empty(), "{target}: {body:?}");
        }
    }

    #[test]
    fn operating_system_name_limit_errors_are_path_free_not_found_errors() {
        let error = safe_open_error(std::io::Error::from_raw_os_error(libc::ENAMETOOLONG));

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "static file is unavailable");
    }

    #[test]
    fn final_file_swap_to_source_symlink_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        let assets = web.join("assets");
        let outside = temp.path().join("source-original.jpg");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("app.js"), b"SAFE ASSET").unwrap();
        std::fs::write(&outside, b"SOURCE SENTINEL").unwrap();
        let root = PinnedDirectory::open(&web).unwrap();

        let result = root.open_file_with_hook(std::path::Path::new("assets/app.js"), || {
            std::fs::remove_file(assets.join("app.js")).unwrap();
            symlink(&outside, assets.join("app.js")).unwrap();
        });

        assert!(result.is_err());
    }

    #[test]
    fn precompressed_sidecar_swaps_to_source_symlinks_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let web = temp.path().join("web");
        let assets = web.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let root = PinnedDirectory::open(&web).unwrap();

        for extension in ["br", "gz"] {
            let sidecar = assets.join(format!("app.js.{extension}"));
            let outside = temp.path().join(format!("source-original.{extension}"));
            std::fs::write(&sidecar, b"SAFE SIDECAR").unwrap();
            std::fs::write(&outside, b"SOURCE SIDECAR SENTINEL").unwrap();

            let result = root.open_file_with_hook(
                std::path::Path::new(&format!("assets/app.js.{extension}")),
                || {
                    std::fs::remove_file(&sidecar).unwrap();
                    symlink(&outside, &sidecar).unwrap();
                },
            );

            assert!(result.is_err(), "{extension}");
        }
    }
}
