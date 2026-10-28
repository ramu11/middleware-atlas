# Configuration and local data layout

Middleware Atlas loads `config/default.toml` at startup. This file contains non-secret application settings and the artifact/product classification rules. Environment variables override supported settings; secrets such as the Dify API key stay in the local `.env` file and are never placed in TOML or committed.

## Project-relative paths

By default the data directory is `<project-root>/data`, containing:

```text
data/
├── middleware-atlas.db
├── artifacts/<case-id>/<artifact-id>/original
├── work/
└── reports/
```

Relative paths in `storage.data_dir` and `ATLAS_DATA_DIR` are always resolved from the project root, independently of the directory from which `cargo run` was launched. Absolute paths and `~/...` paths are also supported. `ATLAS_PROJECT_DIR` can override the detected project root, but must be absolute. `ATLAS_CONFIG_FILE` can select another TOML file; relative config-file paths are also anchored to the project root.

## Supported environment overrides

- `ATLAS_PROJECT_DIR`: absolute project root, optional.
- `ATLAS_CONFIG_FILE`: TOML config path, optional.
- `ATLAS_BIND_ADDR`: listener address; defaults to loopback `127.0.0.1:8080`.
- `ATLAS_WEB_ORIGIN`: allowed browser origin; defaults to `http://localhost:3000`.
- `ATLAS_DATA_DIR`: absolute path, `~/...`, or path relative to project root.
- `ATLAS_MAX_UPLOAD_BYTES`: per-file byte limit.
- `ATLAS_DB_MAX_CONNECTIONS`: SQLite pool upper bound.
- `ATLAS_DB_BUSY_TIMEOUT_SECONDS`: SQLite lock wait timeout.

Environment values override TOML. Blank values in `.env` are treated as unset. Invalid values fail startup instead of silently falling back to defaults.

## Custom artifact classification

Add or reorder `[[diagnostics.artifact_rules]]` entries in TOML. Rules are evaluated top-to-bottom. `match_mode = "any"` matches any filename token or extension; `match_mode = "all"` requires both a configured token and an extension. Keep specific products before generic file/archive rules. `kind` remains a stable machine-oriented classification; `product` is the human-readable product label exposed by the API.

## Data migration note

Changing the default path does not move an existing database automatically. Stop the API before copying the old local data to the project directory. On Linux, if you need to keep the previously created store, use this from the repository root:

```bash
mkdir -p data
cp -a "$HOME/.local/share/middleware-atlas/." data/
```

Do this only while the API is stopped so SQLite's database and WAL files are copied consistently. Do not commit `data/`; it contains case metadata and potentially sensitive customer artifacts.
