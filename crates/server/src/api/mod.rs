mod derivative;
mod error;
mod events;
mod gallery;
mod types;

pub(crate) use derivative::{derivative, request_derivatives};
pub(crate) use events::events;
pub(crate) use gallery::{
    create_selection, folder_access, interaction, resolve_assets, selection_summary, wall,
};

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::collections::HashSet;
use std::sync::Arc;

use crate::AppState;
use crate::folders::FolderError;

pub use error::ApiError;
pub use types::{BootstrapResponse, Capabilities};

const MAX_QUERY_BYTES: usize = 8192;

pub(crate) fn invalid_request() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalidRequest",
        "That request is not valid.",
    )
}

pub(crate) async fn route_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "notFound",
        "That route is unavailable.",
    )
}

pub(crate) fn guard_query_shape(raw_query: Option<&str>) -> Result<(), ApiError> {
    let Some(raw_query) = raw_query else {
        return Ok(());
    };
    if raw_query.len() > MAX_QUERY_BYTES {
        return Err(invalid_request());
    }
    let mut count = 0;
    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        count += 1;
        if count > 8 {
            return Err(invalid_request());
        }
    }
    Ok(())
}

pub(crate) fn query_pairs(raw_query: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    guard_query_shape(raw_query)?;
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for pair in raw_query.unwrap_or_default().split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key).map_err(|_| invalid_request())?;
        if !seen.insert(key.clone()) {
            return Err(invalid_request());
        }
        pairs.push((key, percent_decode(value).map_err(|_| invalid_request())?));
    }
    Ok(pairs)
}

pub(crate) fn validate_ascii_identifier(value: &str, max: usize) -> Result<(), ApiError> {
    if value.len() > max || !value.is_ascii() {
        Err(invalid_request())
    } else {
        Ok(())
    }
}

pub(crate) fn validate_decoded_identifier(value: &str, max: usize) -> Result<(), ApiError> {
    if value.len() > max {
        Err(invalid_request())
    } else {
        Ok(())
    }
}

pub(crate) async fn bootstrap(State(state): State<AppState>) -> impl IntoResponse {
    let source_available = if let Some(engine) = &state.gallery {
        matches!(
            engine.check_relative(std::path::Path::new("")).await,
            Ok(photo_app_service::AccessReply::Complete {
                outcome: photo_app_service::FolderProbeOutcome::Available(_),
                ..
            })
        )
    } else if let Some(root) = state.folder_root.clone() {
        tokio::task::spawn_blocking(move || root.is_available())
            .await
            .unwrap_or(false)
    } else {
        false
    };
    Json(BootstrapResponse {
        root_id: state.gallery.as_ref().and_then(|engine| engine.root_id()),
        capabilities: Capabilities {
            folder_browser: state.folder_root.is_some(),
            video: false,
            original_downloads: state.allow_original_downloads,
        },
        source_available,
    })
}

pub(crate) async fn folders(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, ApiError> {
    guard_query_shape(raw_query.as_deref())?;
    let path = parse_folder_path(raw_query.as_deref())?;
    let include_image_count = parse_image_count(raw_query.as_deref())?;
    let root = state.folder_root.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sourceUnavailable",
            "The photo source is unavailable.",
        )
    })?;
    let root = Arc::clone(root);
    let request_path = path.clone();
    let listing = tokio::task::spawn_blocking(move || {
        if include_image_count {
            root.list_with_image_count(&request_path)
        } else {
            root.list(&request_path)
        }
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internalError",
            "The folder could not be listed.",
        )
    })?;
    if path.is_empty() {
        match &listing {
            Ok(_) => state.record_source_root_listing(true).await,
            Err(FolderError::Unavailable | FolderError::NotDirectory | FolderError::Unreadable) => {
                state.record_source_root_listing(false).await;
            }
            Err(FolderError::InvalidPath | FolderError::OutsideRoot) => {}
        }
    }
    listing
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

fn parse_image_count(query: Option<&str>) -> Result<bool, ApiError> {
    let mut requested = None;
    for pair in query.unwrap_or_default().split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key)? != "includeImageCount" {
            continue;
        }
        if requested.is_some() {
            return Err(invalid_request());
        }
        requested = Some(match percent_decode(value)?.as_str() {
            "true" => true,
            "false" => false,
            _ => return Err(invalid_request()),
        });
    }
    Ok(requested.unwrap_or(false))
}

pub(crate) fn percent_decode(value: &str) -> Result<String, ApiError> {
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
        FolderError::Unavailable | FolderError::NotDirectory | FolderError::Unreadable
            if mounted_root =>
        {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "sourceUnavailable",
                "The photo source is unavailable.",
            )
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivative_identifier_limit_is_decoded_and_fixed() {
        assert!(validate_decoded_identifier(&"x".repeat(512), 512).is_ok());
        assert!(validate_decoded_identifier(&"x".repeat(513), 512).is_err());
    }
}
