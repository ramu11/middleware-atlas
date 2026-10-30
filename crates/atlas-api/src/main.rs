//! HTTP service entry point: loads configuration, initializes private local storage and SQLite, starts workers, and registers API routes.
mod agent;
mod config;
mod constants;
mod diagnostic_jobs;
mod diagnostic_plugins;
mod evidence;
mod knowledge;
mod queries;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use config::AppConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    FromRow, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::sync::Arc;
use std::{
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    time::Duration,
};
use tokio::{fs, io::AsyncWriteExt};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone)]
/// Shared application state for routes that require SQLite, local storage, upload limits, and runtime configuration.
pub(crate) struct AppState {
    pub(crate) pool: SqlitePool,
    pub(crate) data_dir: PathBuf,
    pub(crate) max_upload_bytes: u64,
    pub(crate) config: Arc<AppConfig>,
}

#[derive(Debug)]
/// Internal error type carrying an HTTP status and client-safe message.
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    /// Constructs an API error with its HTTP status and client-safe message.
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    /// Creates a standard HTTP 400 API error.
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    /// Creates the standard HTTP 404 API error without disclosing identifier details.
    pub(crate) fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, constants::ERROR_NOT_FOUND)
    }
    /// Creates a payload-too-large error using the configured upload limit.
    pub(crate) fn too_large(limit: u64) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            constants::upload_too_large(limit),
        )
    }
}

#[derive(Serialize)]
/// JSON error response shape exposed by the API.
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    /// Converts the internal API error into the JSON HTTP response format.
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    /// Maps a database or filesystem failure to a client-safe internal API error while logging details.
    fn from(value: sqlx::Error) -> Self {
        error!(error = %value, "database operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            constants::ERROR_PERSISTENCE,
        )
    }
}

impl From<std::io::Error> for ApiError {
    /// Maps a database or filesystem failure to a client-safe internal API error while logging details.
    fn from(value: std::io::Error) -> Self {
        error!(error = %value, "filesystem operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            constants::ERROR_LOCAL_STORAGE,
        )
    }
}

#[derive(Debug, Serialize, FromRow)]
/// SQLite-backed incident case record returned by case endpoints.
struct CaseRecord {
    id: String,
    name: String,
    display_id: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, FromRow, Clone)]
/// SQLite-backed metadata for an uploaded local artifact.
pub(crate) struct ArtifactRecord {
    pub(crate) id: String,
    pub(crate) case_id: String,
    pub(crate) filename: String,
    pub(crate) kind: String,
    pub(crate) product: String,
    pub(crate) media_type: String,
    pub(crate) size_bytes: i64,
    pub(crate) sha256: String,
    pub(crate) uploaded_at: String,
}

#[derive(Deserialize)]
/// Request schema for creating an incident case.
struct CreateCaseRequest {
    name: String,
}

#[derive(Deserialize)]
/// Request schema for changing an incident case name.
struct UpdateCaseRequest {
    name: String,
}

