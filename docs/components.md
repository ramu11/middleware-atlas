<!-- maintained-by: patch_project_docs.py -->
# Component map

This page explains what each major part of the repository does, what talks to what, and which items are planned versus implemented.

## Runtime flow today

```text
Browser
  └── Next.js / React case-intake UI (localhost:3000)
        └── HTTP + multipart requests
              └── Rust Axum API (127.0.0.1:8080)
                    ├── SQLx + SQLite: cases and artifact metadata
                    ├── Local filesystem: original artifact bytes
                    └── config/default.toml: runtime and classification rules
```

SQLite is the source of truth for case/artifact metadata. Artifact bytes live outside SQLite, beneath the configured data directory. Frontend code does not access the database or local file paths directly.

## Repository components

| Path/component | Responsibility | Status |
|---|---|---|
| `apps/web/app/page.tsx` | Main case-intake experience: case name, select/drag files, review selected files, request persistence/upload, render progress and status. | Implemented |
| `apps/web/app/layout.tsx` | Next.js root layout, document metadata, global CSS and shared persistence dock. | Implemented |
| `apps/web/app/globals.css` | UI tokens, layout, responsive styling and component visuals. | Implemented |
| `apps/web/components/persistence-dock.tsx` | Reads saved cases and artifacts through the API, restores active-case context, refreshes persisted data, and requests artifact deletion. | Implemented |
| `apps/web/lib/atlas-api.ts` | Typed browser-side API client for health-independent case/artifact endpoints, JSON errors, and multipart upload with progress reporting. | Implemented |
| `apps/web/.env.example` | Browser-visible API base URL setting. Never put credentials in `NEXT_PUBLIC_*`. | Implemented |
| `crates/atlas-api/src/main.rs` | Axum routes/handlers, API errors, SQLite startup/migrations, CORS, upload streaming, hashing, and local file operations. | Implemented for the current API slice |
| `crates/atlas-api/src/config.rs` | Typed TOML settings, environment overrides, path resolution, config validation and filename-based product classification. | Implemented |
| `crates/atlas-api/src/constants.rs` | Named route paths, storage names and stable error/constants shared by Rust handlers. | Implemented |
| `crates/atlas-api/src/queries.rs` | Central SQL statements used by the Rust API. Schema changes remain in migration files. | Implemented |
| `crates/atlas-api/migrations/0001_initial.sql` | Initial SQLite schema for cases, artifacts, and the application’s supporting records. | Implemented |
| `crates/atlas-api/migrations/0002_artifact_product.sql` | Adds product classification to artifact records for existing databases. | Implemented |
| `config/default.toml` | Non-secret defaults: listen address, CORS origin, data path, SQLite settings, upload limit and ordered artifact/product rules. | Implemented |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | Rust workspace membership, dependency resolution and pinned toolchain. | Implemented |
| `apps/web/package.json`, `apps/web/package-lock.json`, `.nvmrc` | Frontend scripts/dependencies and pinned Node.js version. | Implemented |
| `.env.example` | Backend environment overrides and placeholders; do not store actual secrets here in Git. | Example configuration |
| `docs/architecture.md` | Intended trust boundaries, Dify/tool orchestration design, and the planned evidence model. | Design reference |
| `docs/configuration.md` | Runtime configuration, path overrides, artifact rules and data migration notes. | Operations reference |
| `docs/persistence.md` | Storage model and implemented HTTP API contract. | Operations reference |

## Technology overview

Versions are controlled in repository files; do not manually update the documentation table when the package manifest changes. Source versions at the time this document was generated:

| Technology | Repository value | Why it is used |
|---|---:|---|
| Node.js | `24.22.1` | Runs the frontend toolchain. |
| Rust | `1.99.1` | Primary backend/control-plane language. |
| Next.js | `16.4.3` | UI framework and development/build server. |
| React | `19.3.1` | Interactive browser components and state. |
| TypeScript | `6.0.3` | Typed UI and API client. |
| ESLint | `10.13.0` | Static lint checks for frontend code. |
| Axum | `0.8.9` | Rust HTTP routes and handlers. |
| Tokio | `1.54.1` | Async runtime and asynchronous filesystem/network work. |
| SQLx | `0.9.0` | Asynchronous SQLite access and migrations. |

Other core crates include Serde for JSON/config serialization, `tower-http` for CORS and HTTP tracing, `sha2` for SHA-256, `uuid` for internal IDs, `dotenvy` for local environment loading, and `tracing` for structured logs. Exact Rust dependency resolution is recorded in `Cargo.lock`.

## Storage and trust boundaries

- **Case metadata:** SQLite database under `<project-root>/data` by default; SQLite is configured for WAL mode.
- **Artifact bytes:** each uploaded file is written to a server-generated artifact directory; metadata includes original filename (sanitized for display), classification, product, size, upload time, media type, and SHA-256.
- **Browser storage:** the active case ID and retry bookkeeping only. File bytes are not stored in `localStorage`.
- **Configuration:** TOML and backend environment variables. Secrets must remain server-side and uncommitted.
- **API access:** defaults to loopback. There is no authentication yet; this is a local single-user development service, not ready for shared/network deployment.

## Planned AI and diagnostics components

These are part of the target design, not active end-to-end features in the current checkpoint:

- **Dify:** future model/provider-agnostic workflow orchestration, retrieval, and approved tool routing. Current `ATLAS_DIFY_*` variables are placeholders; the Rust API does not yet invoke a Dify workflow.
- **Eclipse MAT:** future constrained heap-dump worker for Leak Suspects, overview, histogram, dominator tree and Paths to GC Roots. `ATLAS_MAT_HOME` is not proof that MAT calls are wired into the API.
- **Specialist analyzers:** Kafka/Artemis logs, Debezium, thread dumps, sosreports and OpenShift/must-gather bundles are future tool adapters. Product classification is present, but classification is not the same as diagnostic analysis.
- **RCA and evidence explorer:** future workflow to correlate observations and produce hypotheses with source artifact/evidence references and engineer verification. No autonomous root-cause result should be assumed today.

See [architecture.md](architecture.md) for the target architecture and [getting-started.md](getting-started.md) for the local runbook.
