# Local persistence and artifact API

## Storage model

The Rust API is the source of truth for cases and artifact metadata. SQLite stores case names, product-aware upload metadata, SHA-256 hashes, analysis runs, findings, evidence references and tool-run records. Artifact bytes stream to the local filesystem and are never stored in SQLite.

By default the store is `<project-root>/data`, independent of the shell's current working directory. It contains `middleware-atlas.db`, `artifacts/`, `work/` and `reports/`. Relative `ATLAS_DATA_DIR` values are also anchored to the project root. Unix data directories use mode `0700` and database/artifact files use mode `0600`.

See [configuration.md](configuration.md) for the TOML schema, environment overrides, product classification rules and data migration guidance.

## Start the backend

Install the stable Rust toolchain pinned by `rust-toolchain.toml` and Node.js LTS pinned by `.nvmrc`. From the repository root:

```bash
cp .env.example .env  # optional; inspect it and keep it local
cargo run -p atlas-api
```

In another terminal, install the frontend lockfile and start Next.js:

```bash
cd apps/web
npm install
npm run dev
```

The service binds to `127.0.0.1:8080` by default. Health check:

```bash
curl http://127.0.0.1:8080/health
```

The allowed frontend origin defaults to `http://localhost:3000`; override with `ATLAS_WEB_ORIGIN` when needed. `ATLAS_MAX_UPLOAD_BYTES` defaults to 100 GiB. Lower it if that fits your workstation/disk quota better. Upload bytes are streamed in chunks and hashed during the stream.

## API

- `GET /health`
- `GET /api/v1/cases`
- `POST /api/v1/cases` with `{"name":"Broker incident"}`
- `GET /api/v1/cases/{case_id}`
- `PATCH /api/v1/cases/{case_id}` with `{"name":"Updated title"}`
- `GET /api/v1/cases/{case_id}/artifacts`
- `POST /api/v1/cases/{case_id}/artifacts` as multipart form field `file`
- `DELETE /api/v1/cases/{case_id}/artifacts/{artifact_id}`

The artifact API returns `kind` (stable machine classification) and `product` (human-readable product label) alongside filename, size, media type, upload timestamp and SHA-256.

Example upload:

```bash
curl -F 'file=@/path/to/server.log' \
  http://127.0.0.1:8080/api/v1/cases/CASE_UUID/artifacts
```

## Browser integration

The “Continue to case details” action creates or reuses the active case, updates its name, streams selected files to the API sequentially, and records upload markers in browser local storage so a retry doesn't upload completed files again. The local persistence dock lists saved cases/artifacts and allows deleting an artifact. Browser local storage contains active-case and retry bookkeeping, not file contents.

Set `NEXT_PUBLIC_ATLAS_API_BASE_URL` in `apps/web/.env.local` if the backend is not on `127.0.0.1:8080`. Restart Next.js after changing environment variables.

## Limits and security boundary

This is a local single-user development API and binds to loopback by default. It does not yet include user authentication or multi-user authorization. Do not expose it to a network or deploy it for multiple users until authentication, case-level authorization, CSRF protections where relevant, quotas, retention, backups and deployment-specific threat review are added. The upload handler uses server-generated identifiers and storage paths, strips client directories from display names, uses restrictive local permissions, avoids archive extraction, and calculates SHA-256 while streaming. Archive and diagnostic parsers must still run in isolated, resource-limited workers before they are added.
