<!-- maintained-by: patch_project_docs.py -->
# Local development troubleshooting

## API cannot start / port 8080 is in use

- Ensure only one `cargo run -p atlas-api` process is using `127.0.0.1:8080`.
- Check `ATLAS_BIND_ADDR` in your local `.env` and `config/default.toml`.
- Run from the repository root so local `.env` loading is predictable.

## UI says the API is unavailable

1. In a terminal, run `curl -fsS http://127.0.0.1:8080/health`.
2. Ensure the browser endpoint is correct in `apps/web/.env.local`:
   `NEXT_PUBLIC_ATLAS_API_BASE_URL=http://127.0.0.1:8080`.
3. Restart `npm run dev` after modifying `.env.local`.
4. Check `ATLAS_WEB_ORIGIN` if the browser is being served from an origin other than `http://localhost:3000`.

## Upload fails or large files are rejected

- Check free disk space under the configured data directory.
- Review `[storage].max_upload_bytes` in `config/default.toml` or `ATLAS_MAX_UPLOAD_BYTES`. It is a **per-file** limit; default is 100 GiB. Set a smaller limit if appropriate for the machine.
- Keep the API and frontend running throughout the upload. Retry after reviewing the returned API error.
- Do not extract uploaded archives yet; the current API stores them as files and does not safely inspect or unpack them.

## TypeScript, ESLint, or Next.js build fails

```bash
cd apps/web
node --version             # should match .nvmrc
npm ci                     # clean install from the committed lockfile
npm run typecheck
npm run lint
npm run build
```

If `npm ci` fails because `package.json` and `package-lock.json` differ after a deliberate local dependency change, run `npm install`, run all checks, and review both files before committing. Do not use `npm audit fix --force` as a first response to a warning; inspect `npm audit` and peer dependency compatibility first.

## Rust compile or test fails

```bash
cd /path/to/middleware-atlas
rustc --version
cargo fmt --all -- --check
cargo test --workspace
```

The repository pins the Rust toolchain in `rust-toolchain.toml`. If rustup requests a toolchain download, allow that installation or install the pinned toolchain explicitly.

## Data not where expected

By default runtime data is `<project-root>/data`, not the directory from which the command was launched. See [configuration.md](configuration.md) for `ATLAS_DATA_DIR` and project-root rules. The root `/data/` path is ignored by Git; do not force-add it.

## Security reminders

- Do not expose the API beyond loopback while authentication and authorization are not implemented.
- Never upload real customer artifacts into an untrusted location or commit them to Git.
- Don't put API credentials in frontend `NEXT_PUBLIC_*` variables.
- Dify and MAT settings are placeholders until their integrations are implemented and tested.
