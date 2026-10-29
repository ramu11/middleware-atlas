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

## 4. Request flow

- Atlas uses deterministic Rust routing and the server-side artifact `kind` to select a supported diagnostic plugin.
- Thread dumps are parsed locally into bounded summaries. Heap dumps use fixed Eclipse MAT operations in a local Docker/Podman container when a usable runtime is available.
- The context collector reads selected local summary, context, manifest, and MAT-report excerpts. It skips symlinks, has an aggregate 48 KiB / 24-file limit, bounds individual excerpts, and does not include the original heap dump or MAT index/cache files.
- Rust calls Ollama's `/api/chat` endpoint. The request includes the fixed agent policy, up to 12 recent stored message records, optional bounded evidence, and the current question. The Ollama service itself is treated as stateless.
- Successful user and assistant messages are saved in SQLite, so Atlas can restore recent conversation history after a browser refresh.

`ATLAS_LLM_SEND_HEAP_CONTEXT` defaults to `true` when unset. Set it to `false` to disable attaching diagnostic evidence to new chat requests; the current question and chat history are still sent to the local model.

## Privacy and security boundary

- Only plain-HTTP loopback model URLs are accepted; remote and LAN endpoints are rejected.
- HTTP redirects and environment proxies are disabled for the model client.
- The selected model is verified against Ollama's local `/api/tags` response before chat context is sent.
- The original heap dump and MAT index/cache files are never included in the model prompt. Context is prepared from named summaries/manifests and bounded report excerpts.
- Report text, logs, archives and stack traces are untrusted evidence—not instructions. The prompt directs the model to separate observations and hypotheses, state uncertainty, and ask for missing evidence.
- The API has no user authentication or multi-user authorization yet. Keep it bound to loopback and do not expose it to a network.

The agent is an evidence assistant, not a guarantee of correct root-cause analysis. Engineers must verify important findings against report references and the original artifacts.
