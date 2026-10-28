mod config;
mod constants;
mod queries;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get},
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
struct AppState {
    pool: SqlitePool,
    data_dir: PathBuf,
    max_upload_bytes: u64,
    config: Arc<AppConfig>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, constants::ERROR_NOT_FOUND)
    }
    fn too_large(limit: u64) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            constants::upload_too_large(limit),
        )
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
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
    fn from(value: sqlx::Error) -> Self {
        error!(error = %value, "database operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            constants::ERROR_PERSISTENCE,
        )
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        error!(error = %value, "filesystem operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            constants::ERROR_LOCAL_STORAGE,
        )
    }
}

#[derive(Debug, Serialize, FromRow)]
struct CaseRecord {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, FromRow)]
struct ArtifactRecord {
    id: String,
    case_id: String,
    filename: String,
    kind: String,
    product: String,
    media_type: String,
    size_bytes: i64,
    sha256: String,
    uploaded_at: String,
}

#[derive(Deserialize)]
struct CreateCaseRequest {
    name: String,
}

#[derive(Deserialize)]
struct UpdateCaseRequest {
    name: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[tokio::main]
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
        .route(constants::CASES_ROUTE, get(list_cases).post(create_case))
        .route(constants::CASE_ROUTE, get(get_case).patch(update_case))
        .route(
            constants::CASE_ARTIFACTS_ROUTE,
            get(list_artifacts)
                .post(upload_artifact)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(constants::ARTIFACT_ROUTE, delete(delete_artifact))
        // Only the multipart upload route disables Axum's small default body limit;
        // it enforces ATLAS_MAX_UPLOAD_BYTES while streaming the file to disk.
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, data_dir = %data_dir.display(), max_upload_bytes, application = %config.application.name, "API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn set_private_dir(path: &FsPath) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn set_private_file(path: &FsPath) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

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

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: constants::HEALTH_STATUS_OK,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_cases(State(state): State<AppState>) -> Result<Json<Vec<CaseRecord>>, ApiError> {
    let records = sqlx::query_as::<_, CaseRecord>(queries::LIST_CASES)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(records))
}

async fn create_case(
    State(state): State<AppState>,
    Json(input): Json<CreateCaseRequest>,
) -> Result<(StatusCode, Json<CaseRecord>), ApiError> {
    let name = validate_case_name(&input.name, state.config.application.max_case_name_chars)?;
    let id = Uuid::new_v4().to_string();
    let record = sqlx::query_as::<_, CaseRecord>(queries::INSERT_CASE)
        .bind(id)
        .bind(name)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(record)))
}

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

async fn delete_artifact(
    State(state): State<AppState>,
    Path((case_id, artifact_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_uuid(&case_id)?;
    require_uuid(&artifact_id)?;
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
    if let Err(error) = fs::remove_dir_all(&artifact_dir).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(ApiError::from(error));
        }
    }
    sqlx::query(queries::DELETE_ARTIFACT)
        .bind(&artifact_id)
        .bind(&case_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ensure_case_exists(pool: &SqlitePool, case_id: &str) -> Result<(), ApiError> {
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

fn require_uuid(value: &str) -> Result<(), ApiError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ApiError::not_found())
}

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
    fn filename_never_keeps_client_directories() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename(r"C:\\temp\\heap.hprof"), "heap.hprof");
        assert_eq!(safe_filename(".."), "upload.bin");
    }

    #[test]
    fn validates_case_names() {
        assert!(validate_case_name("  broker outage  ", 140).is_ok());
        assert!(validate_case_name("  ", 140).is_err());
    }
}
