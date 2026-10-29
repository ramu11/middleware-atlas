//! Named SQL statements used by the application API.
//! Keep SQL here so query changes are easy to review and do not clutter handlers.

pub const LIST_CASES: &str = "SELECT id, name, display_id, created_at, updated_at FROM cases ORDER BY updated_at DESC LIMIT 200";
pub const INSERT_CASE: &str = "INSERT INTO cases (id, name, display_id) VALUES (?, ?, ?) RETURNING id, name, display_id, created_at, updated_at";
pub const GET_CASE: &str =
    "SELECT id, name, display_id, created_at, updated_at FROM cases WHERE id = ?";
pub const UPDATE_CASE: &str = "UPDATE cases SET name = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? RETURNING id, name, display_id, created_at, updated_at";
pub const LIST_ARTIFACTS: &str = "SELECT id, case_id, filename, kind, product, media_type, size_bytes, sha256, uploaded_at FROM artifacts WHERE case_id = ? ORDER BY uploaded_at DESC";
pub const INSERT_ARTIFACT: &str = "INSERT INTO artifacts (id, case_id, filename, kind, product, media_type, size_bytes, sha256) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id, case_id, filename, kind, product, media_type, size_bytes, sha256, uploaded_at";
pub const TOUCH_CASE: &str =
    "UPDATE cases SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?";
pub const ARTIFACT_EXISTS: &str = "SELECT 1 FROM artifacts WHERE id = ? AND case_id = ?";
pub const DELETE_ARTIFACT: &str = "DELETE FROM artifacts WHERE id = ? AND case_id = ?";
pub const CASE_EXISTS: &str = "SELECT 1 FROM cases WHERE id = ?";
