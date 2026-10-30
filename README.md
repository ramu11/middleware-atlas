# Middleware Atlas

**A local-first support-engineering copilot for middleware and JVM diagnostics.**

Middleware Atlas keeps case metadata, uploaded artifacts, diagnostic outputs, and agent conversations on the workstation. The Rust backend runs the diagnostic flow, calls a local Ollama model directly, and invokes fixed Eclipse Memory Analyzer (MAT) operations inside a local Docker or Podman container when a container runtime is available.

## What is implemented

- Case creation and listing, artifact upload and deletion, filename-based artifact classification, upload size limits, and streaming SHA-256 hashing.
- SQLite persistence for cases, artifact metadata, diagnostic tool runs, readable incident references, and agent conversation turns.
- Local JVM thread-dump triage: bounded parsing of thread-state entries, blocked-thread names, lock mentions, and explicit deadlock markers.
- Eclipse MAT heap-dump analysis through a constrained local container, when Docker or Podman is available. The fixed operations include Leak Suspects, heap overview, histogram, dominator tree, Paths to GC Roots, and heap comparison.
- Local agent chat through the Ollama HTTP API. The agent uses recent SQLite conversation history and bounded diagnostic evidence; it can explain reports, distinguish observations from hypotheses, and ask for missing details.
- Server-side technical-topic classification through local Ollama and automatic chat research through configured web/Jira providers. Outbound queries are sanitized locally and limited to user-authored chat text; local attachments and diagnostic evidence are excluded.
- Eight suggested questions in the UI and persistent conversation history across browser refreshes.

Artifact classification is not the same as analysis: Kafka, Artemis, Debezium, sosreport, and OpenShift artifacts can be classified, but specialized analyzers for those artifact types are not all implemented yet.

## Architecture

```text
Browser: Next.js / React
          |
          | HTTP API and artifact uploads
          v
Rust API: Axum
  |       |             |
  |       |             +--> Local filesystem: artifacts and workspaces
  |       +----------------> SQLite: cases, tool runs, agent messages
  |
  +--> Local thread-dump parser
  |
  +--> Docker / Podman --> constrained Eclipse MAT worker (heap dumps)
  |
  +--> Ollama at 127.0.0.1:11434 --> locally installed model
```

The browser talks only to the Rust API. The backend selects diagnostic behavior from validated artifact kinds and fixed operation names; the chat model is not given arbitrary shell access and does not independently invoke tools. Ollama calls are made directly by Rust, not through a hosted orchestration service.

## Prerequisites

- Git.
- Node.js and Rust versions pinned in `.nvmrc` and `rust-toolchain.toml`.
- Ollama with a model downloaded to the local model store for agent chat.
- Docker or Podman for actual Eclipse MAT execution on heap dumps. Without a usable container runtime, Atlas can prepare the request workspace but must report that MAT did not run.

## Run locally

From the repository root, create an untracked `.env` file with your local model settings:

```bash
cat > .env <<'EOF'
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
EOF
```

Install/start Ollama and pull the example model if it suits your hardware:

```bash
ollama pull qwen3:8b
ollama list
curl -fsS http://127.0.0.1:11434/api/tags
```

Start the API in Terminal 1:

```bash
cargo run -p atlas-api
```

Start the UI in Terminal 2:

```bash
cd apps/web
npm ci
npm run dev
```

Open `http://localhost:3000`. The API defaults to `http://127.0.0.1:8080`; verify it with:

```bash
curl -fsS http://127.0.0.1:8080/health
```

If the browser must use another API address, configure `NEXT_PUBLIC_ATLAS_API_BASE_URL` in `apps/web/.env.local` and restart Next.js. Never put secrets in `NEXT_PUBLIC_*` values.

See [Getting started](docs/getting-started.md), [Local agent setup](docs/local-agent.md), [Online technical research](docs/knowledge-research.md), [Architecture](docs/architecture.md), [Configuration](docs/configuration.md), [Persistence and API](docs/persistence.md), [Component map](docs/components.md), and [Troubleshooting](docs/troubleshooting.md).

## Local inference and privacy boundary

The default model endpoint is `http://127.0.0.1:11434`. The Rust backend accepts only plain-HTTP loopback URLs, disables HTTP proxies and redirects for these requests, verifies that the selected model is already installed in Ollama, and rejects model names containing `cloud`. It does not require an API key for local Ollama.

`ATLAS_LLM_SEND_HEAP_CONTEXT` controls whether bounded local diagnostic context is attached to chat requests; it defaults to enabled when unset. Raw heap dumps and MAT index/cache files are not sent to the model. The agent context collector selects specific local summaries/manifests and bounded MAT report excerpts; it does not upload the original dump as chat context.

The API is a local single-user development service without user authentication or multi-user authorization. Keep it bound to loopback and do not expose it to a network. Local inference does not make the application production-ready by itself.

## Online research references in chat

Approved technical chat turns can include references from the configured web and Jira providers.
The chat UI displays provider status and clickable source references beneath the assistant answer.
Reference metadata is stored with that assistant message in the local SQLite conversation history,
so it is restored when the investigation is reopened. Existing messages without research metadata
remain compatible. Outbound provider queries remain limited to sanitized user-authored text; local
attachments, assistant content, and diagnostic evidence are not sent to search providers.

See [Online technical knowledge research](docs/knowledge-research.md) for provider configuration,
redaction controls, data boundaries, and known limitations.

## Development checks

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

Do not commit `.env`, local databases, `data/`, customer artifacts, generated reports, model weights, or local backups.

## Implemented versus planned

Implemented flows are case/artifact persistence, local thread-dump triage, constrained MAT heap operations when a local container runtime is usable, and Ollama-backed evidence-led chat. Broader Kafka/Artemis log analytics, Debezium-specific analysis, sosreport/OpenShift analyzers, a dedicated evidence explorer, and fully structured evidence-backed RCA workflows remain future work. The agent's output is investigative guidance, not a substitute for validating conclusions against the original diagnostic evidence.
