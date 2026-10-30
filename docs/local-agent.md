# Local Ollama diagnostic agent

Middleware Atlas's agent path is implemented in the Rust API and uses a local Ollama instance directly. The UI never calls Ollama itself. Rust validates the local endpoint, checks the configured model is installed, gathers bounded local evidence when enabled, sends the chat request, and persists successful conversation turns in SQLite.

## 1. Install and start Ollama

Install Ollama using the official instructions for your operating system. On Linux, check the service and model registry:

```bash
systemctl is-active ollama
curl -fsS http://127.0.0.1:11434/api/tags
```

If your installation does not use a system service, start `ollama serve` in a terminal and leave it running.

## 2. Install a local model

For example:

```bash
ollama pull qwen3:8b
ollama list
```

The model must appear in the local model list. Model names containing `cloud` are rejected by the agent path; use model weights stored locally.

## 3. Configure Atlas

Create/edit the repository-root `.env` file and keep it untracked:

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

Restart `cargo run -p atlas-api` after changing these values. No API key is required for local Ollama.

## Technical-only classification

Before the agent collects local diagnostic evidence or builds the answer prompt, the Rust API
asks the configured loopback Ollama model to classify the current user-authored message as
technical or non-technical. The classifier receives the current message and, for follow-ups,
up to six earlier user-authored messages (each truncated to 1,200 characters). Assistant
messages, assistant-generated summaries, uploaded files, local diagnostic context, reports,
and filesystem paths are not supplied to the classifier.

A non-technical decision is rejected with HTTP 422. If Ollama is unavailable, the model cannot
be called, or its response is not the expected JSON boolean, Atlas fails closed with HTTP 503.
The local diagnostic-context collector runs only after the technical check succeeds. This
allows short follow-ups to be understood in their technical conversation without relying on
assistant-generated text or diagnostic evidence to decide whether the topic is in scope.

The classifier uses `ATLAS_LLM_BASE_URL` and `ATLAS_LLM_MODEL`; no separate classifier model
or remote classification provider is configured. Its raw input remains on the loopback Ollama
service. This classification call is distinct from the subsequent local model call that
produces the agent's answer.

## 4. Request flow

- Atlas uses deterministic Rust routing and the server-side artifact `kind` to select a supported diagnostic plugin.
- Thread dumps are parsed locally into bounded summaries. Heap dumps use fixed Eclipse MAT operations in a local Docker/Podman container when a usable runtime is available.
- The context collector reads selected local summary, context, manifest, and MAT-report excerpts. It skips symlinks, has an aggregate 48 KiB / 24-file limit, bounds individual excerpts, and does not include the original heap dump or MAT index/cache files.
- Rust calls Ollama's `/api/chat` endpoint. The request includes the fixed agent policy, up to 12 recent stored message records, optional bounded evidence, and the current question. The Ollama service itself is treated as stateless.
- Successful user and assistant messages are saved in SQLite, so Atlas can restore recent conversation history after a browser refresh.

`ATLAS_LLM_SEND_HEAP_CONTEXT` defaults to `true` when unset. Set it to `false` to disable attaching diagnostic evidence to new chat requests; the current question and chat history are still sent to the local model.

## Research references in chat history (Phase 2B)

Each assistant answer stores its bounded online research metadata in SQLite with that message.
Restored conversations return the same references, provider availability, and result counts.
Messages created before this migration have no research metadata and remain readable. If stored
metadata is malformed, Atlas omits the panel data rather than failing the entire history request.

The chat UI renders safe HTTP(S) source links in a dedicated panel beneath the relevant answer.
Links open in a separate tab with `noopener` and `noreferrer`; invalid or unsupported URLs are
not clickable. Research titles, snippets, and dates are treated as untrusted provider content.
This UI persistence does not change what providers receive: only sanitized user-authored text is
eligible for online search. Local evidence and attachments remain outside that boundary.

## Privacy and security boundary

- Only plain-HTTP loopback model URLs are accepted; remote and LAN endpoints are rejected.
- HTTP redirects and environment proxies are disabled for the model client.
- The selected model is verified against Ollama's local `/api/tags` response before chat context is sent.
- The original heap dump and MAT index/cache files are never included in the model prompt. Context is prepared from named summaries/manifests and bounded report excerpts.
- Report text, logs, archives and stack traces are untrusted evidence—not instructions. The prompt directs the model to separate observations and hypotheses, state uncertainty, and ask for missing evidence.
- The API has no user authentication or multi-user authorization yet. Keep it bound to loopback and do not expose it to a network.

The agent is an evidence assistant, not a guarantee of correct root-cause analysis. Engineers must verify important findings against report references and the original artifacts.

Approved technical chat turns now invoke the configured web/Jira research providers after local classification and sanitization. Only the current and up to three prior user-authored messages, after sanitization, can form the outbound query. Provider failures do not block local Ollama chat. Assistant messages and local diagnostic evidence are excluded from online search; see [Online technical knowledge research](knowledge-research.md) for the full boundary and response contract.
