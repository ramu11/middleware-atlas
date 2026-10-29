# Local persistence and artifact API

## Storage model

The Rust API is the source of truth for cases and artifact metadata. SQLite stores case records, artifact metadata, SHA-256 values, diagnostic tool-run records, readable incident references, and agent conversation turns. Uploaded bytes and generated outputs live on the local filesystem rather than in SQLite.

By default, the data directory is `<project-root>/data`, independent of the shell's current directory. Typical contents are:

```text
data/
├── middleware-atlas.db
├── artifacts/<case-id>/<artifact-id>/original
├── work/<case-id>/...
└── reports/
```

On Unix, Atlas applies restrictive permissions to local data directories/files. Keep the directory out of Git: it may contain customer evidence, diagnostic output, and conversation history. See [configuration.md](configuration.md) for path handling and environment overrides.

## Start the backend and frontend

Create the root `.env` for local Ollama (optional if using defaults, but the selected model must be installed):

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

Then start the API from the repository root:

```bash
cargo run -p atlas-api
```

In another terminal:

```bash
cd apps/web
npm ci
npm run dev
```

The API defaults to `127.0.0.1:8080`, and the allowed browser origin defaults to `http://localhost:3000`. The default upload size limit is 100 GiB per file; configure a smaller limit if appropriate for your workstation.

## HTTP API

- `GET /health`
- `GET /api/v1/cases`
- `POST /api/v1/cases` with `{"name":"Broker incident"}`
- `GET /api/v1/cases/{case_id}`
- `PATCH /api/v1/cases/{case_id}` with `{"name":"Updated title"}`
- `GET /api/v1/cases/{case_id}/artifacts`
- `POST /api/v1/cases/{case_id}/artifacts` as multipart form field `file`
- `DELETE /api/v1/cases/{case_id}/artifacts/{artifact_id}`
- `POST /api/v1/cases/{case_id}/diagnostics/prepare`
- `POST /api/v1/cases/{case_id}/diagnostics/heap/{operation}`
- `POST /api/v1/cases/{case_id}/agent/chat`
- `GET /api/v1/cases/{case_id}/agent/messages`

The API returns an internal case UUID for API use and a readable incident reference for people. Artifact metadata includes the canonical `kind`, human-readable `product`, sanitized filename, size, media type, upload timestamp, and SHA-256.

Example upload:

```bash
curl -F 'file=@/path/to/server.log' \
  http://127.0.0.1:8080/api/v1/cases/CASE_UUID/artifacts
```

## Diagnostic preparation

The preparation body contains a problem description and optional artifact IDs. If `artifact_ids` is missing/empty, all supported heap/thread-dump artifacts in the case may be selected. For example:

```bash
curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  -d '{"problem_description":"The service stops responding after several hours","artifact_ids":["ARTIFACT_UUID"]}' \
  http://127.0.0.1:8080/api/v1/cases/CASE_UUID/diagnostics/prepare
```

The route uses the server-side artifact `kind` to select a fixed plugin:

- `heap_dump` → `mat.heap_dump.prepare`, stages the dump and request/manifests in a private local workspace, then the API attempts the default `analyze_heap_dump` operation.
- `thread_dump` → `jvm.thread_dump.prepare`, applies a local text parser (64 MiB input limit) and creates `thread-dump-summary.json`, `thread-dump-summary.md`, and `thread-dump-context.md` with run metadata.

The thread summary includes recognizable JVM thread-state counts, a bounded list of blocked thread names, lock-related mentions, and any explicit deadlock marker. These are triage cues, not definitive deadlock detection or confirmed root cause. Original thread-dump text remains on disk; the model context collector uses the named summary/context files, not the original dump file.

Preparation/tool-run records are persisted in SQLite; workspace files are written beneath `data/work/`. Raw heap dump files and MAT index/cache files are not included in model requests.

## Eclipse MAT operations

When a usable Docker or Podman runtime is available, the heap diagnostic flow builds the local `middleware-atlas/mat:1.17.0` image if necessary and executes fixed MAT operations in a one-shot container. The container has no network access, uses a read-only root filesystem, drops Linux capabilities, and receives configured resource limits. The input is staged as an isolated writable copy because MAT writes indexes beside its input; the saved original artifact is left untouched. Building the image for the first time does require downloading the Eclipse MAT distribution.

