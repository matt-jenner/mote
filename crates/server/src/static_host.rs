use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{Method, Request, Response, StatusCode, Uri};
use axum::routing::any;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const HTML_CACHE: &str = "no-cache";

#[derive(Clone)]
struct StaticHost {
    web_root: Arc<PathBuf>,
}

pub(crate) fn router(web_root: PathBuf) -> Router {
    Router::new()
        .fallback(any(serve))
        .with_state(StaticHost {
            web_root: Arc::new(web_root),
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
    let method = request.method().clone();
    let uri = request.uri().clone();
    if method != Method::GET && method != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::empty())
            .expect("static method response is valid");
    }

    let response = ServeDir::new(host.web_root.as_ref())
        .precompressed_br()
        .precompressed_gzip()
        .oneshot(request)
        .await
        .expect("static file service is infallible");
    if response.status() != StatusCode::NOT_FOUND {
        return cache_static(response, &uri, false);
    }
    if !is_interface_route(&uri) {
        return response.map(Body::new);
    }

    let index_request = Request::builder()
        .method(method)
        .uri("/index.html")
        .body(Body::empty())
        .expect("index request is valid");
    let response = ServeFile::new(host.web_root.join("index.html"))
        .precompressed_br()
        .precompressed_gzip()
        .oneshot(index_request)
        .await
        .expect("index file service is infallible");
    cache_static(response, &uri, true)
}

fn is_interface_route(uri: &Uri) -> bool {
    let path = uri.path();
    !path.starts_with("/api/")
        && path != "/api"
        && !path.starts_with("/healthz/")
        && path != "/healthz"
        && Path::new(path).extension().is_none()
}

fn cache_static<B>(response: Response<B>, uri: &Uri, interface_fallback: bool) -> Response<Body>
where
    B: axum::body::HttpBody<Data = axum::body::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    let mut response = response.map(Body::new);
    let cache = if !interface_fallback && uri.path().starts_with("/assets/") {
        HeaderValue::from_static(IMMUTABLE_CACHE)
    } else {
        HeaderValue::from_static(HTML_CACHE)
    };
    response.headers_mut().insert(CACHE_CONTROL, cache);
    response
}
