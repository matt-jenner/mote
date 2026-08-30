use axum::body::{Body, Bytes};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use futures_util::stream;
use photo_app_service::{GalleryScope, SelectionEventSubscription};
use std::convert::Infallible;
use std::pin::Pin;
use std::time::Duration;

use super::gallery::{gallery, map_service_error};
use super::types::EventsParams;
use super::{invalid_request, query_pairs, validate_ascii_identifier};
use crate::AppState;

pub(crate) async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, super::ApiError> {
    validate_ascii_identifier(&id, 128)?;
    let params = parse_events_params(raw_query.as_deref())?;
    validate_ascii_identifier(&params.client_id, 128)?;
    let replay = replay_id(&headers, params.after_event_id)?;
    let engine = gallery(&state)?;
    let selection = engine.resolve_selection(&id).map_err(map_service_error)?;
    engine
        .ensure_running(&selection)
        .await
        .map_err(map_service_error)?;
    let subscription = engine.subscribe(&selection, params.client_id, params.scope, replay);
    let body = Body::from_stream(event_stream(subscription));
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

fn parse_events_params(raw_query: Option<&str>) -> Result<EventsParams, super::ApiError> {
    let mut client_id = None;
    let mut scope = None;
    let mut after_event_id = None;
    for (key, value) in query_pairs(raw_query)? {
        match key.as_str() {
            "clientId" => client_id = Some(value),
            "scope" => {
                scope = Some(
                    serde_json::from_value::<GalleryScope>(serde_json::Value::String(value))
                        .map_err(|_| invalid_request())?,
                )
            }
            "afterEventId" => {
                if value.len() > 20 || !value.is_ascii() {
                    return Err(invalid_request());
                }
                after_event_id = Some(value.parse::<u64>().map_err(|_| invalid_request())?);
            }
            _ => return Err(invalid_request()),
        }
    }
    Ok(EventsParams {
        client_id: client_id.ok_or_else(invalid_request)?,
        scope: scope.ok_or_else(invalid_request)?,
        after_event_id,
    })
}

fn replay_id(headers: &HeaderMap, query_id: Option<u64>) -> Result<Option<u64>, super::ApiError> {
    let Some(header) = headers.get("last-event-id") else {
        return Ok(query_id);
    };
    let value = header.to_str().map_err(|_| invalid_request())?;
    if value.len() > 20 || !value.is_ascii() {
        return Err(invalid_request());
    }
    Ok(Some(value.parse::<u64>().map_err(|_| invalid_request())?))
}

struct EventStreamState {
    subscription: SelectionEventSubscription,
    heartbeat: Pin<Box<tokio::time::Sleep>>,
}

fn event_stream(
    subscription: SelectionEventSubscription,
) -> impl futures_util::Stream<Item = Result<Bytes, Infallible>> {
    stream::unfold(
        EventStreamState {
            subscription,
            heartbeat: Box::pin(tokio::time::sleep(Duration::from_secs(15))),
        },
        |mut state| async move {
            tokio::select! {
                event = state.subscription.recv() => {
                    event.map(|event| (Ok(Bytes::from(format_event(&event))), state))
                }
                _ = &mut state.heartbeat => {
                    state.heartbeat.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(15));
                    Some((Ok(Bytes::from_static(b": heartbeat\n\n")), state))
                }
            }
        },
    )
}

fn format_event(event: &photo_app_service::SequencedWallUpdate) -> String {
    let data = serde_json::to_string(&event.update).unwrap_or_else(|_| "{}".to_owned());
    format!("event: wallUpdate\nid: {}\ndata: {data}\n\n", event.id)
}