The route `POST /api/v1/cases/{case_id}/diagnostics/heap/{operation}` exposes these named operations:

- `analyze_heap_dump` — combined Leak Suspects, overview, histogram, and top dominator/consumer reports.
- `generate_leak_suspects` — Leak Suspects report.
- `generate_heap_overview` — overview report.
- `get_histogram` with optional `class_filter`.
- `get_dominator_tree` with `top_n` (default 50; allowed range 1–500).
- `find_paths_to_gc_roots` with a hexadecimal `object_id`.
- `compare_heap_dumps` with `baseline_id` identifying a second heap dump in the same case.

Operations are allowlisted; they are not arbitrary shell commands. Results include report paths and a `mat-execution.json` manifest. The MAT worker creates `llm-context.md` containing bounded report excerpts (up to 128 KiB at the MAT context-building stage); the agent context collector then applies a smaller aggregate limit before anything is attached to chat. The raw dump, MAT index/cache files, and entire zipped/HTML reports are not sent as-is to the model.

If neither Docker nor Podman is available, Atlas records a warning and reports that MAT was not executed. Install/start a supported local engine or configure `ATLAS_MAT_RUNTIME` appropriately before retrying. Heap analysis can require substantial disk and RAM. Defaults include `ATLAS_MAT_DOCKER_MEMORY=6g`, `ATLAS_MAT_DOCKER_CPUS=2`, `ATLAS_MAT_JAVA_MAX_HEAP=4g`, and `ATLAS_MAT_TIMEOUT_SECONDS=7200`.

Eclipse MAT references: [Batch mode](https://help.eclipse.org/latest/topic/org.eclipse.mat.ui.help/tasks/batch.html) and [Query Report](https://help.eclipse.org/latest/topic/org.eclipse.mat.ui.help/reference/inspections/query_report.html).

## Local agent chat with Ollama

The browser posts user messages to `/api/v1/cases/{case_id}/agent/chat`; Rust calls local Ollama directly at `/api/chat`. Before sending a question/context, the API validates the configured plain-HTTP loopback URL and verifies the configured model against the local `/api/tags` model list. Redirects and environment proxies are disabled. The model must be locally installed; cloud-named models are rejected.

Configuration:

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

The default model is `qwen3:8b`. `ATLAS_LLM_SEND_HEAP_CONTEXT` defaults to `true` when unset. When enabled, the agent context collector scans local workspaces for a fixed set of summary/context/manifests and selected MAT report ZIPs, ignores symlinks, and caps the aggregate context at 48 KiB and 24 files. Individual report excerpts are bounded. Raw `.hprof`, `.phd`, `.heapdump` inputs and MAT indexes/cache files are excluded. Set the variable to `false` to stop attaching diagnostic evidence context to new chat requests.

The chat input is limited to 12,000 characters. The model request is bounded to 160 KiB and times out after 360 seconds; the backend also limits response size. Atlas sends up to 12 recent stored message records as history for a new request and stores successful user/assistant turns in SQLite. The UI can restore recent history after refresh; the API message-list endpoint returns up to 100 recent records.

The local model is asked to reference available evidence, distinguish observations from hypotheses, avoid overclaiming, and ask for missing evidence. Treat the output as diagnostic guidance that requires engineering verification.

## Browser integration and security boundary

The browser uses `NEXT_PUBLIC_ATLAS_API_BASE_URL` from `apps/web/.env.local` if the API is not at the default address. Since `NEXT_PUBLIC_*` variables are exposed to the browser, they must not contain credentials.

This is a local single-user development API. It does not yet implement user authentication or multi-user authorization. Do not expose it to a network or deploy it for shared use before adding authentication, case-level authorization, suitable CSRF protections, quotas, retention/backup policies, and deployment-specific threat review. The API validates IDs, uses server-generated storage paths, streams uploads to disk while hashing, avoids general archive extraction, and constrains known diagnostic operations. Keep local data and customer artifacts out of Git.
