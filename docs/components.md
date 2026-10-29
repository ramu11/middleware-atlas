# Component map

This page describes the files and runtime behavior in the current local-agent checkpoint. “Implemented” means code exists in the repository; it does not mean every artifact type has a specialist analyzer.

## Runtime flow

```text
Browser: Next.js / React
  └── Rust Axum API (127.0.0.1:8080)
        ├── SQLite: cases, artifact metadata, tool runs, agent messages
        ├── Local filesystem: original uploads, summaries, MAT reports/workspaces
        ├── Local thread-dump parser
        ├── Docker / Podman: fixed Eclipse MAT worker operations for heap dumps
        └── Local Ollama API (127.0.0.1:11434): agent chat
```

The browser talks only to the Rust API. Rust selects diagnostic behavior using server-side artifact classification and fixed operation names. The agent chat does not hand arbitrary tool or shell execution to the model.

## Main components

| Path | Responsibility | Status |
|---|---|---|
| `apps/web/app/page.tsx` | Case intake, file selection, uploads, and investigation flow. | Implemented |
| `apps/web/components/persistence-dock.tsx` | Lists persisted cases/artifacts, restores active case state, refreshes records, and requests artifact deletion. | Implemented |
| `apps/web/components/agent-conversation.tsx` | Suggested prompts, incident reference, diagnostic run summaries, chat composer, and conversation restoration. | Implemented |
| `apps/web/lib/atlas-api.ts` | Typed browser client for case/artifact/diagnostic/agent routes and multipart uploads. | Implemented |
| `apps/web/lib/sha256-stream.ts` | Streaming SHA-256 support used by upload handling. | Implemented |
| `apps/web/app/globals.css` and `apps/web/app/layout.tsx` | Shared UI styling, root layout, document metadata. | Implemented |
| `crates/atlas-api/src/main.rs` | Axum routes/handlers, startup and migrations, uploads/hashing, classification, diagnostic routing, bounded evidence collection, local Ollama requests, SQLite agent history. | Implemented |
| `crates/atlas-api/src/diagnostic_plugins.rs` | Private workspace preparation, thread-dump summary generation, fixed Eclipse MAT operations, report collection and manifests. | Implemented |
| `crates/atlas-api/src/config.rs` | Typed TOML settings, environment overrides, path resolution, validation and artifact classification. | Implemented |
| `crates/atlas-api/src/constants.rs` | Named API routes and stable constants. | Implemented |
| `crates/atlas-api/src/queries.rs` | Shared SQL statement definitions. | Implemented |
| `crates/atlas-api/migrations/0001_initial.sql` | Initial SQLite schema. | Implemented |
| `crates/atlas-api/migrations/0002_artifact_product.sql` | Product classification field/migration. | Implemented |
| `crates/atlas-api/migrations/0003_agent_conversation.sql` | Readable incident references, saved agent context hash, and `agent_messages`. | Implemented |
| `config/default.toml` | Non-secret defaults: listener/origin, storage/database settings, upload size, and ordered artifact rules. | Implemented |
| `tools/mat/Dockerfile`, `tools/mat/mat-job` | Build and run the constrained Eclipse MAT worker. | Implemented; container engine required at runtime |
| `docs/local-agent.md` | Ollama installation/configuration and privacy boundary. | Operations reference |
| `docs/architecture.md` | Current runtime flow, trust boundaries and implemented/planned separation. | Architecture reference |
| `docs/configuration.md` | Environment variables, path handling, MAT resource controls and data layout. | Operations reference |
| `docs/persistence.md` | SQLite/filesystem model and HTTP API. | Operations reference |
| `docs/getting-started.md` | Local setup and investigation workflow. | Operations reference |
| `docs/troubleshooting.md` | Common setup/runtime failures. | Operations reference |

## Data and trust boundaries

- SQLite stores metadata and agent conversation turns; artifact/report bytes are kept in the filesystem under `data/`.
- The browser stores active-case and upload retry bookkeeping, not original file contents.
- Thread-dump analysis reads local text and writes bounded summaries; it is not a formal deadlock proof.
- Heap analysis calls named MAT operations inside a local container. The worker has no network access and has configured resource limits.
- Local agent chat calls Ollama directly. The model URL must be loopback-only, proxies/redirects are disabled, and the chosen model must exist in the local Ollama registry.
- The context collector only includes explicitly selected summary/context/manifest files and bounded MAT report excerpts. It skips symlinks and excludes raw heap dump files and MAT indexes/cache files.
- The API is intended for a single local user and does not yet implement authentication or multi-user authorization.

## Not yet complete

Product classification rules for Kafka, Artemis, Debezium, sosreports and OpenShift do not mean full analyzers for those products are available. Dedicated log/bundle analyzers, a report/evidence explorer, background job management, and a formal structured RCA/evidence-verification flow remain future work.
