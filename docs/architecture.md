# Middleware Atlas Architecture

## 1. Responsibilities

### React / Next.js frontend

Provides case management, artifact upload, analysis progress, chat, report
viewing and evidence-linked findings. It talks only to the Rust application
API. Local model configuration and filesystem paths never reach the browser.

### Rust application backend

Rust is the primary application and security boundary. It owns:

- Authentication and case-level authorization.
- Case and artifact metadata.
- Upload validation and artifact storage.
- Background job lifecycle, cancellation and progress.
- Tool adapter registry and input validation.
- Loopback-only Ollama client and application-level agent orchestration.
- Evidence/finding schemas, audit records and retention rules.
- Resource limits for diagnostic subprocesses.

### Local inference and agent orchestration

The Rust API talks directly to a local Ollama service on loopback. It rejects non-loopback model URLs, disables HTTP proxies and redirects, and verifies the requested model is available in the local Ollama model list before sending any incident context. Use locally pulled model weights only; do not use cloud models.

The model is not an authorization boundary. Deterministic Rust routing chooses approved diagnostic plugins from uploaded artifact kinds and explicit operations. Model responses may explain evidence and ask follow-up questions, but may not invent filesystem paths, shell commands, or tool permissions.

Conversation turns and bounded evidence context remain on the local machine. The raw heap dump and MAT indexes are never included in model requests.

## 2. Pluggable tool architecture

Define a versioned tool contract independent of local Ollama and any one model.

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

## 3. Local agent orchestration

The agent combines a fixed system policy, recent conversation turns from SQLite, bounded MAT/thread-dump evidence, and the current user question. Evidence is labelled untrusted input. Responses should separate observed facts, hypotheses, confidence, alternative explanations, and verification steps. If evidence is insufficient, the agent asks for specific missing information.

### Tool routing

Rust routes uploaded `.hprof`/`.phd`/`.heapdump` artifacts to the MAT adapter and thread dumps to the JVM thread-dump adapter. The tool registry has fixed operation IDs and validated parameters. Model content cannot invoke arbitrary commands or alter container permissions.

### Evidence retrieval

Local retrieval is implemented as bounded extraction from MAT ZIP archives, text/CSV/HTML reports, thread-dump summaries, the incident description, and manifests. It limits excerpt size and count, ignores symlinks, excludes raw heap dumps/indexes, and is not a general-purpose web or remote knowledge retrieval client.

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

- Use server-side secrets for local Ollama and model-provider credentials.
- Enforce authorization on every case, artifact and tool operation.
- Treat logs, archives, HTML reports and model output as untrusted.
- Defend against path traversal, symlinks and decompression bombs.
- Use job timeouts, quotas and concurrency controls.
- Do not expose arbitrary command execution.
- Require explicit approval for actions that modify customer systems.
- Keep customer artifacts and secrets out of Git.
- Record tool invocations and access events without logging sensitive payloads.
