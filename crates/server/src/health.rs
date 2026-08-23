use std::fs::OpenOptions;

use axum::Json;
use axum::http::StatusCode;
use serde::Serialize;

use crate::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ComponentHealth {
    pub status: ComponentStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Healthy,
    Unhealthy,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SourceHealthCounts {
    pub available: u64,
    pub unavailable: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub database: ComponentHealth,
    pub cache: ComponentHealth,
    pub sources: SourceHealthCounts,
    pub active_warnings: u64,
}

pub(crate) async fn healthz(state: AppState) -> (StatusCode, Json<HealthReport>) {
    let snapshot = state
        .catalog
        .lock()
        .ok()
        .and_then(|catalog| catalog.health_snapshot().ok());
    let database_status = if snapshot.is_some() {
        ComponentStatus::Healthy
    } else {
        ComponentStatus::Unhealthy
    };
    let cache_status = if cache_is_writable(&state.cache_root) {
        ComponentStatus::Healthy
    } else {
        ComponentStatus::Unhealthy
    };
    let sources =
        snapshot.map_or_else(SourceHealthCounts::default, |snapshot| SourceHealthCounts {
            available: snapshot.available_sources,
            unavailable: snapshot.unavailable_sources,
        });
    let active_warnings = snapshot.map_or(0, |snapshot| snapshot.active_warnings);
    let status = if database_status == ComponentStatus::Unhealthy
        || cache_status == ComponentStatus::Unhealthy
    {
        HealthStatus::Unhealthy
    } else if sources.unavailable > 0 || active_warnings > 0 {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };
    let status_code = if status == HealthStatus::Unhealthy {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        status_code,
        Json(HealthReport {
            status,
            database: ComponentHealth {
                status: database_status,
            },
            cache: ComponentHealth {
                status: cache_status,
            },
            sources,
            active_warnings,
        }),
    )
}

fn cache_is_writable(root: &std::path::Path) -> bool {
    let probe = root.join(format!(".healthz-{}", uuid::Uuid::new_v4()));
    let opened = OpenOptions::new().create_new(true).write(true).open(&probe);
    match opened {
        Ok(file) => {
            drop(file);
            std::fs::remove_file(probe).is_ok()
        }
        Err(_) => false,
    }
}