#[derive(Serialize)]
/// Response schema for the API health endpoint.
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[tokio::main]
/// Bootstraps the API process, private storage, SQLite pool, diagnostic worker, middleware, routes, and listener.
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(constants::DEFAULT_LOG_FILTER)),
        )
        .with_target(false)
        .init();

    let config = Arc::new(AppConfig::load()?);
    let data_dir = config.resolve_data_dir();
    fs::create_dir_all(&data_dir).await?;
    fs::create_dir_all(data_dir.join(constants::ARTIFACTS_DIR)).await?;
    fs::create_dir_all(data_dir.join(constants::WORK_DIR)).await?;
    fs::create_dir_all(data_dir.join(constants::REPORTS_DIR)).await?;
    set_private_dir(&data_dir)?;
    set_private_dir(&data_dir.join(constants::ARTIFACTS_DIR))?;
    set_private_dir(&data_dir.join(constants::WORK_DIR))?;
    set_private_dir(&data_dir.join(constants::REPORTS_DIR))?;

    let database_path = data_dir.join(&config.storage.database_filename);
    let sqlite_options = SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(config.database.busy_timeout_seconds));
    let pool = SqlitePoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect_with(sqlite_options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    set_private_file(&database_path)?;
    cleanup_partial_uploads(&data_dir.join(constants::ARTIFACTS_DIR)).await?;

    let max_upload_bytes = config.storage.max_upload_bytes;
    let state = AppState {
        pool,
        data_dir: data_dir.clone(),
        max_upload_bytes,
        config: Arc::clone(&config),
    };
    tokio::spawn(diagnostic_jobs::run_diagnostic_job_worker(state.clone()));

    let bind_addr: SocketAddr = config.server.bind_addr.parse()?;
    let origin = HeaderValue::from_str(&config.server.web_origin)?;
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::ACCEPT]);

    let app = Router::new()
        .route(constants::HEALTH_ROUTE, get(health))
        .route(
            constants::KNOWLEDGE_SEARCH_ROUTE,
            post(knowledge::search_knowledge),
        )
        .route(constants::CASES_ROUTE, get(list_cases).post(create_case))
        .route(
            constants::CASE_ROUTE,
            get(get_case).patch(update_case).delete(delete_case),
        )
        .route(
            constants::CASE_ARTIFACTS_ROUTE,
            get(list_artifacts)
                .post(upload_artifact)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(constants::ARTIFACT_ROUTE, delete(delete_artifact))
        .route(
            constants::DIAGNOSTIC_PREPARE_ROUTE,
            post(prepare_diagnostics),
        )
        // Only the multipart upload route disables Axum's small default body limit;
        // it enforces ATLAS_MAX_UPLOAD_BYTES while streaming the file to disk.
        .route(
            constants::HEAP_OPERATION_ROUTE,
            post(diagnostic_jobs::run_heap_operation),
        )
        .route(
            "/api/v1/diagnostic-jobs/{job_id}",
            get(diagnostic_jobs::get_diagnostic_job),
        )
        .route(
            "/api/v1/diagnostic-jobs/{job_id}/cancel",
            post(diagnostic_jobs::cancel_diagnostic_job),
        )
        .route(constants::AGENT_CHAT_ROUTE, post(agent::agent_chat))
        .route(
            constants::AGENT_MESSAGES_ROUTE,
            get(agent::list_agent_messages),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, data_dir = %data_dir.display(), max_upload_bytes, application = %config.application.name, "API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Applies owner-only directory permissions on Unix platforms.
fn set_private_dir(path: &FsPath) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

/// Applies owner-only file permissions on Unix platforms.
pub(crate) fn set_private_file(path: &FsPath) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

/// Recursively removes incomplete upload files left by interrupted requests.
async fn cleanup_partial_uploads(root: &FsPath) -> std::io::Result<()> {
    let mut entries = match fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let kind = entry.file_type().await?;
        if kind.is_dir() {
            Box::pin(cleanup_partial_uploads(&path)).await?;
        } else if entry.file_name().to_string_lossy().ends_with(".part") {
            let _ = fs::remove_file(path).await;
        }
    }
    Ok(())
}

/// Creates a human-readable incident reference from the case name and internal identifier.
fn readable_incident_id(name: &str, internal_id: &str) -> String {
    let normalized = name.to_ascii_lowercase();
    let category = if ["heap", "memory", "leak", "oom", "outofmemory"]
        .iter()
        .any(|token| normalized.contains(token))
    {
        "HEAP-MEMORY"
    } else if ["thread", "deadlock", "blocked", "lock"]
        .iter()
        .any(|token| normalized.contains(token))
    {
        "THREAD-LOCK"
    } else if normalized.contains("kafka") || normalized.contains("mirrormaker") {
        "KAFKA"
    } else if normalized.contains("artemis") || normalized.contains("activemq") {
        "ARTEMIS"
    } else if normalized.contains("openshift") || normalized.contains("must-gather") {
        "OPENSHIFT"
    } else {
        "INCIDENT"
    };
    let suffix: String = internal_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(10)
        .collect::<String>()
        .to_ascii_uppercase();
    format!("ATLAS-{category}-{suffix}")
}

/// Returns the API health state and compiled application version.
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: constants::HEALTH_STATUS_OK,
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Lists persisted incident cases from SQLite.
async fn list_cases(State(state): State<AppState>) -> Result<Json<Vec<CaseRecord>>, ApiError> {
    let records = sqlx::query_as::<_, CaseRecord>(queries::LIST_CASES)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(records))
}

/// Validates and persists a new incident case with a readable display reference.
async fn create_case(
    State(state): State<AppState>,
    Json(input): Json<CreateCaseRequest>,
) -> Result<(StatusCode, Json<CaseRecord>), ApiError> {
    let name = validate_case_name(&input.name, state.config.application.max_case_name_chars)?;
    let id = Uuid::new_v4().to_string();
    let display_id = readable_incident_id(&name, &id);
    let record = sqlx::query_as::<_, CaseRecord>(queries::INSERT_CASE)
        .bind(id)
        .bind(name)
        .bind(display_id)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(record)))
}

