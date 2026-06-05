use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthorizedClient {
    pub client_id: Uuid,
}

impl<S> FromRequestParts<S> for AuthorizedClient
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthorizedClient>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
