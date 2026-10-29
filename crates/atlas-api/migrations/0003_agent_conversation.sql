-- Human-readable public incident reference and persistent assistant conversation.
ALTER TABLE cases ADD COLUMN display_id TEXT NOT NULL DEFAULT '';
ALTER TABLE cases ADD COLUMN agent_context_sha256 TEXT;

UPDATE cases
SET display_id = 'ATLAS-' ||
    CASE
        WHEN lower(name) LIKE '%heap%' OR lower(name) LIKE '%memory%' OR lower(name) LIKE '%leak%' OR lower(name) LIKE '%outofmemory%' THEN 'HEAP-MEMORY'
        WHEN lower(name) LIKE '%thread%' OR lower(name) LIKE '%deadlock%' OR lower(name) LIKE '%blocked%' OR lower(name) LIKE '%lock%' THEN 'THREAD-LOCK'
        WHEN lower(name) LIKE '%kafka%' OR lower(name) LIKE '%mirrormaker%' THEN 'KAFKA'
        WHEN lower(name) LIKE '%artemis%' OR lower(name) LIKE '%activemq%' THEN 'ARTEMIS'
        WHEN lower(name) LIKE '%openshift%' OR lower(name) LIKE '%must-gather%' THEN 'OPENSHIFT'
        ELSE 'INCIDENT'
    END || '-' || upper(substr(replace(id, '-', ''), 1, 10))
WHERE display_id = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_cases_display_id ON cases(display_id);

CREATE TABLE IF NOT EXISTS agent_messages (
    id TEXT PRIMARY KEY NOT NULL,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    provider_message_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_agent_messages_case_created
    ON agent_messages(case_id, created_at, id);
