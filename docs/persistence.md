# Local persistence and artifact API

## Storage model

The Rust API is the source of truth for cases and artifact metadata. SQLite stores case names, product-aware upload metadata, SHA-256 hashes, analysis runs, findings, evidence references and tool-run records. Artifact bytes stream to the local filesystem and are never stored in SQLite.

By default the store is `<project-root>/data`, independent of the shell's current working directory. It contains `middleware-atlas.db`, `artifacts/`, `work/` and `reports/`. Relative `ATLAS_DATA_DIR` values are also anchored to the project root. Unix data directories use mode `0700` and database/artifact files use mode `0600`.

See [configuration.md](configuration.md) for the TOML schema, environment overrides, product classification rules and data migration guidance.

## Start the backend

Install the stable Rust toolchain pinned by `rust-toolchain.toml` and Node.js LTS pinned by `.nvmrc`. From the repository root:

```bash
cp .env.example .env  # optional; inspect it and keep it local
cargo run -p atlas-api
```

In another terminal, install the frontend lockfile and start Next.js:

```bash
cd apps/web
npm install
npm run dev
```

The service binds to `127.0.0.1:8080` by default. Health check:

```bash
curl http://127.0.0.1:8080/health
```

The allowed frontend origin defaults to `http://localhost:3000`; override with `ATLAS_WEB_ORIGIN` when needed. `ATLAS_MAX_UPLOAD_BYTES` defaults to 100 GiB. Lower it if that fits your workstation/disk quota better. Upload bytes are streamed in chunks and hashed during the stream.

## API

- `GET /health`
- `GET /api/v1/cases`
- `POST /api/v1/cases` with `{"name":"Broker incident"}`
- `GET /api/v1/cases/{case_id}`
- `PATCH /api/v1/cases/{case_id}` with `{"name":"Updated title"}`
- `GET /api/v1/cases/{case_id}/artifacts`
- `POST /api/v1/cases/{case_id}/artifacts` as multipart form field `file`
- `DELETE /api/v1/cases/{case_id}/artifacts/{artifact_id}`
- `POST /api/v1/cases/{case_id}/diagnostics/prepare` with a problem description and optional heap/thread artifact IDs

The artifact API returns `kind` (stable machine classification) and `product` (human-readable product label) alongside filename, size, media type, upload timestamp and SHA-256.

Example upload:

```bash
curl -F 'file=@/path/to/server.log' \
  http://127.0.0.1:8080/api/v1/cases/CASE_UUID/artifacts
```

## Browser integration

The “Continue to case details” action creates or reuses the active case, updates its name, streams selected files to the API sequentially, and records upload markers in browser local storage so a retry doesn't upload completed files again. The local persistence dock lists saved cases/artifacts and allows deleting an artifact. Browser local storage contains active-case and retry bookkeeping, not file contents.

Set `NEXT_PUBLIC_ATLAS_API_BASE_URL` in `apps/web/.env.local` if the backend is not on `127.0.0.1:8080`. Restart Next.js after changing environment variables.

## Limits and security boundary

This is a local single-user development API and binds to loopback by default. It does not yet include user authentication or multi-user authorization. Do not expose it to a network or deploy it for multiple users until authentication, case-level authorization, CSRF protections where relevant, quotas, retention, backups and deployment-specific threat review are added. The upload handler uses server-generated identifiers and storage paths, strips client directories from display names, uses restrictive local permissions, avoids archive extraction, and calculates SHA-256 while streaming. Archive and diagnostic parsers must still run in isolated, resource-limited workers before they are added.

## Diagnostic plugin preparation

The local diagnostic endpoint accepts a problem description plus artifact IDs and routes supported artifacts by the Rust API's canonical `kind`:

- `heap_dump` → `mat.heap_dump.prepare`: creates a private workspace with the dump staged under its original filename, `analysis-request.json`, `problem-description.md`, `mat-operations.json`, and a `README.md`. Staging uses a hard link to avoid copying large heap files. This endpoint prepares the MAT request; it does **not** execute Eclipse MAT or claim reports already exist.
- `thread_dump` → `jvm.thread_dump.prepare`: runs a bounded local text parser (64 MiB input limit) and creates `thread-dump-summary.json`, `thread-dump-summary.md`, `thread-dump-context.md`, `analysis-request.json`, and a `README.md`, alongside the staged input hard link. State counts and lock mentions are triage indicators, not a definitive deadlock detector or RCA.

