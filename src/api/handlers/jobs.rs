use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::api::error::ApiError;
use crate::api::routes::ServerState;
use crate::db::JobRepository;
use crate::metrics::record_job_scheduled;
use crate::models::{AuthorizedClient, CreateJobRequest, JobResponse, PatchJobRequest};
use crate::recurring::RecurringService;

#[derive(Clone)]
pub struct AppState {
    pub repo: JobRepository,
    pub notify: Arc<Notify>,
    pub recurring: Arc<RecurringService>,
}

pub async fn create_job(
    State(state): State<ServerState>,
    client: AuthorizedClient,
    Json(req): Json<CreateJobRequest>,
) -> Result<(StatusCode, Json<JobResponse>), ApiError> {
    let app = &state.app;

    if req.idempotency_key.is_empty() {
        return Err(ApiError::BadRequest("idempotency_key is required".into()));
    }

    if let Some(existing) = app.repo.get_by_idempotency_key(&req.idempotency_key).await? {
        return Ok((StatusCode::OK, Json(JobResponse::from(existing))));
    }

    let job = if let Some(ref recurrence) = req.recurrence {
        let (_template, instance) = app
            .recurring
            .create_recurring(client.client_id, &req, recurrence)
            .await?;
        instance
    } else {
        app.repo
            .insert_job(
                client.client_id,
                &req,
                false,
                false,
                None,
                None,
                None,
                None,
                crate::models::ConcurrencyPolicy::QueueOnce,
                None,
                req.scheduled_at,
            )
            .await?
    };

    record_job_scheduled();
    app.notify.notify_one();

    Ok((StatusCode::CREATED, Json(JobResponse::from(job))))
}

pub async fn get_job(
    State(state): State<ServerState>,
    client: AuthorizedClient,
    Path(id): Path<Uuid>,
) -> Result<Json<JobResponse>, ApiError> {
    let job = state
        .app
        .repo
        .get_job_by_id(id, client.client_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(JobResponse::from(job)))
}

pub async fn cancel_job(
    State(state): State<ServerState>,
    client: AuthorizedClient,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let app = &state.app;
    let job = app
        .repo
        .get_job_by_id(id, client.client_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    if job.is_template {
        let count = app
            .repo
            .cancel_template_and_instances(id, client.client_id)
            .await?;
        if count == 0 {
            return Err(ApiError::NotFound);
        }
    } else if !app.repo.cancel_job(id, client.client_id).await? {
        return Err(ApiError::BadRequest(
            "only scheduled jobs can be cancelled".into(),
        ));
    }

    app.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn patch_job(
    State(state): State<ServerState>,
    client: AuthorizedClient,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchJobRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    let app = &state.app;

    if req.cron_expr.is_empty() {
        return Err(ApiError::BadRequest("cron_expr is required".into()));
    }

    if !app
        .repo
        .update_cron_expr(id, client.client_id, &req.cron_expr)
        .await?
    {
        return Err(ApiError::NotFound);
    }

    let job = app
        .repo
        .get_job_by_id(id, client.client_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    app.notify.notify_one();
    Ok(Json(JobResponse::from(job)))
}
