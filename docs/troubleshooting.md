# Local development troubleshooting

## API cannot start or port 8080 is already in use

- Ensure only one `cargo run -p atlas-api` process is using `127.0.0.1:8080`.
- Check `ATLAS_BIND_ADDR` in `.env` and `config/default.toml`.
- Run from the repository root so local `.env` loading is predictable.

## UI says the API is unavailable

1. Run `curl -fsS http://127.0.0.1:8080/health`.
2. Check `apps/web/.env.local` contains `NEXT_PUBLIC_ATLAS_API_BASE_URL=http://127.0.0.1:8080` if using a non-default API address.
3. Restart `npm run dev` after changing `.env.local`.
4. Check `ATLAS_WEB_ORIGIN` if the frontend origin differs from `http://localhost:3000`.

## Ollama is unavailable or the model is rejected

```bash
systemctl is-active ollama
curl -fsS http://127.0.0.1:11434/api/tags
ollama list
```

- Start Ollama using your installation's service method or run `ollama serve` in a terminal.
- Pull the configured model, for example `ollama pull qwen3:8b`, and make sure its name matches `ATLAS_LLM_MODEL`.
- Check the root `.env`; Atlas only accepts a plain-HTTP loopback URL. Remote/LAN endpoints, cloud-named models, redirects, and environment proxies are blocked by design.
- Restart `cargo run -p atlas-api` after changing environment settings. The error response should state that no model data was sent when Ollama is unavailable or the configured model is not installed.

## Heap dump has no MAT report

- Verify Docker or Podman is installed and its daemon/socket is usable by the same user running Atlas.
- Check `ATLAS_MAT_RUNTIME=auto`, `docker`, or `podman` and the worker image/build context settings.
- The first image build downloads the Eclipse MAT distribution and needs network access; the analysis container itself runs without network access.
- Inspect the diagnostic warning, `mat-execution.json`, and `mat-container.log` in the local work area. If no usable container runtime is detected, Atlas prepares the heap request but correctly reports that MAT did not execute.
- Ensure sufficient free disk space and raise/tune MAT memory, CPU, Java heap, or timeout settings as needed for the dump.

## Thread-dump summary looks incomplete

The local parser accepts up to 64 MiB and recognizes common JVM thread formats. JVM-specific formats may not expose standard `java.lang.Thread.State:` entries. A missing explicit deadlock marker does not prove there is no deadlock, and WAITING/TIMED_WAITING alone are not proof of a hang. Compare several dumps and inspect the original stacks/lock owners.

## Upload fails or large files are rejected

- Check disk space in the configured data directory.
- Review `[storage].max_upload_bytes` in `config/default.toml` or `ATLAS_MAX_UPLOAD_BYTES`. The default per-file limit is 100 GiB.
- Keep the API and frontend running throughout the upload, and review the API error before retrying.
- Atlas stores uploaded archives as files; do not assume generic archive extraction or a product-specific analyzer is implemented.

## TypeScript, ESLint, Next.js build, Rust compile, or tests fail

Frontend checks from `apps/web`:

```bash
npm ci
npm run typecheck
npm run lint
npm run build
```

Rust checks from the repository root:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

Use the repository's pinned runtimes in `.nvmrc` and `rust-toolchain.toml`. If you intentionally changed dependencies, review the manifests and lockfiles together.

## Data is not where expected

By default the data directory is `<project-root>/data`, not the current shell directory. See [configuration.md](configuration.md) for `ATLAS_DATA_DIR` and project-root behavior. The root `/data/` directory is ignored by Git; do not force-add it.

## Security reminders

- The API is a local single-user development service without authentication. Do not bind it to an external interface or expose it to a network.
- Never commit `.env`, local databases, generated reports, model weights, or customer artifacts.
- Do not place secrets in `NEXT_PUBLIC_*` variables.
- Validate agent conclusions against source artifacts and reports; the model output is not proof of root cause.
