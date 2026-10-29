# Getting started: run Middleware Atlas locally

This guide describes the current implementation: the case/artifact UI, Rust API, SQLite/filesystem persistence, local thread-dump triage, constrained Eclipse MAT execution for heap dumps, and direct local Ollama agent chat.

## 1. Prerequisites

- Git.
- Node.js matching `.nvmrc` and Rust matching `rust-toolchain.toml`.
- A local Ollama installation for agent chat.
- A locally downloaded Ollama model. The example below uses `qwen3:8b`; choose a model that fits your workstation.
- Docker or Podman if you want real Eclipse MAT heap analysis. The application can prepare heap workspaces without a usable container engine, but it must not report MAT analysis as successful when MAT did not run.
- Enough free disk space for original diagnostic files plus local workspaces and MAT indexes/reports. Heap operations may need additional temporary space roughly comparable to the dump size.

Install frontend packages using the repository lockfile:

```bash
cd apps/web
npm ci
cd ../..
```

## 2. Configure local Ollama

Start Ollama using your OS's normal service method. Verify it and pull a local model:

```bash
curl -fsS http://127.0.0.1:11434/api/tags
ollama pull qwen3:8b
ollama list
```

Create the root `.env` file. Keep it local and do not commit it:

```bash
cat > .env <<'EOF'
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
EOF
```

The default values are also used when variables are unset, but the model must already be installed in local Ollama. `ATLAS_LLM_SEND_HEAP_CONTEXT=true` allows Atlas to attach bounded report/summary context to chat requests. Set it to `false` to disable diagnostic-context attachment to new requests.

Atlas accepts only a plain-HTTP loopback URL for the model service. It blocks remote/LAN endpoints, redirects, and environment proxies, and checks Ollama's local model list before posting any incident context. Do not configure a cloud model for customer data.

## 3. Start the API and UI

In Terminal 1, from the repository root:

```bash
cargo run -p atlas-api
```

The API listens on `http://127.0.0.1:8080` by default.

In Terminal 2:

```bash
cd /path/to/middleware-atlas/apps/web
npm run dev
```

Open `http://localhost:3000`. If the UI needs a different API address, create/update `apps/web/.env.local`:

```dotenv
NEXT_PUBLIC_ATLAS_API_BASE_URL=http://127.0.0.1:8080
```

Restart Next.js after changing browser environment variables. Values prefixed with `NEXT_PUBLIC_` are sent to the browser and must not contain secrets.

## 4. Verify the API

```bash
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8080/api/v1/cases
```

The health endpoint returns a small JSON object with `status` and `version`. API routes and request examples are in [persistence.md](persistence.md).

## 5. Run an investigation

1. Confirm that the API is running and open the Middleware Atlas UI.
2. Enter a case name or choose a saved case, describe the symptoms, and add relevant files.
3. Continue to the case details/investigation flow. Atlas stores case/artifact metadata in SQLite and the file bytes beneath the local data directory.
4. The Rust API uses the server-side artifact classification to select supported diagnostic work. Thread dumps are summarized locally. Heap dumps trigger the fixed MAT analysis flow when Docker or Podman is reachable.
5. Review the tool-run summary and warnings. A missing/unavailable container runtime means a heap request was prepared but MAT did not execute; install/start Docker or Podman and retry when appropriate.
6. Ask the agent a question or select one of the eight suggested prompts. Atlas sends the current question, recent saved conversation messages, and—when enabled—bounded diagnostic context to the configured local Ollama model.
7. Continue the same conversation to ask follow-up questions or supply missing details. The chat history is stored in SQLite and can be restored after a browser refresh.

The model explains the evidence available to it; it does not have arbitrary shell access and does not autonomously select/run arbitrary tools. Check findings against the named report files and original artifacts.

## 6. Local data layout

The default location is `<project-root>/data/`, ignored by Git. Typical contents include:

```text
data/
├── middleware-atlas.db
├── artifacts/<case-id>/<artifact-id>/original
├── work/<case-id>/...
└── reports/
```

The exact work subdirectories depend on artifact IDs and tool runs. SQLite holds metadata and conversation turns; file contents and generated reports are stored on disk, not in SQLite. The browser stores active-case and upload retry state only.

See [configuration.md](configuration.md) for path/env behavior. The default listener is loopback-only, but the API currently has no user authentication or multi-user authorization. Do not expose it to a network.

## 7. Eclipse MAT details

The heap worker image is built from `tools/mat` if it is missing. The initial image build requires access to download the Eclipse MAT distribution; the one-shot worker runs with network access disabled. Input is copied into an isolated writable workspace because MAT writes index files next to its input. The saved original artifact remains unchanged.

The runtime preference is `ATLAS_MAT_RUNTIME=auto` by default; set it to `docker` or `podman` to force a specific engine. Resource controls include `ATLAS_MAT_DOCKER_MEMORY`, `ATLAS_MAT_DOCKER_CPUS`, `ATLAS_MAT_JAVA_MAX_HEAP`, and `ATLAS_MAT_TIMEOUT_SECONDS`. See [persistence.md](persistence.md#eclipse-mat-operations) and [configuration.md](configuration.md).

## 8. Developer checks

From the repository root:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

From `apps/web`:

```bash
npm run typecheck
npm run lint
npm run build
```

Commit `Cargo.lock` and `apps/web/package-lock.json`. Do not commit `data/`, `.env`, `.env.local`, original customer artifacts, generated reports, model weights, or local backups.

## Current boundaries

The implemented diagnostic analyzers in this checkpoint are local thread-dump summary parsing and fixed Eclipse MAT heap operations. Kafka, Artemis, Debezium, sosreport, and OpenShift names/rules are available for artifact classification, but full product-specific analyzers for those artifact types are not all implemented. A report/evidence explorer, background analysis jobs, and a formal end-to-end RCA verification workflow are future work.
