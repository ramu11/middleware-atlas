-- Product classification is stored separately from the stable machine-oriented artifact kind.
ALTER TABLE artifacts ADD COLUMN product TEXT NOT NULL DEFAULT 'Other / Unclassified';
CREATE INDEX IF NOT EXISTS idx_artifacts_product ON artifacts(product);
