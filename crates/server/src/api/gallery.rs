use axum::Json;
use axum::extract::{Path, RawQuery, Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use photo_app_service::{AppServiceError, GalleryScope, SortDirection, WallQueryRequest};
use std::path::Path as FsPath;

use super::types::{CreateSelectionRequest, InteractionRequest, WallParams};
use super::{invalid_request, query_pairs, validate_ascii_identifier, validate_decoded_identifier};
use crate::{AppState, FolderError};

pub(crate) async fn create_selection(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Result<impl IntoResponse, super::ApiError> {
    require_no_query(raw_query.as_deref())?;
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| invalid_request())?;
    let request: CreateSelectionRequest =
        serde_json::from_slice(&body).map_err(|_| invalid_request())?;
    validate_decoded_identifier(&request.path, 4096).map_err(|_| {
        super::ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalidFolderPath",
            "That folder path is not valid.",
        )
    })?;
    crate::folders::validate_relative(&request.path)
        .map_err(|error| map_folder_error(error, request.path.is_empty()))?;
    let engine = state.gallery.as_ref().ok_or_else(|| {
        super::ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sourceUnavailable",
            "The photo source is unavailable.",
        )
    })?;
    let reply = engine
        .check_relative(FsPath::new(&request.path))
        .await
        .map_err(map_service_error)?;
    let summary = match reply {
        photo_app_service::AccessReply::Complete {
            outcome: photo_app_service::FolderProbeOutcome::Available(proof),
            ..
        } => engine.select_validated(&proof).map_err(map_service_error)?,
        photo_app_service::AccessReply::Complete { outcome, .. } => {
            return Err(match outcome {
                photo_app_service::FolderProbeOutcome::Invalid => {
                    map_folder_error(FolderError::InvalidPath, false)
                }
                photo_app_service::FolderProbeOutcome::Unreadable => {
                    map_folder_error(FolderError::Unreadable, request.path.is_empty())
                }
                photo_app_service::FolderProbeOutcome::RootOffline => {
                    map_folder_error(FolderError::Unavailable, true)
                }
                _ => map_folder_error(FolderError::Unavailable, request.path.is_empty()),
            });
        }
        photo_app_service::AccessReply::Checking { .. } => {
            return Err(super::ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "folderUnavailable",
                "That folder is unavailable.",
            ));
        }
    };
    let selection = engine
        .resolve_selection(&summary.id)
        .map_err(map_service_error)?;
    engine
        .ensure_running(&selection)
        .await
        .map_err(map_service_error)?;
    Ok((StatusCode::CREATED, Json(summary)))
}

