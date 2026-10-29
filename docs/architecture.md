# Middleware Atlas Architecture

## 1. Runtime components

### Next.js / React frontend

The browser provides case intake, file selection/upload, the local persistence dock, diagnostic-agent chat, suggested prompts, conversation restoration, and display of tool-run summaries and warnings. It communicates with the Rust API only. Local filesystem paths, Ollama configuration, and server-side settings are not passed to the browser.

### Rust application API

`crates/atlas-api/src/main.rs` is the application control plane. It owns the HTTP routes, SQLite startup/migrations, case and artifact metadata, upload streaming, SHA-256 hashing, artifact classification, diagnostic routing, chat-history persistence, local model requests, and API errors. `config.rs`, `constants.rs`, and `queries.rs` separate typed runtime configuration, stable route/constants, and SQL statements. `diagnostic_plugins.rs` implements the thread-dump and heap/MAT plugin operations.

### Local stores

- SQLite (`data/middleware-atlas.db` by default) stores case and artifact metadata, diagnostic tool-run records, incident display references, and agent messages.
- The filesystem beneath `data/` stores original uploads and per-case/per-run workspaces, request manifests, summaries, MAT reports, bounded report context, and container logs.
- Browser local storage holds active-case and upload-retry bookkeeping, not uploaded file bytes.

## 2. End-to-end agent flow

1. The user creates or selects an investigation, describes the symptom, and uploads diagnostic files.
2. Rust assigns/retains an internal case UUID and a readable incident reference such as `ATLAS-HEAP-MEMORY-…`. The backend classifies each file using the ordered rules in `config/default.toml`.
3. When diagnostic preparation is requested, Rust resolves the supplied artifact IDs against the current case and routes supported `heap_dump` and `thread_dump` artifacts to their fixed plugins.
4. Thread dumps are read and summarized locally. The parser has a 64 MiB input limit and emits bounded JSON/Markdown summaries; state counts and lock mentions are triage signals, not proof of a deadlock.
5. Heap dumps are staged in a private work area. Rust invokes one of the approved Eclipse MAT operations in the project-owned container image when Docker or Podman is available. The container runs without network access, with a read-only root filesystem, dropped Linux capabilities, and configured CPU, memory, Java heap, and time limits. If a usable runtime is unavailable, the API warns that MAT was not executed rather than claiming results exist.
6. Rust collects only an allowlist of local context files and bounded excerpts from MAT report archives. The agent context is capped at 48 KiB and 24 evidence files; individual context-file excerpts are bounded. Original heap dumps and MAT index/cache files are excluded.
7. When the user submits a chat turn, Rust validates the configured Ollama URL, checks the model against Ollama's local `/api/tags` list, and calls the local `/api/chat` endpoint. It supplies a fixed policy, up to the 12 most recent stored messages, bounded evidence when enabled, and the user's current question.
8. A successful turn is saved to SQLite. The UI can restore the recent conversation after a refresh. The Ollama endpoint itself is treated as stateless; Atlas sends the saved history with each request.

This is a deterministic application flow, not autonomous model-driven tool execution. The model explains results and asks follow-up questions; Rust routes diagnostics and executes only named operations. The agent does not get unrestricted shell or filesystem access.

## 3. Local Ollama boundary

The defaults are:

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

The backend accepts only plain HTTP on a loopback address (`localhost`, IPv4 loopback, or IPv6 loopback), rejects credentials/query/fragment/path components in the base URL, disables redirects and environment proxies, and checks that the requested model is already installed locally. Model names containing `cloud` are rejected. A missing model or unavailable Ollama service fails before Atlas sends incident context in a model request.

`ATLAS_LLM_SEND_HEAP_CONTEXT` controls attachment of bounded diagnostic context and defaults to enabled if unset. Turning it off prevents diagnostic evidence context from being attached to new model requests; it does not disable chat itself. The current implementation does not support remote model providers.

The local evidence policy treats report text, logs, archives, and stack traces as untrusted data rather than instructions. The prompt directs the model to separate observed facts from hypotheses, avoid claiming a confirmed root cause without evidence, and ask for specific missing data where needed. This is guidance to the model, not a substitute for application-level security controls.

## 4. Diagnostic plugins

### JVM thread dumps

The thread-dump plugin writes the staged input reference, `thread-dump-summary.json`, `thread-dump-summary.md`, `thread-dump-context.md`, the problem description, and run manifests. It counts recognizable JVM thread states, captures a bounded set of blocked thread names, counts lock-related mentions, and notes explicit deadlock markers. Thread formats differ; missing markers or WAITING/TIMED_WAITING states do not prove a healthy or unhealthy JVM.

### Eclipse MAT heap dumps

The heap plugin prepares a private workspace and invokes fixed operations using the project-owned `middleware-atlas/mat:1.17.0` image, built from `tools/mat`. Supported operations include:

- `analyze_heap_dump` — composite report run (Leak Suspects, overview, histogram, dominators/top consumers).
- `generate_leak_suspects` — Leak Suspects report.
- `generate_heap_overview` — overview report.
- `get_histogram` — histogram, optionally filtered by `class_filter`.
- `get_dominator_tree` — dominator tree with bounded `top_n`.
- `find_paths_to_gc_roots` — query for a hexadecimal `object_id`.
- `compare_heap_dumps` — comparison with a same-case `baseline_id`.

MAT writes indexes beside its input, so the worker uses an isolated writable copy/workspace rather than modifying the stored original. MAT report context is prepared separately from the raw dump. The context builder has a 128 KiB upper bound; the later agent collector further restricts the evidence sent to chat. If the container runtime cannot be used, MAT request preparation and execution status are recorded without claiming that a report was generated.

## 5. API routes

The main routes are:

- `GET /health`
- `GET /api/v1/cases`, `POST /api/v1/cases`
- `GET /api/v1/cases/{case_id}`, `PATCH /api/v1/cases/{case_id}`
- `GET /api/v1/cases/{case_id}/artifacts`
- `POST /api/v1/cases/{case_id}/artifacts` (multipart field `file`)
- `DELETE /api/v1/cases/{case_id}/artifacts/{artifact_id}`
- `POST /api/v1/cases/{case_id}/diagnostics/prepare`
- `POST /api/v1/cases/{case_id}/diagnostics/heap/{operation}`
- `POST /api/v1/cases/{case_id}/agent/chat`
- `GET /api/v1/cases/{case_id}/agent/messages`

The API schema and examples are documented in [persistence.md](persistence.md).

## 6. Limits and security

- The chat message limit is 12,000 characters. Prompt serialization is limited to 160 KiB, the local model request times out after 360 seconds, and model responses are bounded.
- Diagnostic context collection has explicit byte/file limits, skips symlinks, and selects named report/summary files rather than walking and sending original artifacts indiscriminately.
- The MAT worker runs without container network access and has configurable resource limits.
- Uploaded archives are not generally extracted into arbitrary filesystem paths. Diagnostic operations and paths are determined server-side.
- The API defaults to loopback but currently has no authentication or multi-user authorization. Do not expose it to a network or treat the present development service as production-hardened.
- Keep `data/`, customer files, reports, and `.env` out of version control.

## 7. Known gaps

Artifact classification for Kafka, Artemis, Debezium, sosreports, and OpenShift is not the same as a complete analyzer for each product. Specialized log and bundle analyzers, background job orchestration, a dedicated report/evidence explorer, and structured evidence-backed RCA/verification workflows remain future work. The current model chat should be treated as a local evidence assistant; engineers must verify findings against source artifacts and reports.
