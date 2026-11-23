# Contributing to Aurum

Thanks for considering a contribution — code, docs, bug reports, and ideas are all welcome.

## Getting a dev environment running

The fastest path to a working environment is the same Docker Compose setup end users run:

```bash
cp .env.example .env
docker compose up -d --build
```

This gives you Postgres, the Rust backend (`crates/aurum-server`, with migrations applied automatically on boot), and the built frontend served through nginx at `http://localhost:3000`.

For active frontend development with hot reload:

```bash
cd frontend
npm install
npm run dev
```

The dev server proxies API calls to `http://localhost:8000` (override with `VITE_API_PROXY_TARGET`) — point it at the Dockerized backend above, or run the backend locally. You need a stable [Rust toolchain](https://rustup.rs) (1.88 or newer) and a Postgres 16 that is reachable from the host. The Compose database is not published to the host, so start a throwaway one (user, password and database `aurum` are the server's defaults):

```bash
docker run -d --name aurum-dev-db -p 5432:5432 \
  -e POSTGRES_USER=aurum -e POSTGRES_PASSWORD=aurum -e POSTGRES_DB=aurum postgres:16-alpine

# migrations and seed data are applied on start, then the API listens on :8000
AURUM_POSTGRES_HOST=127.0.0.1 cargo run -p aurum-server -- serve
```

The backend, the CLI (`aurum`), the HTTP client and the mobile FFI layer are one Cargo workspace (`crates/`); see [docs/adr](docs/adr) for how it is structured and [crates/aurum-server/PORTING.md](crates/aurum-server/PORTING.md) for the backend's conventions.

### Running the tests

```bash
# Rust: formatting, lints and every test of the workspace. The API tests of
# aurum-server need a Postgres whose user can CREATE DATABASE; each test gets
# its own throwaway database, your real `aurum` one is never touched.
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
AURUM_POSTGRES_HOST=127.0.0.1 AURUM_POSTGRES_PORT=5432 cargo test --workspace

# Frontend
cd frontend && npm run build && npm run test
```

CI runs the same commands (`.github/workflows/ci.yml`).

## Before opening a PR

- **Frontend:** `npm run build` (runs `tsc -b` then `vite build`) must pass with no type errors.
- **Backend:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must pass; new behaviour comes with a test in `crates/aurum-server/tests`. New tables or columns need a new numbered SQL migration in `crates/aurum-server/migrations/` (never edit an existing one). A new or changed endpoint also needs a matching `aurum` CLI command, its built-in skill and docs.
- **New user-facing text** goes through the translation system in `frontend/src/lib/i18n.ts` (both `ru` and `en` — the `en` object is typed against `ru`'s keys, so a missing translation is a build error, not a runtime surprise) rather than being hardcoded in a component.
- **Mobile:** check your change at a narrow viewport — Aurum is designed mobile-first.
- Keep PRs focused. A bug fix doesn't need an accompanying refactor.

Opening a PR pre-fills this checklist for you — see the [PR template](.github/PULL_REQUEST_TEMPLATE.md).

## Reporting bugs

Open an [Issue](../../issues) with steps to reproduce. For security issues, please use GitHub's private Security tab instead of a public issue.

## Proposing features

Open an [Issue](../../issues) first for anything that changes the data model or adds a new area of the app — it's a lot easier to align on the shape of a feature before code exists than to rework a finished PR.

## License

Aurum is licensed under [PolyForm Noncommercial 1.0.0](LICENSE), not a traditional OSI open source license — see the [README's License section](README.md#-license) for what that means in practice. By submitting a PR, you agree your contribution is licensed under the same terms as the rest of the project.
