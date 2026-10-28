# Middleware Atlas

**A local-first, model-agnostic AI copilot for middleware troubleshooting
and support engineering.**

Middleware Atlas helps engineers investigate customer cases involving:

- Apache Kafka and ActiveMQ Artemis
- Debezium and Kafka MirrorMaker 2
- Java heap dumps and thread dumps
- RHEL sosreports
- OpenShift and Red Hat OpenShift AI (RHOAI)

## Architecture principles

- **Rust first:** Rust owns the application API, case management, artifact
  lifecycle, security boundaries, diagnostic adapters and job execution.
- **Dify for agent orchestration:** Use
  [Dify](https://github.com/langgenius/dify) for configurable AI workflows,
  knowledge retrieval and model-driven tool orchestration.
- **Model agnostic:** Select and change the model through Dify/provider
  configuration rather than coupling application code to one model.
- **Pluggable tools:** Diagnostic capabilities implement stable contracts.
  MAT is the first planned adapter; future tools must not require rewriting
  the core application.
- **Python only where useful:** Python may implement a specialist adapter
  when its ecosystem offers a concrete advantage. It is not the core backend.
- **Evidence-backed findings:** Distinguish observations, hypotheses and
  confirmed causes. Preserve references to source artifacts and report sections.
- **Local-first:** Support local inference providers, including a compatible
  Qwen serving endpoint, subject to Dify's provider configuration.
- **Safe execution:** The LLM must never receive unrestricted shell access.

## System architecture

    React / Next.js UI
            |
            v
    Rust application API (Axum)
       |              |
       |              +--> Tool adapter registry
       |                       |
       |                       +--> Eclipse MAT
       |                       +--> Log analyzers
       |                       +--> Thread-dump analyzers
       |                       +--> RHEL sosreport analyzers
       |                       +--> Future diagnostic tools
       |
       +--> Dify API
                 |
                 +--> Configured LLM provider/model
                 +--> Knowledge retrieval
                 +--> Workflows and agent orchestration
                 +--> Approved tool integrations

Dify handles AI workflow orchestration. Rust remains the trusted control
plane for case authorization, artifact access, tool execution, resource
limits and audit records.

## Planned stack

| Area | Technology |
|---|---|
| Primary language | Rust |
| Backend/API | Tokio, Axum, Serde |
| UI | Next.js, React, TypeScript |
| UI components | Tailwind CSS, shadcn/ui |
| AI orchestration | Dify, accessed through its API |
| Model selection | Dify-configured provider/model; local Qwen supported through a compatible provider |
| Tool integration | Versioned tool contracts and Rust adapters |
| Heap analysis | Eclipse MAT batch/query interface; evaluate Eclipse Jifa |
| Case metadata | SQLite (SQLx, WAL mode) |
| Artifact storage | Local filesystem initially; optional S3-compatible storage |
| Optional specialist workers | Python behind narrow, versioned contracts |

## Model agnosticism

Do not hard-code Qwen, model names, provider-specific payloads or inference
endpoints into business logic.

Configure the model/provider in Dify. The application communicates with Dify
through a configurable base URL and an API key supplied securely at runtime.

A model/provider is supported only if the selected Dify version and installed
provider support the required capabilities. Tool calling, structured outputs,
context length and streaming must be tested for each selected model.

## Pluggable diagnostic tools

Each tool should have:

- A stable tool ID and version.
- A typed input and output schema.
- A description of supported operations.
- Authorization and case-scope checks.
- Execution timeout and resource limits.
- Structured findings and evidence references.
- Health/readiness information where applicable.
- Audit records and consistent error responses.

The first tool adapter is planned to wrap Eclipse MAT. Additional adapters
may provide Kafka log analysis, Artemis diagnostics, thread-dump analysis,
sosreport analysis, OpenShift inspection and internal support utilities.

Dify can orchestrate approved tools, but tools that access customer artifacts
must execute behind the trusted Rust service or another explicitly secured
tool service. Never expose arbitrary filesystem access or arbitrary commands
to the model.

## Planned roadmap

1. Rust workspace and API.
2. Dify connectivity, configuration and health checks.
3. Case workspace and artifact upload.
4. Model-independent chat and workflow integration.
5. Knowledge ingestion with product/version metadata.
6. Pluggable log-analysis tools.
7. Eclipse MAT heap-analysis adapter.
8. Thread dumps, sosreports and OpenShift diagnostics.
9. Evidence correlation and RCA.
10. Evaluation against sanitized, previously resolved cases.

## Development

Start the Rust API:

    cargo run -p atlas-api

Health check:

    curl http://127.0.0.1:8080/health

Run checks:

    cargo fmt --all -- --check
    cargo test --workspace

Dify should be deployed separately using its official instructions:
https://github.com/langgenius/dify

See `docs/architecture.md` for boundaries and integration details.

See `docs/persistence.md` for the local SQLite schema and API endpoints, and `docs/configuration.md` for runtime settings and artifact classification.

## Security

- Never commit customer artifacts, generated reports, model weights, secrets
  or real case data.
- Keep Dify API keys on the server; never expose them to the browser.
- Isolate case artifacts and validate every artifact ID.
- Run archive extraction, MAT and other parsers in constrained workers.
- Use allowlisted diagnostic operations rather than arbitrary shell commands.
- Apply timeouts, resource quotas, access controls and retention policies.
