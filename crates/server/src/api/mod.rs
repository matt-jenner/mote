mod error;
mod types;

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::AppState;
use crate::folders::FolderError;

pub use error::ApiError;
pub use types::{BootstrapResponse, Capabilities};

pub(crate) async fn bootstrap(State(state): State<AppState>) -> impl IntoResponse {
    let source_available = state
        .folder_root
        .as_ref()
        .is_some_and(|root| root.is_available());
    Json(BootstrapResponse {
        capabilities: Capabilities {
            folder_browser: state.folder_root.is_some(),
            video: false,
        },
        source_available,
    })
}

pub(crate) async fn folders(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, ApiError> {
    let path = parse_folder_path(raw_query.as_deref())?;
    let root = state.folder_root.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sourceUnavailable",
            "The photo source is unavailable.",
        )
    })?;
    root.list(&path)
        .map(Json)
        .map_err(|error| map_folder_error(error, path.is_empty()))
}

fn parse_folder_path(query: Option<&str>) -> Result<String, ApiError> {
    let mut path = None;
    for pair in query.unwrap_or_default().split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key)?;
        if key != "path" {
            continue;
        }
        if path.is_some() {
            return Err(invalid_folder_path());
        }
        path = Some(percent_decode(value)?);
    }
    let path = path.unwrap_or_default();
    if path.len() > 4096 {
        return Err(invalid_folder_path());
    }
    Ok(path)
}

fn percent_decode(value: &str) -> Result<String, ApiError> {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let high = hex(bytes[index + 1]);
                let low = hex(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    decoded.push(high << 4 | low);
                    index += 2;
                } else {
                    return Err(invalid_folder_path());
                }
            }
            b'%' => return Err(invalid_folder_path()),
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| invalid_folder_path())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid_folder_path() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalidFolderPath",
        "That folder path is not valid.",
    )
}

fn map_folder_error(error: FolderError, mounted_root: bool) -> ApiError {
    match error {
        FolderError::InvalidPath | FolderError::OutsideRoot => invalid_folder_path(),
        FolderError::Unavailable if mounted_root => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sourceUnavailable",
            "The photo source is unavailable.",
        ),
        FolderError::Unavailable | FolderError::NotDirectory => ApiError::new(
            StatusCode::NOT_FOUND,
            "folderUnavailable",
            "That folder is unavailable.",
        ),
        FolderError::Unreadable => ApiError::new(
            StatusCode::FORBIDDEN,
            "folderUnreadable",
            "That folder cannot be read.",
        ),
    }
}