```bash
curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  -d '{"problem_description":"The service stops responding after several hours","artifact_ids":["ARTIFACT_UUID"]}' \
  http://127.0.0.1:8080/api/v1/cases/CASE_UUID/diagnostics/prepare
```

If `artifact_ids` is omitted or empty, all heap/thread dump artifacts in the case are selected. Preparation outputs live below `data/work/<case-id>/<tool-run-id>/`; `tool_runs` stores each preparation record in SQLite. Raw heap dumps remain local and are not sent to an LLM. Dify/model orchestration and actual MAT execution remain separate future integrations.

## Eclipse MAT Docker operations

When a Docker or Podman runtime is reachable, a heap-dump preparation request automatically builds (if absent) and runs the project-owned `middleware-atlas/mat:1.17.0` image. The image is built from the official Eclipse MAT 1.17.0 stand-alone release and a Java 21 runtime. The worker is one-shot, has no network access, uses a read-only container root filesystem, drops Linux capabilities, and has configurable memory/CPU/time limits. It mounts only the private workspace `data/work/<case-id>/heap/<artifact-id>`. MAT input is copied into this workspace before execution because MAT writes index files beside the input; this requires additional disk space roughly equal to the dump size, while the original saved artifact remains untouched. The worker is built on first use unless `ATLAS_MAT_IMAGE` already exists locally.

The `POST /api/v1/cases/{case_id}/diagnostics/heap/{operation}` endpoint exposes these fixed operations for the agent/tool adapter:

- `analyze_heap_dump` — Leak Suspects, Overview, histogram, top 50 dominators and Top Components.
- `generate_leak_suspects` — MAT Leak Suspects report.
- `generate_heap_overview` — MAT Overview report.
- `get_histogram` with optional `class_filter` — histogram report and, when supplied, a locally filtered text excerpt.
- `get_dominator_tree` with `top_n` (default 50; allowed 1–500).
- `find_paths_to_gc_roots` with a hexadecimal `object_id` such as `0x7f1234ab`.
- `compare_heap_dumps` with `baseline_id` identifying a second heap dump in the same case.

The response and `tool_runs` record include generated report paths. Each successful MAT run creates `mat-execution.json` and a bounded `llm-context.md` assembled from text/CSV reports. The raw dump, indexes and zipped/HTML reports are not copied into the LLM context. When explicitly enabled with `ATLAS_DIFY_SEND_HEAP_CONTEXT=true`, a configured server-side Dify app receives only the bounded context for the requested operation and may return an analysis. This does not yet configure Dify to autonomously call back into these seven operations; the TypeScript/Rust operations are exposed for a Dify tool adapter/workflow to invoke. Set `ATLAS_DIFY_APP_MODE=chat` or `workflow` to match the configured Dify app.

MAT may consume many GB of RAM and disk. Defaults are `ATLAS_MAT_DOCKER_MEMORY=6g`, `ATLAS_MAT_DOCKER_CPUS=2`, `ATLAS_MAT_JAVA_MAX_HEAP=4g`, and `ATLAS_MAT_TIMEOUT_SECONDS=7200`. Tune these for the host and dump size. If Docker is missing or the daemon is unavailable, Atlas leaves the MAT request prepared and returns an actionable warning instead of claiming analysis ran.

The worker uses Eclipse MAT's documented batch reports and query interface: [Batch mode](https://help.eclipse.org/latest/topic/org.eclipse.mat.ui.help/tasks/batch.html) and [Query Report](https://help.eclipse.org/latest/topic/org.eclipse.mat.ui.help/reference/inspections/query_report.html).

## Local-only diagnostic-agent chat

Atlas keeps incident records, uploaded files, generated reports, prompt history, and chat history on the local machine. The Rust backend uses a loopback-only Ollama API (`http://127.0.0.1:11434`) and blocks non-loopback model URLs, redirects, and environment proxies. The model must appear in Ollama's local `/api/tags` list before chat requests are sent. Use a locally pulled model, not an Ollama cloud model.

Set `ATLAS_LLM_BASE_URL=http://127.0.0.1:11434` and `ATLAS_LLM_MODEL=qwen3:8b` in the untracked root `.env` after installing Ollama and pulling the chosen model. `ATLAS_LLM_SEND_HEAP_CONTEXT=true` attaches bounded MAT report excerpts and thread-dump summaries to the local model. Raw `.hprof` files and MAT index files are never sent to the model. The local agent stores conversation turns in SQLite, restores recent history across refreshes, asks for missing evidence where appropriate, and offers eight suggested prompts. No Dify integration is present.
