use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::path::Component;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::routing::any;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};
use tower::ServiceExt;
use tower_http::services::ServeDir;
use tower_http::services::fs::{Backend, File as TowerFile};
use tower_http::set_header::SetResponseHeaderLayer;

const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const HTML_CACHE: &str = "no-cache";

#[derive(Clone)]
struct StaticHost {
    backend: SecureBackend,
}

pub(crate) fn router(web_root: PathBuf) -> Router {
    let backend = SecureBackend::new(&web_root).unwrap_or_else(|error| {
        tracing::error!(%error, "static web root is unavailable; static serving is disabled");
        SecureBackend::unavailable()
    });
    Router::new()
        .fallback(any(serve))
        .with_state(StaticHost { backend })
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
    let method = request.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return empty_response(StatusCode::METHOD_NOT_ALLOWED);
    }

    let normalized = match NormalizedPath::parse(request.uri().path()) {
        Ok(path) => path,
        Err(()) => return empty_response(StatusCode::BAD_REQUEST),
    };
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

impl NormalizedPath {
    fn parse(raw: &str) -> Result<Self, ()> {
        if !raw.starts_with('/') {
            return Err(());
        }
        let raw = raw.as_bytes();
        let mut decoded = Vec::with_capacity(raw.len());
        let mut index = 0;
        while index < raw.len() {
            match raw[index] {
                b'%' if index + 2 < raw.len() => {
                    let high = hex(raw[index + 1]).ok_or(())?;
                    let low = hex(raw[index + 2]).ok_or(())?;
                    let byte = high << 4 | low;
                    if matches!(byte, b'.' | b'/' | b'\\' | 0) {
                        return Err(());
                    }
                    decoded.push(byte);
                    index += 3;
                }
                b'%' => return Err(()),
                b'\\' | 0 => return Err(()),
                byte => {
                    decoded.push(byte);
                    index += 1;
                }
            }
        }
        let decoded = String::from_utf8(decoded).map_err(|_| ())?;
        let relative = decoded.strip_prefix('/').ok_or(())?;
        let mut components = if relative.is_empty() {
            Vec::new()
        } else {
            relative.split('/').map(str::to_owned).collect::<Vec<_>>()
        };
        if components.last().is_some_and(String::is_empty) {
            components.pop();
        }
        if components
            .iter()
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(());
        }
        let canonical = canonical_uri_path(&components);
        let uri = Uri::from_str(&canonical).map_err(|_| ())?;
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

    fn is_interface_route(&self) -> bool {
        if self
            .components
            .first()
            .is_some_and(|value| value == "api" || value == "healthz")
        {
            return false;
        }
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
    root: Option<Arc<SecureRoot>>,
}

impl SecureBackend {
    fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            root: Some(Arc::new(SecureRoot::open(path)?)),
        })
    }

    fn unavailable() -> Self {
        Self { root: None }
    }
}

#[derive(Debug)]
struct SecureRoot {
    #[cfg(unix)]
    directory: std::fs::File,
}

impl SecureRoot {
    #[cfg(unix)]
    fn open(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;

        let canonical = path.canonicalize()?;
        if !canonical.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "static root is not absolute",
            ));
        }
        let mut directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for component in canonical.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = openat(
                        directory.as_raw_fd(),
                        name,
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )?;
                }
                Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "invalid static root",
                    ));
                }
            }
        }
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "static root is not a directory",
            ));
        }
        Ok(Self { directory })
    }

    #[cfg(not(unix))]
    fn open(_path: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure descriptor-relative static serving is unavailable on this platform",
        ))
    }

    #[cfg(unix)]
    fn open_file(&self, path: &Path) -> io::Result<std::fs::File> {
        self.open_file_with_hook(path, || {})
    }

    #[cfg(not(unix))]
    fn open_file(&self, _path: &Path) -> io::Result<std::fs::File> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "static serving is unavailable",
        ))
    }

    #[cfg(unix)]
    fn open_file_with_hook<F>(&self, path: &Path, before_final_open: F) -> io::Result<std::fs::File>
    where
        F: FnOnce(),
    {
        self.open_with_hook(path, true, before_final_open)
    }

    #[cfg(unix)]
    fn metadata(&self, path: &Path) -> io::Result<std::fs::Metadata> {
        self.open_with_hook(path, false, || {})?.metadata()
    }

    #[cfg(not(unix))]
    fn metadata(&self, _path: &Path) -> io::Result<std::fs::Metadata> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "static serving is unavailable",
        ))
    }

    #[cfg(unix)]
    fn open_with_hook<F>(
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
        }
        before_final_open();
        let file = openat(
            directory.as_raw_fd(),
            components.last().expect("components are non-empty"),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )?;
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
        Some(libc::ELOOP) | Some(libc::ENOTDIR) | Some(libc::EACCES) | Some(libc::EPERM)
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
    use std::os::unix::fs::symlink;

    use super::SecureRoot;

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
        let root = SecureRoot::open(&web).unwrap();

        let file = root
            .open_file_with_hook(std::path::Path::new("assets/app.js"), || {
                std::fs::rename(&assets, web.join("assets-pinned")).unwrap();
                symlink(&outside, &assets).unwrap();
            })
            .unwrap();

        assert_eq!(read(file), b"SAFE ASSET");
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
        let root = SecureRoot::open(&web).unwrap();

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
        let root = SecureRoot::open(&web).unwrap();

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
