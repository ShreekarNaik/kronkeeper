use axum::extract::State;
use axum::response::IntoResponse;

use crate::api::routes::ServerState;

pub async fn metrics(State(state): State<ServerState>) -> impl IntoResponse {
    state.metrics.render()
}
