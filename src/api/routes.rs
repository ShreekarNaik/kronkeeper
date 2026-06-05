use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::api::handlers::health::health;
use crate::api::handlers::jobs::{cancel_job, create_job, get_job, patch_job, AppState};
use crate::api::handlers::metrics::metrics;
use crate::api::middleware::auth::{require_api_key, AuthState};

#[derive(Clone)]
pub struct ServerState {
    pub pool: PgPool,
    pub app: AppState,
    pub auth: AuthState,
    pub metrics: PrometheusHandle,
    pub scheduler_alive: Arc<AtomicBool>,
}

pub fn build_router(state: ServerState) -> Router {
    let protected = Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/{id}", get(get_job))
        .route("/api/v1/jobs/{id}", delete(cancel_job))
        .route("/api/v1/jobs/{id}", patch(patch_job))
        .route("/metrics", get(metrics))
        .layer(middleware::from_fn_with_state(state.auth.clone(), require_api_key))
        .with_state(state.clone());

    let public = Router::new()
        .route("/health", get(health))
        .with_state(state);

    Router::new()
        .merge(protected)
        .merge(public)
        .layer(TraceLayer::new_for_http())
}
