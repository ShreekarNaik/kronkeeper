use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use uuid::Uuid;

use crate::models::AuthorizedClient;

#[derive(Clone)]
pub struct AuthState {
    pub api_keys: Arc<DashMap<String, Uuid>>,
}

pub async fn require_api_key(
    State(auth): State<AuthState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let api_key = req
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let Some(api_key) = api_key else {
        return (StatusCode::UNAUTHORIZED, "missing X-API-Key header").into_response();
    };

    let Some(client_id) = auth.api_keys.get(&api_key).map(|e| *e.value()) else {
        return (StatusCode::UNAUTHORIZED, "invalid API key").into_response();
    };

    req.extensions_mut().insert(AuthorizedClient { client_id });
    next.run(req).await
}

pub async fn load_api_keys(repo: &crate::db::JobRepository) -> anyhow::Result<Arc<DashMap<String, Uuid>>> {
    let map = DashMap::new();
    for (api_key, client_id) in repo.load_api_keys().await? {
        map.insert(api_key, client_id);
    }
    Ok(Arc::new(map))
}
