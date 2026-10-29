# Local-only agent model for Middleware Atlas

Middleware Atlas uses a local Ollama instance directly. There is no Dify dependency and no remote model-provider integration. The Rust backend only permits a loopback HTTP address, disables proxies and redirects, verifies that the configured model is in the local Ollama model registry, and sends bounded context to that local instance. Configure a local model only; do not use Ollama cloud models.

## 1. Install and start Ollama

Install Ollama using its official instructions for your OS. On Linux, check that its service is running:

```bash
systemctl is-active ollama
curl -fsS http://127.0.0.1:11434/api/tags
```

If your installation does not create a system service, start `ollama serve` in a terminal and keep it running.

## 2. Download a local model

For example, use Qwen 3 8B if it fits your hardware:

```bash
ollama pull qwen3:8b
ollama list
```

The model must appear in `ollama list` and the local `/api/tags` response. Never select a cloud model for customer incident data.

## 3. Configure Middleware Atlas

Edit the repository-root `.env` file; keep it untracked. No API key or Dify credentials are required for local Ollama.

```dotenv
ATLAS_LLM_BASE_URL=http://127.0.0.1:11434
ATLAS_LLM_MODEL=qwen3:8b
ATLAS_LLM_SEND_HEAP_CONTEXT=true
```

Use `ATLAS_LLM_SEND_HEAP_CONTEXT=false` to disable attaching report excerpts to model requests. When enabled, Atlas includes at most 48 KiB of bounded local evidence plus recent conversation turns. It excludes the raw heap dump and MAT indexes.

## 4. Start Atlas and test

Restart `cargo run -p atlas-api` after changing `.env`. Keep Next.js running separately. Describe the incident and upload a heap or thread dump. Atlas runs the matching local diagnostic plugin, collects report evidence, then asks Ollama to explain observed facts, hypotheses, confidence, verification steps, and missing evidence. Follow-up turns reuse recent message history from SQLite; the local Ollama endpoint itself is stateless.

Check connectivity without sending incident evidence:

```bash
curl -fsS http://127.0.0.1:11434/api/tags | jq '.models[].name'
```

## Privacy boundary

- Atlas rejects a non-loopback `ATLAS_LLM_BASE_URL`.
- Atlas disables HTTP proxies and redirects for model requests.
- Atlas checks the requested model against Ollama's local model list and rejects model names containing `cloud`.
- Raw `.hprof`, `.phd`, `.heapdump` files and MAT index/cache files are never included in the request. Only bounded report excerpts, the problem description, and conversation turns are included when context sharing is enabled.
- A local Ollama endpoint cannot guarantee privacy if the user independently configures another service to proxy requests externally. Run an actual locally pulled model and do not use a cloud provider/model.

The API is a local single-user development service without authentication; keep it bound to loopback and do not expose it to a network.
