use axum::body::Body;
use axum::extract::{Path, RawQuery, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use photo_app_service::AppServiceError;

use super::gallery::{gallery, map_service_error};
use super::types::DerivativeHttpRequest;
use super::{invalid_request, query_pairs, validate_ascii_identifier, validate_decoded_identifier};
use crate::AppState;

pub(crate) async fn request_derivatives(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Result<impl IntoResponse, super::ApiError> {
    if !query_pairs(raw_query.as_deref())?.is_empty() {
        return Err(invalid_request());
    }
    validate_ascii_identifier(&id, 128)?;
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| invalid_request())?;
    let request: DerivativeHttpRequest =
        serde_json::from_slice(&body).map_err(|_| invalid_request())?;
    if !(1..=250).contains(&request.request.asset_ids.len()) {
        return Err(invalid_request());
    }
    for asset_id in &request.request.asset_ids {
        validate_ascii_identifier(asset_id, 128)?;
    }
    let engine = gallery(&state)?;
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    engine
        .ensure_running(&selection)
        .await
        .map_err(map_service_error)?;
    engine
        .request_derivatives(&selection, request.scope, request.request)
        .await
        .map_err(map_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn derivative(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
    headers: axum::http::HeaderMap,
) -> Result<Response, super::ApiError> {
    if !query_pairs(raw_query.as_deref())?.is_empty() {
        return Err(invalid_request());
    }
    validate_decoded_identifier(&id, 512)?;
    let engine = gallery(&state)?;
    let managed = engine.open_derivative(&id).map_err(|error| match error {
        AppServiceError::UnknownAsset
        | AppServiceError::Cache(_)
        | AppServiceError::Catalog(_)
        | AppServiceError::StatePoisoned => super::ApiError::new(
            StatusCode::NOT_FOUND,
            "notFound",
            "That derivative is not available.",
        ),
        _ => invalid_request(),
    })?;
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == managed.etag)
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        response.headers_mut().insert(ETAG, header(&managed.etag)?);
        response.headers_mut().insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
        response.headers_mut().insert(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        );
        return Ok(response);
    }
    let mut file = managed.file;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|_| {
        super::ApiError::new(
            StatusCode::NOT_FOUND,
            "notFound",
            "That derivative is not available.",
        )
    })?;
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(managed.content_type));
    response.headers_mut().insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&managed.content_length.to_string())
            .map_err(|_| invalid_request())?,
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response.headers_mut().insert(ETAG, header(&managed.etag)?);
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn header(value: &str) -> Result<HeaderValue, super::ApiError> {
    HeaderValue::from_str(value).map_err(|_| invalid_request())
}
