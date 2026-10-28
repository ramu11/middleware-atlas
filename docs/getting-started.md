<!-- maintained-by: patch_project_docs.py -->
# Getting started: run Middleware Atlas locally

This guide describes how to run the current local-development version of Middleware Atlas. At this checkpoint, the working slice is the **case intake UI + Rust API + SQLite metadata + local artifact files**. Dify-driven analysis and Eclipse MAT execution are planned integrations, not functioning agent features yet.

## 1. Components and software you need

| Component/tool | Version in this repository | Purpose |
|---|---:|---|
| Node.js | `24.22.1` |
| Rust toolchain | `1.99.1` |
| Next.js | `16.4.3` |
| React | `19.3.1` |
| TypeScript | `6.0.3` |
| ESLint | `10.13.0` |
| Axum | `0.8.9` |
| SQLx | `0.9.0` |
| Tokio | `1.54.1` |
| SQLite | Bundled through SQLx's SQLite driver | Local case/artifact metadata database |
| npm | The version installed with your Node.js toolchain | Install frontend dependencies and run Next.js scripts |
| curl | Any recent version | Health checks and API smoke tests |
| Python 3 | Optional | Run repository patch/smoke-test scripts if needed |
| Browser | Recent Chromium, Firefox, or Safari | Use the Next.js case intake UI |

**Source of truth for versions:** `.nvmrc`, `rust-toolchain.toml`, `apps/web/package.json`, `Cargo.toml`, `crates/atlas-api/Cargo.toml`, and the lockfiles. Use the repository versions rather than independently upgrading a single package.

### Prerequisites

- Git and a working checkout of this repository.
- `rustup` installed so Rust can use `rust-toolchain.toml`. The first Rust build downloads crates and may take a few minutes.
- Node.js matching `.nvmrc`. If you use `nvm`, run `nvm install` and `nvm use` from the repository root. If `nvm` is not installed, install the Node.js version shown in `.nvmrc` using your preferred version manager.
- Enough free disk space for the source artifacts you intend to inspect. Heap dumps and diagnostic bundles can be very large.

## 2. First-time setup

From the repository root:

```bash
cd /path/to/middleware-atlas

# Check the pinned runtimes
rustc --version
cargo --version
node --version
npm --version

# Optional local backend settings; keep .env untracked and do not put real secrets in Git
cp .env.example .env
```

Review `.env` and `config/default.toml` before running. The checked-in example is for local development; it is not a production deployment configuration.

Install frontend packages once (or again after `package.json` / `package-lock.json` changes):

```bash
cd apps/web
npm ci
```

If you have just changed the dependency manifest locally and intentionally need to refresh the lockfile, use `npm install` instead of `npm ci`, run all checks, and review both `package.json` and `package-lock.json` before committing.

## 3. Start the API and UI

Start the Rust API in **Terminal 1**. Run it from the repository root so `.env` is found in the expected location:

```bash
cd /path/to/middleware-atlas
cargo run -p atlas-api
```

Default API address: `http://127.0.0.1:8080`.

Start Next.js in **Terminal 2**:

```bash
cd /path/to/middleware-atlas/apps/web
npm run dev
```

Open **http://localhost:3000** in your browser. Keep both terminals running while using the application.

If the UI needs a different API address, create `apps/web/.env.local` from `apps/web/.env.example`, set `NEXT_PUBLIC_ATLAS_API_BASE_URL`, and restart Next.js:

```dotenv
NEXT_PUBLIC_ATLAS_API_BASE_URL=http://127.0.0.1:8080
```

`NEXT_PUBLIC_*` values are exposed to the browser. Do not put secrets or Dify API keys in them.

## 4. Verify the API is running

```bash
curl -fsS http://127.0.0.1:8080/health
```

Expected response shape:

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

The current API endpoints are documented in [persistence.md](persistence.md). A minimal check of case listing is:

```bash
curl -fsS http://127.0.0.1:8080/api/v1/cases
```

## 5. Use the current application

1. Confirm the API is available (the **Local cases / Local persistence** dock shows connection status).
2. Enter a case name or select an existing case from the dock.
3. Add diagnostic files using **Browse files** or drag-and-drop.
4. Select **Continue to case details** to create/update the case and upload selected artifacts to the Rust API. Uploads are sent sequentially; large files can take time and require sufficient disk space.
5. Review persisted case/artifact metadata in the dock. It includes classification, product, file size, and a shortened SHA-256 digest.
6. Refresh the browser or restart the API, then re-open the saved case to confirm metadata and files persist.

The backend streams upload data to disk and computes SHA-256 while receiving it. It does not put the whole file in SQLite. The browser stores only the active case and upload retry bookkeeping; it does not persist file bytes.

## 6. Where data is stored

Default location: `<project-root>/data/` (ignored by Git). Typical layout:

```text
data/
├── middleware-atlas.db
├── artifacts/<case-id>/<artifact-id>/original
├── work/
└── reports/
```

`config/default.toml` is the primary non-secret config file. Supported environment overrides and path behavior are described in [configuration.md](configuration.md). The default listener is loopback-only (`127.0.0.1:8080`), and the service currently has no user authentication or multi-user authorization. **Do not expose it to a network.**

## 7. Developer checks

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

Run all checks after UI, configuration, schema, or API changes. Commit `Cargo.lock` and `apps/web/package-lock.json`; do not commit `data/`, `.env`, `.env.local`, customer artifacts, dumps, generated reports, or local backup files.

## 8. Current boundaries (important)

- **Implemented now:** case CRUD endpoints, product-aware file metadata, streaming artifact upload, size limit, SHA-256, local filesystem storage, SQLite migrations, and the case-intake/persistence UI.
- **Not connected yet:** Dify workflow/chat invocation, real diagnostic analyzer jobs, Eclipse MAT execution, report/evidence explorer, and end-to-end root-cause analysis.
- `ATLAS_DIFY_*` and `ATLAS_MAT_*` entries in `.env.example` are configuration placeholders at this stage; they do not mean those integrations are active.

Further reading: [Component map](components.md), [Architecture](architecture.md), [Configuration](configuration.md), and [Persistence/API](persistence.md).
