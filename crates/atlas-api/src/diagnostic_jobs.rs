//! Durable SQLite-backed MAT job queue, worker, and status/cancellation endpoints.
use crate::queries;
use crate::{
    ApiError, AppState, ArtifactRecord, diagnostic_plugins, ensure_case_exists, require_uuid,
    run_heap_operation_now,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::fs;
use tracing::error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct HeapOperationRequest {
    #[serde(default)]
    pub(crate) problem_description: Option<String>,
    #[serde(default)]
    pub(crate) artifact_id: Option<String>,
    #[serde(default)]
    pub(crate) class_filter: Option<String>,
    #[serde(default)]
    pub(crate) top_n: Option<usize>,
    #[serde(default)]
    pub(crate) object_id: Option<String>,
    #[serde(default)]
    pub(crate) baseline_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) job_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct DiagnosticJobRow {
    id: String,
    case_id: String,
    artifact_id: Option<String>,
    operation: String,
    status: String,
    progress: i64,
    stage: String,
    cancel_requested: i64,
    error: Option<String>,
    result_json: Option<String>,
    created_at: String,
    updated_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct QueuedDiagnosticJob {
    id: String,
    case_id: String,
    artifact_id: Option<String>,
    operation: String,
    request_json: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DiagnosticJobResponse {
    id: String,
    case_id: String,
    operation: String,
    status: String,
    progress: i64,
    stage: String,
    cancel_requested: bool,
    error: Option<String>,
    result: Option<serde_json::Value>,
    created_at: String,
    updated_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

fn present_job(row: DiagnosticJobRow) -> DiagnosticJobResponse {
    DiagnosticJobResponse {
        id: row.id,
        case_id: row.case_id,
        operation: row.operation,
        status: row.status,
        progress: row.progress,
        stage: row.stage,
        cancel_requested: row.cancel_requested != 0,
        error: row.error,
        result: row
            .result_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
    }
}

pub(crate) async fn enqueue_diagnostic_job(
    state: &AppState,
    id: &str,
    case_id: &str,
    artifact_id: Option<&str>,
    operation: &str,
    request: &HeapOperationRequest,
) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(request).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not encode diagnostic job: {error}"),
        )
    })?;
    sqlx::query("INSERT INTO diagnostic_jobs (id, case_id, artifact_id, baseline_artifact_id, operation, request_json, status, progress, stage) VALUES (?, ?, ?, ?, ?, ?, 'queued', 0, 'Queued for MAT analysis')")
        .bind(id).bind(case_id).bind(artifact_id).bind(if operation == "compare_heap_dumps" { request.baseline_id.as_deref() } else { None }).bind(operation).bind(request_json)
        .execute(&state.pool).await?;
    Ok(())
}

async fn load_job(state: &AppState, job_id: &str) -> Result<DiagnosticJobResponse, ApiError> {
    let row = sqlx::query_as::<_, DiagnosticJobRow>("SELECT id, case_id, artifact_id, operation, status, progress, stage, cancel_requested, error, result_json, created_at, updated_at, started_at, finished_at FROM diagnostic_jobs WHERE id = ?")
        .bind(job_id).fetch_optional(&state.pool).await?.ok_or_else(ApiError::not_found)?;
    Ok(present_job(row))
}