/// Loads one case by UUID or returns the standard not-found response.
async fn get_case(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
) -> Result<Json<CaseRecord>, ApiError> {
    require_uuid(&case_id)?;
    let record = sqlx::query_as::<_, CaseRecord>(queries::GET_CASE)
        .bind(case_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(record))
}

/// Validates and updates the display name of an existing case.
async fn update_case(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
    Json(input): Json<UpdateCaseRequest>,
) -> Result<Json<CaseRecord>, ApiError> {
    require_uuid(&case_id)?;
    let name = validate_case_name(&input.name, state.config.application.max_case_name_chars)?;
    let record = sqlx::query_as::<_, CaseRecord>(queries::UPDATE_CASE)
        .bind(name)
        .bind(case_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(record))
}

/// Deletes an idle case and cleans up its case-scoped local directories.
async fn delete_case(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_uuid(&case_id)?;
    let active_jobs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM diagnostic_jobs WHERE case_id=? AND status IN ('queued','running')",
    )
    .bind(&case_id)
    .fetch_one(&state.pool)
    .await?;
    if active_jobs > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This case has queued or running diagnostics. Cancel or wait for those jobs to finish before deleting the case.",
        ));
    }

    // Delete the database row first. The schema's ON DELETE CASCADE relationships
    // remove artifacts, findings, tool runs, and agent messages in the same DB operation.
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(queries::DELETE_CASE)
        .bind(&case_id)
        .execute(&mut *transaction)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    transaction.commit().await?;

    // Only case-scoped directories are removed. If filesystem cleanup fails after
    // the database delete, log it so the API does not misreport the delete as failed.
    for bucket in [constants::ARTIFACTS_DIR, constants::WORK_DIR] {
        let path = state.data_dir.join(bucket).join(&case_id);
        if let Err(cleanup_error) = fs::remove_dir_all(&path).await
            && cleanup_error.kind() != std::io::ErrorKind::NotFound
        {
            error!(
                case_id = %case_id,
                path = %path.display(),
                error = %cleanup_error,
                "Case was deleted from SQLite but local directory cleanup failed"
            );
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Lists uploaded artifacts belonging to a validated case.
async fn list_artifacts(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
) -> Result<Json<Vec<ArtifactRecord>>, ApiError> {
    require_uuid(&case_id)?;
    ensure_case_exists(&state.pool, &case_id).await?;
    let records = sqlx::query_as::<_, ArtifactRecord>(queries::LIST_ARTIFACTS)
        .bind(case_id)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(records))
}

/// Streams one bounded upload to private storage while computing its SHA-256 digest.
async fn upload_artifact(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ArtifactRecord>), ApiError> {
    require_uuid(&case_id)?;
    ensure_case_exists(&state.pool, &case_id).await?;
    let mut field = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad_request(constants::ERROR_MALFORMED_MULTIPART))?
        .ok_or_else(|| ApiError::bad_request(constants::ERROR_EXPECTED_FILE_FIELD))?;
    if field.name() != Some(constants::MULTIPART_FILE_FIELD) {
        return Err(ApiError::bad_request(
            constants::ERROR_WRONG_MULTIPART_FIELD,
        ));
    }

    let filename = safe_filename(
        field
            .file_name()
            .unwrap_or(constants::DEFAULT_UPLOAD_FILENAME),
    );
    let media_type = mime_guess::from_path(&filename)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();
    let (kind, product) = state.config.classify_artifact(&filename);
    let kind = kind.to_owned();
    let product = product.to_owned();
    let artifact_id = Uuid::new_v4().to_string();
    let artifact_dir = state
        .data_dir
        .join(constants::ARTIFACTS_DIR)
        .join(&case_id)
        .join(&artifact_id);
    fs::create_dir_all(&artifact_dir).await?;
    set_private_dir(&artifact_dir)?;
    let temp_path = artifact_dir.join(constants::PARTIAL_ARTIFACT_FILENAME);
    let final_path = artifact_dir.join(constants::STORED_ARTIFACT_FILENAME);
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .await?;

    let write_result: Result<(u64, String), ApiError> = async {
        let mut size: u64 = 0;
        let mut hasher = Sha256::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|_| ApiError::bad_request(constants::ERROR_INTERRUPTED_UPLOAD))?
        {
            size = size.saturating_add(chunk.len() as u64);
            if size > state.max_upload_bytes {
                return Err(ApiError::too_large(state.max_upload_bytes));
            }
            hasher.update(&chunk);
            output.write_all(&chunk).await.map_err(ApiError::from)?;
        }
        output.flush().await.map_err(ApiError::from)?;
        output.sync_all().await.map_err(ApiError::from)?;
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok((size, digest))
    }
    .await;
    drop(output);

    let (size_bytes, sha256) = match write_result {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&artifact_dir).await;
            return Err(error);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    fs::rename(&temp_path, &final_path).await?;

    let artifact = async {
        let mut tx = state.pool.begin().await?;
        let record = sqlx::query_as::<_, ArtifactRecord>(queries::INSERT_ARTIFACT)
            .bind(&artifact_id)
            .bind(&case_id)
            .bind(&filename)
            .bind(&kind)
            .bind(&product)
            .bind(&media_type)
            .bind(size_bytes as i64)
            .bind(&sha256)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(queries::TOUCH_CASE)
            .bind(&case_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok::<ArtifactRecord, sqlx::Error>(record)
    }
    .await;

    match artifact {
        Ok(record) => Ok((StatusCode::CREATED, Json(record))),
        Err(error) => {
            let _ = fs::remove_dir_all(&artifact_dir).await;
            Err(ApiError::from(error))
        }
    }
}

