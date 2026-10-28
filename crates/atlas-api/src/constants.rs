//! Stable API paths, storage names, status values and client-facing messages.
//! SQL lives separately in `queries.rs`; product taxonomy lives in `config/default.toml`.

pub const DEFAULT_LOG_FILTER: &str = "atlas_api=info,tower_http=info";

pub const HEALTH_ROUTE: &str = "/health";
pub const CASES_ROUTE: &str = "/api/v1/cases";
pub const CASE_ROUTE: &str = "/api/v1/cases/{case_id}";
pub const CASE_ARTIFACTS_ROUTE: &str = "/api/v1/cases/{case_id}/artifacts";
pub const ARTIFACT_ROUTE: &str = "/api/v1/cases/{case_id}/artifacts/{artifact_id}";

pub const HEALTH_STATUS_OK: &str = "ok";
pub const ARTIFACTS_DIR: &str = "artifacts";
pub const WORK_DIR: &str = "work";
pub const REPORTS_DIR: &str = "reports";
pub const DEFAULT_UPLOAD_FILENAME: &str = "upload.bin";
pub const MULTIPART_FILE_FIELD: &str = "file";
pub const PARTIAL_ARTIFACT_FILENAME: &str = "original.part";
pub const STORED_ARTIFACT_FILENAME: &str = "original";

pub const ERROR_NOT_FOUND: &str = "The requested case or artifact was not found";
pub const ERROR_PERSISTENCE: &str = "A persistence operation failed";
pub const ERROR_LOCAL_STORAGE: &str = "A local storage operation failed";
pub const ERROR_MALFORMED_MULTIPART: &str = "Malformed multipart upload";
pub const ERROR_EXPECTED_FILE_FIELD: &str = "Expected one multipart field named 'file'";
pub const ERROR_WRONG_MULTIPART_FIELD: &str = "Expected the multipart field to be named 'file'";
pub const ERROR_INTERRUPTED_UPLOAD: &str = "The upload stream was interrupted or malformed";
pub const ERROR_EMPTY_CASE_NAME: &str = "Case name cannot be empty";
pub fn upload_too_large(limit: u64) -> String {
    format!("File exceeds the configured upload limit of {limit} bytes")
}

pub fn case_name_too_long(max_chars: usize) -> String {
    format!("Case name must be at most {max_chars} characters")
}
