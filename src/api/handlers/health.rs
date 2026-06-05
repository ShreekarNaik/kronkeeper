use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::api::routes::ServerState;

pub async fn health(State(state): State<ServerState>) -> impl IntoResponse {
    let db_ok = crate::db::pool::ping(&state.pool).await.is_ok();
    let scheduler_ok = state.scheduler_alive.load(Ordering::Relaxed);

    if db_ok && scheduler_ok {
        (
            StatusCode::OK,
            Json(json!({ "status": "ok", "database": "up", "scheduler": "up" })),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "degraded",
                "database": if db_ok { "up" } else { "down" },
                "scheduler": if scheduler_ok { "up" } else { "down" },
            })),
        )
    }
}