#[derive(Deserialize)]
/// Request schema selecting diagnostic artifacts and describing the incident.
struct PrepareDiagnosticsRequest {
    problem_description: String,
    #[serde(default)]
    artifact_ids: Vec<String>,
}

#[derive(Serialize)]
/// Response schema describing prepared diagnostic plugins and their execution state.
pub(crate) struct PrepareDiagnosticsResponse {
    pub(crate) case_id: String,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) preparations: Vec<diagnostic_plugins::PreparedPlugin>,
}

/// Validates selected artifacts and prepares the configured local diagnostic plugins.
async fn prepare_diagnostics(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
    Json(input): Json<PrepareDiagnosticsRequest>,
) -> Result<Json<PrepareDiagnosticsResponse>, ApiError> {
    require_uuid(&case_id)?;
    ensure_case_exists(&state.pool, &case_id).await?;
    let problem_description = input.problem_description.trim().to_owned();
    if problem_description.is_empty() {
        return Err(ApiError::bad_request(
            "Problem description cannot be empty.",
        ));
    }
    if problem_description.chars().count() > 20_000 {
        return Err(ApiError::bad_request(
            "Problem description must be 20,000 characters or fewer.",
        ));
    }
    if input.artifact_ids.len() > 20 {
        return Err(ApiError::bad_request(
            "Prepare at most 20 diagnostic artifacts per request.",
        ));
    }

    let all_artifacts = sqlx::query_as::<_, ArtifactRecord>(queries::LIST_ARTIFACTS)
        .bind(&case_id)
        .fetch_all(&state.pool)
        .await?;
    let mut requested_ids = Vec::new();
    for artifact_id in input.artifact_ids {
        Uuid::parse_str(&artifact_id)
            .map_err(|_| ApiError::bad_request("Each artifact_id must be a valid UUID."))?;
        if !requested_ids.contains(&artifact_id) {
            requested_ids.push(artifact_id);
        }
    }

    let mut selected_artifacts = Vec::new();
    if requested_ids.is_empty() {
        selected_artifacts.extend(
            all_artifacts
                .iter()
                .filter(|artifact| matches!(artifact.kind.as_str(), "heap_dump" | "thread_dump"))
                .cloned(),
        );
    } else {
        for artifact_id in &requested_ids {
            let artifact = all_artifacts
                .iter()
                .find(|artifact| artifact.id == *artifact_id)
                .ok_or_else(ApiError::not_found)?;
            if !matches!(artifact.kind.as_str(), "heap_dump" | "thread_dump") {
                return Err(ApiError::bad_request(format!(
                    "Artifact '{}' has kind '{}'; only heap_dump and thread_dump are supported by these plugins.",
                    artifact.filename, artifact.kind
                )));
            }
            selected_artifacts.push(artifact.clone());
        }
    }

    if selected_artifacts.is_empty() {
        return Err(ApiError::bad_request(
            "No heap dump or thread dump artifacts are available for this case. Save and upload a .hprof/.phd heap dump or a recognized thread dump first.",
        ));
    }

    let mut preparations = Vec::with_capacity(selected_artifacts.len());
    for artifact in selected_artifacts {
        let _heap_artifact_guard = if artifact.kind == "heap_dump" {
            Some(
                diagnostic_plugins::heap_artifact_lock(&case_id, &artifact.id)
                    .lock_owned()
                    .await,
            )
        } else {
            None
        };
        let tool_run_id = Uuid::new_v4().to_string();
        let mut prepared = diagnostic_plugins::prepare(
            &state.data_dir,
            &case_id,
            &tool_run_id,
            &problem_description,
            &artifact.id,
            &artifact.kind,
            &artifact.filename,
            artifact.size_bytes,
            &artifact.sha256,
        )
        .await
        .map_err(|message| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message))?;

        if artifact.kind == "heap_dump" {
            let job_id = prepared.tool_run_id.clone();
            let request = diagnostic_jobs::HeapOperationRequest {
                problem_description: Some(problem_description.clone()),
                artifact_id: Some(artifact.id.clone()),
                class_filter: None,
                top_n: Some(50),
                object_id: None,
                baseline_id: None,
                job_id: Some(job_id.clone()),
            };
            diagnostic_jobs::enqueue_diagnostic_job(
                &state,
                &job_id,
                &case_id,
                Some(&artifact.id),
                "analyze_heap_dump",
                &request,
            )
            .await?;
            prepared.job_id = Some(job_id);
            prepared.status = "queued".to_owned();
            prepared.summary = "Heap dump staged; Eclipse MAT analysis has been queued. Poll the job status to follow execution.".to_owned();
            prepared.warnings.push("MAT execution is asynchronous; the final result is available when this diagnostic job completes.".to_owned());
        }
        let input_json = serde_json::json!({
            "schema_version": "1",
            "problem_description": problem_description,
            "artifact_id": artifact.id,
            "artifact_kind": artifact.kind,
            "artifact_filename": artifact.filename,
        })
        .to_string();
        let output_json = serde_json::to_string(&prepared).map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not encode plugin result: {error}"),
            )
        })?;
        // Heap jobs persist their tool_run when the worker completes; the queue row
        // is the authoritative status while work is pending. Thread preparation is inline.
        if artifact.kind != "heap_dump" {
            sqlx::query(
                "INSERT INTO tool_runs (id, case_id, artifact_id, tool_id, tool_version, status, input_json, output_json, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(id) DO UPDATE SET tool_id=excluded.tool_id, status=excluded.status, input_json=excluded.input_json, output_json=excluded.output_json, started_at=excluded.started_at, finished_at=excluded.finished_at"
            )
            .bind(&prepared.tool_run_id)
            .bind(&case_id)
            .bind(&artifact.id)
            .bind(&prepared.tool_id)
            .bind(&prepared.tool_version)
            .bind(&prepared.status)
            .bind(input_json)
            .bind(output_json)
            .execute(&state.pool)
            .await?;
        }
        preparations.push(prepared);
    }

    Ok(Json(PrepareDiagnosticsResponse {
        case_id,
        status: "prepared".to_owned(),
        message: "Local diagnostic preparation completed. Thread dumps were summarized locally; heap dumps are analyzed with fixed Eclipse MAT operations when a usable Docker or Podman runtime is available. When enabled, bounded local diagnostic context can be attached to the direct local Ollama chat request; raw heap dumps and MAT indexes are excluded.".to_owned(),
        preparations,
    }))
}

