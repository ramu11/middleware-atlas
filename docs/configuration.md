# Configuration and local data layout

Middleware Atlas reads non-secret defaults from `config/default.toml` and selected environment overrides. Create a repository-root `.env` for local backend settings and keep it untracked. Do not place credentials in TOML or expose them through frontend `NEXT_PUBLIC_*` variables.

## Local data layout

The default data directory is `<project-root>/data`:

```text
data/
├── middleware-atlas.db
├── artifacts/<case-id>/<artifact-id>/original
├── work/<case-id>/...
└── reports/
```

SQLite stores case/artifact metadata, diagnostic tool-run records, readable incident references and agent messages. Uploaded files and generated reports/workspaces live on disk. Do not commit `data/`.

Relative `storage.data_dir` and `ATLAS_DATA_DIR` paths are resolved from the project root, regardless of the shell's current directory. Absolute paths and `~/...` paths are supported. `ATLAS_PROJECT_DIR` can override the detected project root but must be absolute. `ATLAS_CONFIG_FILE` selects another TOML file; relative paths are anchored to the project root.

## Server and persistence environment variables

- `ATLAS_PROJECT_DIR`: absolute project root, optional.
- `ATLAS_CONFIG_FILE`: TOML configuration path, optional.
- `ATLAS_BIND_ADDR`: API listen address; default `127.0.0.1:8080`.
- `ATLAS_WEB_ORIGIN`: allowed frontend origin; default `http://localhost:3000`.
- `ATLAS_DATA_DIR`: data directory, absolute, `~/...`, or project-root-relative.
- `ATLAS_MAX_UPLOAD_BYTES`: per-file upload limit; default 100 GiB.
- `ATLAS_DB_MAX_CONNECTIONS`: SQLite pool upper bound.
- `ATLAS_DB_BUSY_TIMEOUT_SECONDS`: SQLite lock wait timeout.

Environment variables override the corresponding TOML settings. Blank values loaded from `.env` are treated as unset; invalid supported values should be corrected rather than silently assumed to work.

## Local Ollama agent variables

Use these in the repository-root `.env`:

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

- `ATLAS_LLM_BASE_URL`: local Ollama HTTP origin. The backend only accepts plain HTTP loopback hosts such as `127.0.0.1`, `localhost`, or IPv6 loopback. Remote and LAN endpoints are blocked.
- `ATLAS_LLM_MODEL`: name of a model already installed in local Ollama. The code defaults to `qwen3:8b`; model names containing `cloud` are rejected.
- `ATLAS_LLM_SEND_HEAP_CONTEXT`: attach bounded local diagnostic context to new agent chat requests. Defaults to enabled (`true`) when unset. Set to `false` to keep report/summary context out of model requests; chat itself remains available.

The Rust API disables environment proxies and HTTP redirects for its Ollama client and verifies the model against the local `/api/tags` list before sending a chat request. No remote model-provider configuration or API key is used by this agent path.

## Online technical research providers

Online technical research is exposed through `POST /api/v1/knowledge/search`. The endpoint
first uses the same local Ollama settings described above to classify the query. Classification
fails closed when the local model is unavailable or returns an invalid decision. Only the
sanitized current user query is sent to online providers; files and local diagnostic context
are not accessible to that endpoint. See [Online technical knowledge research](knowledge-research.md)
for the full API contract and privacy model.

| Variable | Purpose |
| --- | --- |
| `ATLAS_BRAVE_SEARCH_API_KEY` | Brave Search API credential. When present, Brave is selected for the `web` provider. |
| `ATLAS_SEARXNG_URL` | SearXNG base URL used when no Brave key is configured. Remote instances require HTTPS; HTTP is allowed only on loopback. |
| `ATLAS_JIRA_INSTANCES` | JSON array of one to ten explicitly approved Jira instances (`name`, `base_url`, `api_version`). No Jira host is enabled by default. |
| `ATLAS_RESEARCH_REDACT_TERMS` | Comma-separated organization-specific terms to redact from outbound search text. |
| `ATLAS_RESEARCH_SECRET_KEYS` | Comma-separated additional credential-field labels recognized by the sanitizer. |
| `ATLAS_RESEARCH_PRIVATE_HOST_SUFFIXES` | Comma-separated private DNS suffixes to redact in user queries. |

Keep search credentials in the backend environment or a secret manager. Do not use
`NEXT_PUBLIC_*` variables for credentials. The sanitizer is best-effort; configure known
customer names and identifiers and review `sanitized_query` during rollout.

## Eclipse MAT container settings

- `ATLAS_MAT_RUNTIME`: `auto` (default), `docker`, or `podman`.
- `ATLAS_MAT_IMAGE`: local MAT worker image reference; default `middleware-atlas/mat:1.17.0`.
- `ATLAS_MAT_BUILD_CONTEXT`: optional override for the image build context; repository default is `tools/mat`.
- `ATLAS_MAT_DOCKER_MEMORY`: container memory limit; default `6g`.
- `ATLAS_MAT_DOCKER_CPUS`: CPU limit; default `2`.
- `ATLAS_MAT_JAVA_MAX_HEAP`: Java heap passed to MAT; default `4g`.
- `ATLAS_MAT_TIMEOUT_SECONDS`: operation timeout; default `7200` seconds.
- `ATLAS_MAT_MAX_CONCURRENT_JOBS`: concurrent MAT job limit; default `1`, clamped to the range 1–8.

A working local container runtime is required for actual MAT execution. The first build of the project-owned image needs network access to download the Eclipse MAT distribution. The one-shot analysis container itself runs without network access. MAT generates index files and needs extra local disk space, so Atlas works in an isolated copy rather than modifying the saved original artifact.

## Artifact classification

Edit or reorder `[[diagnostics.artifact_rules]]` in `config/default.toml`. Rules are checked top-to-bottom. `match_mode = "any"` matches a configured filename token or extension; `match_mode = "all"` requires a configured token and extension. Keep specific product rules before generic archive/text rules. `kind` is the stable machine classification used for backend routing; `product` is the readable label. Only supported kinds have diagnostic plugins—classification does not imply that an analyzer exists for each product.

## Moving existing data

Changing `storage.data_dir` does not automatically move an existing database. Stop the API before copying the old local data. If migrating from `$HOME/.local/share/middleware-atlas` on Linux, copy while the application is stopped so SQLite database/WAL files remain consistent:

```bash
mkdir -p data
cp -a "$HOME/.local/share/middleware-atlas/." data/
```

The data directory can contain sensitive incident metadata, conversations, original files, and generated reports. Keep it local and ignored by Git.
