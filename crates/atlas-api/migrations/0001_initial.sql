PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS cases (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_cases_updated_at ON cases(updated_at DESC);

CREATE TABLE IF NOT EXISTS artifacts (
    id          TEXT PRIMARY KEY NOT NULL,
    case_id     TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    filename    TEXT NOT NULL,
    kind        TEXT NOT NULL,
    media_type  TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256      TEXT NOT NULL,
    uploaded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(case_id, id)
);
CREATE INDEX IF NOT EXISTS idx_artifacts_case_uploaded ON artifacts(case_id, uploaded_at DESC);
CREATE INDEX IF NOT EXISTS idx_artifacts_sha256 ON artifacts(sha256);

CREATE TABLE IF NOT EXISTS analysis_runs (
    id          TEXT PRIMARY KEY NOT NULL,
    case_id     TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'queued',
    trigger     TEXT NOT NULL DEFAULT 'manual',
    summary     TEXT,
    started_at  TEXT,
    finished_at TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_analysis_runs_case_created ON analysis_runs(case_id, created_at DESC);

CREATE TABLE IF NOT EXISTS findings (
    id              TEXT PRIMARY KEY NOT NULL,
    case_id         TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    analysis_run_id TEXT REFERENCES analysis_runs(id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    severity        TEXT NOT NULL DEFAULT 'unknown',
    status          TEXT NOT NULL DEFAULT 'hypothesis',
    confidence      REAL,
    summary         TEXT,
    hypothesis      TEXT,
    verified        INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_findings_case_created ON findings(case_id, created_at DESC);

CREATE TABLE IF NOT EXISTS evidence_refs (
    id              TEXT PRIMARY KEY NOT NULL,
    finding_id      TEXT NOT NULL REFERENCES findings(id) ON DELETE CASCADE,
    artifact_id     TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    location_type   TEXT NOT NULL,
    locator         TEXT NOT NULL,
    excerpt         TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_evidence_finding ON evidence_refs(finding_id);

CREATE TABLE IF NOT EXISTS tool_runs (
    id          TEXT PRIMARY KEY NOT NULL,
    case_id     TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    tool_id     TEXT NOT NULL,
    tool_version TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'queued',
    input_json  TEXT NOT NULL DEFAULT '{}',
    output_json TEXT,
    started_at  TEXT,
    finished_at TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_tool_runs_case_created ON tool_runs(case_id, created_at DESC);
