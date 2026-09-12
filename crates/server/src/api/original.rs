use axum::extract::{Path, RawQuery, State, rejection::PathRejection};
use axum::http::StatusCode;
use axum::response::Response;

use super::{ApiError, invalid_request, query_pairs};
use crate::AppState;

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "originalUnavailable",
        "That original is unavailable.",
    )
}

pub(crate) async fn original(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    if !state.allow_original_downloads {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "originalDownloadsDisabled",
            "This site does not offer original downloads.",
        ));
    }
    let Path(id) = path.map_err(|_| invalid_request())?;
    if !query_pairs(query.as_deref())?.is_empty() || id.len() != 36 {
        return Err(invalid_request());
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_request())?;
    let asset_id = photo_domain::AssetId::from_uuid(uuid);

    #[cfg(unix)]
    {
        use axum::body::Body;
        use axum::http::{HeaderValue, header};
        use tokio::io::AsyncReadExt;
        use tokio_util::io::ReaderStream;

        let gallery = state.gallery.clone().ok_or_else(unavailable)?;
        let root = state.original_root.clone().ok_or_else(unavailable)?;
        // Catalog lookup, descriptor walk, and metadata are all blocking work.
        let (file, length, filename) = tokio::task::spawn_blocking(move || {
            let (relative, filename, _kind) = gallery
                .resolve_hosted_original(asset_id)
                .map_err(|_| unavailable())?;
            let relative = relative.to_path_buf().map_err(|_| unavailable())?;
            let file = root
                .open_regular_file(&relative)
                .map_err(|_| unavailable())?;
            let length = file.metadata().map_err(|_| unavailable())?.len();
            Ok::<_, ApiError>((file, length, filename))
        })
        .await
        .map_err(|_| unavailable())??;

        // Bound the stream to the descriptor's measured length if the file grows.
        let stream = ReaderStream::new(tokio::fs::File::from_std(file).take(length));
        let mut response = Response::new(Body::from_stream(stream));
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&length.to_string()).map_err(|_| unavailable())?,
        );
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&attachment(&filename)).map_err(|_| unavailable())?,
        );
        Ok(response)
    }
    #[cfg(not(unix))]
    {
        let _ = asset_id;
        Err(unavailable())
    }
}

#[cfg(unix)]
fn attachment(filename: &str) -> String {
    let safe: String = filename
        .chars()
        .map(|ch| {
            if ch.is_control() || ch == '/' || ch == '\\' {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let fallback: String = safe
        .chars()
        .map(|ch| if ch.is_ascii() && ch != '"' { ch } else { '_' })
        .collect();
    let mut encoded = String::new();
    for byte in safe.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}