pub(crate) async fn run_heap_operation(
    State(state): State<AppState>,
    Path((case_id, operation)): Path<(String, String)>,
    Json(mut input): Json<HeapOperationRequest>,
) -> Result<(StatusCode, Json<DiagnosticJobResponse>), ApiError> {
    require_uuid(&case_id)?;
    ensure_case_exists(&state.pool, &case_id).await?;
    if !matches!(
        operation.as_str(),
        "analyze_heap_dump"
            | "generate_leak_suspects"
            | "generate_heap_overview"
            | "get_histogram"
            | "get_dominator_tree"
            | "find_paths_to_gc_roots"
            | "compare_heap_dumps"
    ) {
        return Err(ApiError::bad_request("Unsupported heap operation."));
    }
    if input.top_n.is_some_and(|value| !(1..=500).contains(&value)) {
        return Err(ApiError::bad_request("top_n must be between 1 and 500."));
    }
    if input
        .class_filter
        .as_deref()
        .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control))
    {
        return Err(ApiError::bad_request(
            "class_filter must be at most 256 characters and contain no control characters.",
        ));
    }
    if operation == "find_paths_to_gc_roots" {
        let object_id = input.object_id.as_deref().ok_or_else(|| {
            ApiError::bad_request("object_id is required for find_paths_to_gc_roots.")
        })?;
        diagnostic_plugins::validate_object_id(object_id).map_err(ApiError::bad_request)?;
    }
    if operation != "compare_heap_dumps" && input.baseline_id.is_some() {
        return Err(ApiError::bad_request(
            "baseline_id is only valid for compare_heap_dumps.",
        ));
    }
    let records = sqlx::query_as::<_, ArtifactRecord>(queries::LIST_ARTIFACTS)
        .bind(&case_id)
        .fetch_all(&state.pool)
        .await?;
    let baseline = if operation == "compare_heap_dumps" {
        let baseline_id = input.baseline_id.as_deref().ok_or_else(|| {
            ApiError::bad_request("baseline_id is required for compare_heap_dumps.")
        })?;
        Uuid::parse_str(baseline_id)
            .map_err(|_| ApiError::bad_request("baseline_id must be a valid UUID."))?;
        Some(
            records
                .iter()
                .find(|item| item.id == baseline_id && item.kind == "heap_dump")
                .cloned()
                .ok_or_else(|| {
                    ApiError::bad_request(
                        "baseline_id must identify a saved heap dump in this case.",
                    )
                })?,
        )
    } else {
        None
    };
    let primary = if let Some(artifact_id) = input.artifact_id.as_deref() {
        Uuid::parse_str(artifact_id)
            .map_err(|_| ApiError::bad_request("artifact_id must be a valid UUID."))?;
        records
            .iter()
            .find(|item| item.id == artifact_id && item.kind == "heap_dump")
            .cloned()
            .ok_or_else(ApiError::not_found)?
    } else {
        records
            .iter()
            .find(|item| {
                item.kind == "heap_dump" && baseline.as_ref().is_none_or(|base| base.id != item.id)
            })
            .cloned()
            .ok_or_else(|| ApiError::bad_request("No saved heap dump is available in this case."))?
    };
    if baseline.as_ref().is_some_and(|base| base.id == primary.id) {
        return Err(ApiError::bad_request(
            "artifact_id and baseline_id must refer to different heap dumps.",
        ));
    }
    input.artifact_id = Some(primary.id.clone());
    let id = Uuid::new_v4().to_string();
    input.job_id = Some(id.clone());
    enqueue_diagnostic_job(&state, &id, &case_id, Some(&primary.id), &operation, &input).await?;
    let job = load_job(&state, &id).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub(crate) async fn get_diagnostic_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<DiagnosticJobResponse>, ApiError> {
    require_uuid(&job_id)?;
    Ok(Json(load_job(&state, &job_id).await?))
}

