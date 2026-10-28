# Middleware Atlas Architecture

## 1. Responsibilities

### React / Next.js frontend

Provides case management, artifact upload, analysis progress, chat, report
viewing and evidence-linked findings. It talks only to the Rust application
API. Dify credentials and filesystem paths never reach the browser.

### Rust application backend

Rust is the primary application and security boundary. It owns:

- Authentication and case-level authorization.
- Case and artifact metadata.
- Upload validation and artifact storage.
- Background job lifecycle, cancellation and progress.
- Tool adapter registry and input validation.
- Dify API client and application-level workflow invocation.
- Evidence/finding schemas, audit records and retention rules.
- Resource limits for diagnostic subprocesses.

### Dify

Dify provides configurable AI application/workflow orchestration, knowledge
retrieval and access to configured model providers. Integrate through the
documented API for the selected Dify deployment/version.

Keep Dify's base URL and API key configurable. API keys must be stored in
server-side environment/secrets configuration, never in frontend code or Git.

Use Dify for model/workflow flexibility, not as the sole authorization boundary.
The Rust backend must independently authorize every artifact and tool operation.

### Model providers

The model is configured in Dify. The application must not assume a specific
model family or provider payload format. Local Qwen inference is one option
when exposed through a provider/interface supported by the deployed Dify
version.

Validate tool calling, structured output, context limits and streaming for
each provider/model combination. Degrade gracefully if a model lacks a feature.

## 2. Pluggable tool architecture

Define a versioned tool contract independent of Dify and any one model.

Conceptual request:

    {
      "tool_id": "mat.heap.leak_suspects",
      "version": "1",
      "case_id": "case-internal-id",
      "artifact_id": "artifact-internal-id",
      "parameters": {}
    }

Conceptual result:

    {
      "status": "succeeded",
      "summary": "Report generated",
      "evidence": [],
      "artifacts": [],
      "warnings": []
    }

These JSON examples describe the proposed contract, not an existing endpoint.

A tool adapter must declare its input schema, output schema, permissions,
resource requirements, timeout and supported operations.

Potential adapters:

- `mat.heap`: Eclipse MAT reports and queries.
- `logs.kafka`: Kafka log parsing and event correlation.
- `logs.artemis`: Artemis log analysis.
- `jvm.thread_dump`: thread-state and stack analysis.
- `rhel.sosreport`: safe extraction and selected diagnostic analysis.
- `openshift.diagnostics`: analysis of supplied diagnostic artifacts.
- Future adapters for metrics, internal runbooks and approved support tools.

Prefer a Rust implementation for the adapter. A Python worker is acceptable
when a specialist library justifies it. Workers communicate through a narrow,
versioned JSON contract over HTTP or stdin/stdout, with timeouts and limited
permissions.

Do not let the model construct arbitrary shell commands or filesystem paths.
The Rust service maps authorized internal IDs to validated files and executes
only configured operations.

## 3. Dify integration patterns

### Application workflow

The Rust backend submits a case question and bounded context to a configured
Dify application/workflow using the supported API. It tracks the workflow run
and returns results to the UI.

### Tool execution

Where a Dify workflow needs a diagnostic tool, expose only approved operations
through a secured integration supported by the chosen Dify version. That
integration calls the Rust tool service or a dedicated restricted tool service.

The Rust service validates the user, case, artifact, operation and parameters
before executing anything. Never treat a model-generated tool request as
authorization.

### Knowledge retrieval

Use Dify knowledge features where they fit the desired ingestion and retrieval
workflow. Keep source metadata such as product, version, document kind, source
URL and update date. For exact exception strings, class names and configuration
keys, evaluate keyword retrieval alongside semantic retrieval.

Do not place private customer artifacts into shared knowledge without explicit
sanitization and review.

## 4. Eclipse MAT integration

Keep the original `.hprof` available to MAT. Generate bounded reports and
structured query results through supported MAT batch/query interfaces.

Initial operations:

- Leak Suspects.
- Overview and top consumers.
- Class histogram.
- Dominator Tree.
- Paths to GC Roots.
- Comparison between heap snapshots.

The MAT adapter returns report locations and compact findings with evidence
references. It does not send the entire heap dump or MAT index/cache files to
the LLM.

Run MAT in a constrained background worker. Enforce memory, CPU, disk,
execution-time and concurrency limits.

Evaluate Eclipse Jifa if interactive heap visualization is preferable to
building all visualizations from scratch.

## 5. Evidence-backed RCA

Each finding should record:

- Observed fact.
- Artifact ID and source location.
- Interpretation or hypothesis.
- Confidence and uncertainty.
- Alternative explanations.
- Next verification step.
- Whether an engineer has verified the conclusion.

The agent must not present correlation as proof or label an RCA confirmed
without sufficient evidence and engineering review.

## 6. Security and reliability

- Use server-side secrets for Dify and model-provider credentials.
- Enforce authorization on every case, artifact and tool operation.
- Treat logs, archives, HTML reports and model output as untrusted.
- Defend against path traversal, symlinks and decompression bombs.
- Use job timeouts, quotas and concurrency controls.
- Do not expose arbitrary command execution.
- Require explicit approval for actions that modify customer systems.
- Keep customer artifacts and secrets out of Git.
- Record tool invocations and access events without logging sensitive payloads.