/// Validates and executes one supported MAT heap operation synchronously.
pub(crate) async fn run_heap_operation_now(
    State(state): State<AppState>,
    Path((case_id, operation)): Path<(String, String)>,
    Json(input): Json<diagnostic_jobs::HeapOperationRequest>,
) -> Result<Json<PrepareDiagnosticsResponse>, ApiError> {
    require_uuid(&case_id)?;
    ensure_case_exists(&state.pool, &case_id).await?;
    let operation_allowed = matches!(
        operation.as_str(),
        "analyze_heap_dump"
            | "generate_leak_suspects"
            | "generate_heap_overview"
            | "get_histogram"
            | "get_dominator_tree"
            | "find_paths_to_gc_roots"
            | "compare_heap_dumps"
    );
    if !operation_allowed {
        return Err(ApiError::bad_request(
            "Unsupported heap operation. Use analyze_heap_dump, generate_leak_suspects, generate_heap_overview, get_histogram, get_dominator_tree, find_paths_to_gc_roots, or compare_heap_dumps.",
        ));
    }
    if let Some(id) = input.artifact_id.as_deref() {
        Uuid::parse_str(id)
            .map_err(|_| ApiError::bad_request("artifact_id must be a valid UUID."))?;
    }
    if let Some(id) = input.baseline_id.as_deref() {
        Uuid::parse_str(id)
            .map_err(|_| ApiError::bad_request("baseline_id must be a valid UUID."))?;
    }
    let records = sqlx::query_as::<_, ArtifactRecord>(queries::LIST_ARTIFACTS)
        .bind(&case_id)
        .fetch_all(&state.pool)
        .await?;
    let baseline_record = if operation == "compare_heap_dumps" {
        let baseline_id = input.baseline_id.as_ref().ok_or_else(|| {
            ApiError::bad_request("baseline_id is required for compare_heap_dumps.")
        })?;
        Some(
            records
                .iter()
                .find(|record| record.id == *baseline_id && record.kind == "heap_dump")
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
    let primary = if let Some(id) = input.artifact_id.as_ref() {
        records
            .iter()
            .find(|record| record.id == *id && record.kind == "heap_dump")
            .cloned()
            .ok_or_else(ApiError::not_found)?
    } else {
        records
            .iter()
            .find(|record| {
                record.kind == "heap_dump"
                    && baseline_record
                        .as_ref()
                        .is_none_or(|baseline| baseline.id != record.id)
            })
            .cloned()
            .ok_or_else(|| ApiError::bad_request("No saved heap dump is available in this case."))?
    };
    if baseline_record
        .as_ref()
        .is_some_and(|baseline| baseline.id == primary.id)
    {
        return Err(ApiError::bad_request(
            "artifact_id and baseline_id must refer to different heap dumps.",
        ));
    }
    let problem = input
        .problem_description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Additional heap analysis requested for this case.")
        .to_owned();
    if problem.chars().count() > 20_000 {
        return Err(ApiError::bad_request(
            "problem_description must be 20,000 characters or fewer.",
        ));
    }
    let _heap_artifact_guard = diagnostic_plugins::heap_artifact_lock(&case_id, &primary.id)
        .lock_owned()
        .await;
    let tool_run_id = input
        .job_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut prepared = diagnostic_plugins::prepare(
        &state.data_dir,
        &case_id,
        &tool_run_id,
        &problem,
        &primary.id,
        &primary.kind,
        &primary.filename,
        primary.size_bytes,
        &primary.sha256,
    )
    .await
    .map_err(|message| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message))?;
    prepared.job_id = input.job_id.clone();
    let baseline = baseline_record.map(|record| diagnostic_plugins::HeapBaseline {
        artifact_id: record.id,
        filename: record.filename,
        sha256: record.sha256,
    });
    if let Err(message) = diagnostic_plugins::execute_heap_operation(
        &state.data_dir,
        &case_id,
        &mut prepared,
        &operation,
        input.class_filter.as_deref(),
        input.top_n.unwrap_or(50),
        input.object_id.as_deref(),
        baseline,
    )
    .await
    {
        prepared.status = "failed".to_owned();
        prepared.summary = format!("MAT operation '{operation}' failed.");
        prepared.warnings.push(message);
        let _ = diagnostic_plugins::record_mat_failure(
            &state.data_dir,
            &case_id,
            &mut prepared,
            &operation,
        )
        .await;
    }
    let input_json = serde_json::json!({
        "schema_version":"1", "operation":operation, "problem_description":problem,
        "artifact_id":primary.id, "class_filter":input.class_filter, "top_n":input.top_n.unwrap_or(50),
        "object_id":input.object_id, "baseline_id":input.baseline_id,
    }).to_string();
    let output_json = serde_json::to_string(&prepared).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not encode MAT result: {error}"),
        )
    })?;
    sqlx::query("INSERT INTO tool_runs (id, case_id, artifact_id, tool_id, tool_version, status, input_json, output_json, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(id) DO UPDATE SET tool_id=excluded.tool_id, status=excluded.status, input_json=excluded.input_json, output_json=excluded.output_json, started_at=excluded.started_at, finished_at=excluded.finished_at")
        .bind(&prepared.tool_run_id).bind(&case_id).bind(&primary.id).bind(&prepared.tool_id)
        .bind(&prepared.tool_version).bind(&prepared.status).bind(input_json).bind(output_json)
        .execute(&state.pool).await?;
    let status = prepared.status.clone();
    let message = if status == "succeeded" {
        format!(
            "MAT operation '{operation}' completed. Review llm-context.md and the report files returned by the tool."
        )
    } else {
        format!(
            "MAT operation '{operation}' finished with status '{status}'. Review warnings and execution files."
        )
    };
    Ok(Json(PrepareDiagnosticsResponse {
        case_id,
        status,
        message,
        preparations: vec![prepared],
    }))
}