pub(crate) async fn cancel_diagnostic_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<DiagnosticJobResponse>, ApiError> {
    require_uuid(&job_id)?;

    // Perform the queued-to-cancelled or running-cancel-requested transition in
    // one SQLite statement. This prevents a worker from claiming a queued job
    // between a SELECT and a later conditional UPDATE.
    let transition = sqlx::query_as::<_, (String, String, Option<String>)>(
        "UPDATE diagnostic_jobs \
         SET status=CASE WHEN status='queued' THEN 'cancelled' ELSE 'running' END, \
             progress=CASE WHEN status='queued' THEN 100 ELSE progress END, \
             stage=CASE WHEN status='queued' THEN 'Cancelled before execution' ELSE 'Cancellation requested' END, \
             cancel_requested=1, \
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
             finished_at=CASE WHEN status='queued' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE finished_at END \
         WHERE id=? AND status IN ('queued','running') \
         RETURNING status, case_id, artifact_id"
    )
    .bind(&job_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((status, case_id, artifact_id)) = transition
        && status == "running"
    {
        if let Some(artifact_id) = artifact_id.as_deref() {
            let workspace = state
                .data_dir
                .join("work")
                .join(&case_id)
                .join("heap")
                .join(artifact_id);
            let marker = workspace.join(format!(".cancel-requested-{job_id}"));
            if fs::create_dir_all(&workspace).await.is_ok()
                && let Err(error) = fs::write(marker, b"cancel requested\n").await
            {
                error!(job_id = %job_id, error = %error, "could not persist MAT cancellation marker");
            }
        }
        diagnostic_plugins::cancel_mat_job(&job_id).await;
    }

    // A terminal job is left unchanged. A missing ID is reported as not found by load_job.
    Ok(Json(load_job(&state, &job_id).await?))
}

async fn recover_interrupted_diagnostic_jobs(state: &AppState) {
    let stale =
        sqlx::query_scalar::<_, String>("SELECT id FROM diagnostic_jobs WHERE status='running'")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    for id in stale {
        diagnostic_plugins::cancel_mat_job(&id).await;
    }
    if let Err(error) = sqlx::query("UPDATE diagnostic_jobs SET status='failed', progress=100, stage='Interrupted by API restart', error='Atlas restarted while this job was running. Submit it again.', updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status='running'")
        .execute(&state.pool).await {
        error!(error = %error, "could not recover interrupted diagnostic jobs");
    }
}

pub(crate) async fn run_diagnostic_job_worker(state: AppState) {
    recover_interrupted_diagnostic_jobs(&state).await;
    loop {
        let claimed = sqlx::query_as::<_, QueuedDiagnosticJob>("UPDATE diagnostic_jobs SET status='running', progress=10, stage='Starting MAT operation', started_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=(SELECT id FROM diagnostic_jobs WHERE status='queued' ORDER BY created_at LIMIT 1) AND status='queued' RETURNING id, case_id, artifact_id, operation, request_json")
            .fetch_optional(&state.pool).await;
        let job = match claimed {
            Ok(Some(job)) => job,
            Ok(None) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            Err(error) => {
                error!(error = %error, "diagnostic job claim failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        if let Err(error) = sqlx::query("UPDATE diagnostic_jobs SET progress=25, stage='Running MAT; job status remains queryable', updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND status='running'")
            .bind(&job.id).execute(&state.pool).await {
            error!(job_id = %job.id, error = %error, "could not update diagnostic job stage");
        }
        let request = match serde_json::from_str::<HeapOperationRequest>(&job.request_json) {
            Ok(mut request) => {
                request.job_id = Some(job.id.clone());
                request
            }
            Err(error) => {
                let message = format!("Stored diagnostic job request is invalid: {error}");
                let _ = sqlx::query("UPDATE diagnostic_jobs SET status='failed', progress=100, stage='Invalid stored request', error=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
                    .bind(message).bind(&job.id).execute(&state.pool).await;
                continue;
            }
        };
        let result = run_heap_operation_now(
            State(state.clone()),
            Path((job.case_id.clone(), job.operation.clone())),
            Json(request),
        )
        .await;
        let cancellation_requested =
            sqlx::query_scalar::<_, i64>("SELECT cancel_requested FROM diagnostic_jobs WHERE id=?")
                .bind(&job.id)
                .fetch_optional(&state.pool)
                .await
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0;
        match result {
            Ok(response) => {
                let result_json = serde_json::to_string(&response.0).ok();
                let plugin_status = response
                    .0
                    .preparations
                    .first()
                    .map(|p| p.status.as_str())
                    .unwrap_or("failed");
                let final_status = if cancellation_requested {
                    "cancelled"
                } else if plugin_status == "succeeded" {
                    "succeeded"
                } else {
                    "failed"
                };
                let final_stage = if cancellation_requested {
                    "Cancelled"
                } else {
                    match final_status {
                        "failed" => "MAT operation failed",
                        "succeeded" => "MAT operation completed",
                        _ => "Finished",
                    }
                };
                let _ = sqlx::query("UPDATE diagnostic_jobs SET status=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE ? END, progress=100, stage=CASE WHEN cancel_requested=1 THEN 'Cancelled' ELSE ? END, result_json=?, error=CASE WHEN cancel_requested=1 THEN NULL WHEN ?='failed' THEN 'MAT operation failed; inspect the result warnings and container log.' ELSE NULL END, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
                    .bind(final_status).bind(final_stage).bind(result_json).bind(final_status).bind(&job.id).execute(&state.pool).await;
            }
            Err(error) => {
                let final_status = if cancellation_requested {
                    "cancelled"
                } else {
                    "failed"
                };
                let _ = sqlx::query("UPDATE diagnostic_jobs SET status=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE ? END, progress=100, stage=CASE WHEN cancel_requested=1 THEN 'Cancelled' ELSE ? END, error=CASE WHEN cancel_requested=1 THEN NULL ELSE ? END, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
                    .bind(final_status).bind(if cancellation_requested { "Cancelled" } else { "Failed" }).bind(error.message).bind(&job.id).execute(&state.pool).await;
            }
        }
        if let Some(artifact_id) = job.artifact_id.as_deref() {
            let marker = state
                .data_dir
                .join("work")
                .join(&job.case_id)
                .join("heap")
                .join(artifact_id)
                .join(format!(".cancel-requested-{}", job.id));
            let _ = fs::remove_file(marker).await;
        }
    }
}
