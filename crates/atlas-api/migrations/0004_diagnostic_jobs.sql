-- Persistent queue for long-running MAT diagnostics.
CREATE TABLE IF NOT EXISTS diagnostic_jobs (
    id               TEXT PRIMARY KEY NOT NULL,
    case_id          TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    artifact_id      TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    baseline_artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    operation        TEXT NOT NULL,
    request_json     TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'queued'
                     CHECK (status IN ('queued','running','succeeded','failed','cancelled')),
    progress         INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    stage            TEXT NOT NULL DEFAULT 'Queued',
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0,1)),
    error            TEXT,
    result_json      TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at       TEXT,
    finished_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_diagnostic_jobs_claim
    ON diagnostic_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS idx_diagnostic_jobs_case
    ON diagnostic_jobs(case_id, created_at DESC);