/// Removes an artifact and its derived MAT workspace after active jobs have stopped.
async fn delete_artifact(
    State(state): State<AppState>,
    Path((case_id, artifact_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_uuid(&case_id)?;
    require_uuid(&artifact_id)?;
    let active_jobs = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM diagnostic_jobs WHERE case_id=? AND (artifact_id=? OR baseline_artifact_id=?) AND status IN ('queued','running')")
        .bind(&case_id).bind(&artifact_id).bind(&artifact_id).fetch_one(&state.pool).await?;
    if active_jobs > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This artifact has queued or running diagnostic jobs. Cancel or wait for those jobs to finish before deleting the artifact.",
        ));
    }
    let exists = sqlx::query_scalar::<_, i64>(queries::ARTIFACT_EXISTS)
        .bind(&artifact_id)
        .bind(&case_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found());
    }
    let artifact_dir = state
        .data_dir
        .join(constants::ARTIFACTS_DIR)
        .join(&case_id)
        .join(&artifact_id);
    if let Err(error) = fs::remove_dir_all(&artifact_dir).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(ApiError::from(error));
    }
    // MAT may have created an independent copy plus large index/cache files.
    // Keep the database record if cleanup fails so deletion can be retried.
    let derived_workspace = state
        .data_dir
        .join(constants::WORK_DIR)
        .join(&case_id)
        .join("heap")
        .join(&artifact_id);
    if let Err(error) = fs::remove_dir_all(&derived_workspace).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        error!(case_id = %case_id, artifact_id = %artifact_id, error = %error, "could not remove derived heap-analysis workspace; retry artifact deletion after resolving the filesystem issue");
        return Err(ApiError::from(error));
    }
    sqlx::query(queries::DELETE_ARTIFACT)
        .bind(&artifact_id)
        .bind(&case_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks that a case exists before a case-scoped operation proceeds.
pub(crate) async fn ensure_case_exists(pool: &SqlitePool, case_id: &str) -> Result<(), ApiError> {
    let exists = sqlx::query_scalar::<_, i64>(queries::CASE_EXISTS)
        .bind(case_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

/// Validates an externally supplied UUID and maps invalid identifiers to not-found.
pub(crate) fn require_uuid(value: &str) -> Result<(), ApiError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ApiError::not_found())
}

/// Trims and bounds a user-supplied case name.
fn validate_case_name(input: &str, max_chars: usize) -> Result<String, ApiError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(constants::ERROR_EMPTY_CASE_NAME));
    }
    if value.chars().count() > max_chars {
        return Err(ApiError::bad_request(constants::case_name_too_long(
            max_chars,
        )));
    }
    Ok(value.to_owned())
}

/// Strips client path components and control characters from an uploaded filename.
fn safe_filename(input: &str) -> String {
    let normalized = input.replace('\\', "/");
    let leaf = normalized
        .rsplit('/')
        .next()
        .unwrap_or(constants::DEFAULT_UPLOAD_FILENAME);
    let safe: String = leaf
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect();
    let safe = safe.trim();
    if safe.is_empty() || safe == "." || safe == ".." {
        constants::DEFAULT_UPLOAD_FILENAME.to_owned()
    } else {
        safe.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{safe_filename, validate_case_name};

    #[test]
    /// Verifies client-controlled directory components are removed from uploaded filenames.
    fn filename_never_keeps_client_directories() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename(r"C:\\temp\\heap.hprof"), "heap.hprof");
        assert_eq!(safe_filename(".."), "upload.bin");
    }

    #[test]
    /// Verifies case-name trimming and empty-name validation.
    fn validates_case_names() {
        assert!(validate_case_name("  broker outage  ", 140).is_ok());
        assert!(validate_case_name("  ", 140).is_err());
    }
}