pub(crate) async fn selection_summary(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, super::ApiError> {
    require_no_query(raw_query.as_deref())?;
    validate_ascii_identifier(&id, 128)?;
    let engine = gallery(&state)?;
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    let summary = engine
        .selection_summary(&selection)
        .map_err(map_service_error)?;
    Ok(Json(summary))
}

pub(crate) async fn wall(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, super::ApiError> {
    validate_ascii_identifier(&id, 128)?;
    let params = parse_wall_params(raw_query.as_deref())?;
    let engine = gallery(&state)?;
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    engine
        .ensure_running(&selection)
        .await
        .map_err(map_service_error)?;
    let page = engine
        .query_wall(
            &selection,
            params.scope,
            WallQueryRequest {
                cursor: params.cursor,
                limit: params.limit,
                direction: params.direction,
            },
        )
        .await
        .map_err(map_service_error)?;
    Ok(Json(page))
}

pub(crate) async fn interaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Result<impl IntoResponse, super::ApiError> {
    require_no_query(raw_query.as_deref())?;
    validate_ascii_identifier(&id, 128)?;
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| invalid_request())?;
    let request: InteractionRequest =
        serde_json::from_slice(&body).map_err(|_| invalid_request())?;
    validate_ascii_identifier(&request.client_id, 128)?;
    let engine = std::sync::Arc::clone(gallery(&state)?);
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    let _ = engine
        .update_client_interaction(&selection, &request.client_id, request.scope, request.state)
        .await
        .map_err(map_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_wall_params(raw_query: Option<&str>) -> Result<WallParams, super::ApiError> {
    let mut scope = None;
    let mut direction = None;
    let mut cursor = None;
    let mut limit = None;
    for (key, value) in query_pairs(raw_query)? {
        match key.as_str() {
            "scope" => scope = Some(parse_enum::<GalleryScope>(&value)?),
            "direction" => direction = Some(parse_enum::<SortDirection>(&value)?),
            "cursor" => {
                if value.len() > 2048 {
                    return Err(invalid_request());
                }
                cursor = Some(value);
            }
            "limit" => {
                let parsed = value.parse::<u32>().map_err(|_| invalid_request())?;
                if !(1..=250).contains(&parsed) {
                    return Err(super::ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "invalidLimit",
                        "That wall page limit is not valid.",
                    ));
                }
                limit = Some(parsed);
            }
            _ => return Err(invalid_request()),
        }
    }
    Ok(WallParams {
        scope: scope.ok_or_else(invalid_request)?,
        direction: direction.ok_or_else(invalid_request)?,
        cursor,
        limit: limit.ok_or_else(invalid_request)?,
    })
}

fn require_no_query(raw_query: Option<&str>) -> Result<(), super::ApiError> {
    if query_pairs(raw_query)?.is_empty() {
        Ok(())
    } else {
        Err(invalid_request())
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, super::ApiError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| invalid_request())
}

pub(crate) fn gallery(
    state: &AppState,
) -> Result<&std::sync::Arc<photo_app_service::GalleryEngine>, super::ApiError> {
    state.gallery.as_ref().ok_or_else(|| {
        super::ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sourceUnavailable",
            "The photo source is unavailable.",
        )
    })
}

pub(crate) fn map_service_error(error: AppServiceError) -> super::ApiError {
    match error {
        AppServiceError::UnknownAsset => super::ApiError::new(
            StatusCode::NOT_FOUND,
            "notFound",
            "That selection is not available.",
        ),
        AppServiceError::InvalidCursor => super::ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalidCursor",
            "That cursor is not valid.",
        ),
        AppServiceError::InvalidLimit => super::ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalidLimit",
            "That wall page limit is not valid.",
        ),
        AppServiceError::DerivativeUnavailable => super::ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "derivativeUnavailable",
            "The requested derivative is not currently available.",
        ),
        AppServiceError::DerivativeFailed => super::ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "derivativeFailed",
            "The requested derivative could not be generated.",
        ),
        _ => invalid_request(),
    }
}

fn map_folder_error(error: FolderError, mounted_root: bool) -> super::ApiError {
    match error {
        FolderError::InvalidPath | FolderError::OutsideRoot => super::ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalidFolderPath",
            "That folder path is not valid.",
        ),
        FolderError::Unavailable | FolderError::NotDirectory | FolderError::Unreadable
            if mounted_root =>
        {
            super::ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "sourceUnavailable",
                "The photo source is unavailable.",
            )
        }
        FolderError::Unavailable | FolderError::NotDirectory => super::ApiError::new(
            StatusCode::NOT_FOUND,
            "folderUnavailable",
            "That folder is unavailable.",
        ),
        FolderError::Unreadable => super::ApiError::new(
            StatusCode::FORBIDDEN,
            "folderUnreadable",
            "That folder cannot be read.",
        ),
    }
}

pub(crate) async fn folder_access(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<impl IntoResponse, super::ApiError> {
    require_no_query(query.as_deref())?;
    validate_ascii_identifier(&id, 128)?;
    let engine = gallery(&state)?;
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    let summary = engine
        .selection_summary(&selection)
        .map_err(map_service_error)?;
    let reply = engine
        .check_relative(FsPath::new(&summary.path))
        .await
        .map_err(map_service_error)?;
    Ok(Json(photo_app_service::FolderAccess::from_reply(
        summary.folder_id,
        &reply,
    )))
}
