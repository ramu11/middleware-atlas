-- Persist bounded online research references with the assistant answer they informed.
-- Existing conversation rows remain valid and return without research metadata.
ALTER TABLE agent_messages
    ADD COLUMN research_json TEXT
    CHECK (research_json IS NULL OR length(research_json) <= 32768);
