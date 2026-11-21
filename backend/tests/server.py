"""Runs the server under test as a real OS process — the Python (FastAPI)
backend or the Rust rewrite, picked by AURUM_TEST_SERVER=python|rust.

Both implement the same process contract, so nothing else in the suite
knows which one it's talking to:

    migrate   create/upgrade the schema of AURUM_POSTGRES_DB
    seed      default categories/account/app_settings row (idempotent)
    serve     HTTP on AURUM_HOST:AURUM_PORT, API under /api

configured purely through environment variables (AURUM_POSTGRES_*,
AURUM_DEFAULT_CURRENCY, AURUM_CORS_ORIGINS, AURUM_COINGECKO_API_KEY,
AURUM_ENABLE_DOCS, AURUM_HOST, AURUM_PORT, AURUM_NBP_BASE_URL,
AURUM_COINGECKO_BASE_URL). See tests/README.md.
"""
from __future__ import annotations

import os
import socket
import subprocess
import sys
import time
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path

import httpx
from dotenv import dotenv_values

BACKEND_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = BACKEND_DIR.parent
# Overridable so several suites (e.g. parallel porting work) can run side by
# side against the same Postgres without dropping each other's database.
TEST_DB_NAME = os.environ.get("AURUM_TEST_DB_NAME", "aurum_test")
SERVER_KIND = os.environ.get("AURUM_TEST_SERVER", "python").strip().lower() or "python"
STARTUP_TIMEOUT_SECONDS = 60

if SERVER_KIND not in {"python", "rust"}:
    raise RuntimeError(f"AURUM_TEST_SERVER must be 'python' or 'rust', got {SERVER_KIND!r}")


@dataclass(frozen=True)
class DbConfig:
    user: str
    password: str
    host: str
    port: int

    @classmethod
    def from_environment(cls) -> "DbConfig":
        """Same precedence the app itself uses (pydantic-settings): real
        environment first, then backend/.env, then app/core/config.py's
        defaults — so a local .env that already points the app at Postgres
        keeps working for the tests too."""
        dotenv = dotenv_values(BACKEND_DIR / ".env") if (BACKEND_DIR / ".env").exists() else {}

        def value(name: str, default: str) -> str:
            key = f"AURUM_POSTGRES_{name}"
            return os.environ.get(key) or dotenv.get(key) or default

        return cls(
            user=value("USER", "aurum"),
            password=value("PASSWORD", "aurum"),
            host=value("HOST", "db"),
            port=int(value("PORT", "5432")),
        )

    def dsn(self, db_name: str) -> str:
        return f"postgresql://{self.user}:{self.password}@{self.host}:{self.port}/{db_name}"

    def sqlalchemy_url(self, db_name: str) -> str:
        return f"postgresql+asyncpg://{self.user}:{self.password}@{self.host}:{self.port}/{db_name}"

    def env(self, db_name: str) -> dict[str, str]:
        return {
            "AURUM_POSTGRES_USER": self.user,
            "AURUM_POSTGRES_PASSWORD": self.password,
            "AURUM_POSTGRES_HOST": self.host,
            "AURUM_POSTGRES_PORT": str(self.port),
            "AURUM_POSTGRES_DB": db_name,
        }


def rust_binary() -> Path:
    return Path(os.environ.get("AURUM_SERVER_BIN") or REPO_ROOT / "target" / "debug" / "aurum-server")


def _command(subcommand: str, env: Mapping[str, str]) -> tuple[list[str], Path | None]:
    """The argv (and, if it matters, cwd) for one contract subcommand.
    cwd=None means "the run's scratch directory", deliberately *not*
    backend/: the Python app reads backend/.env from its cwd, and a
    developer's real .env (real CoinGecko key, CORS origins, ...) must not
    leak into a server whose config the tests control."""
    if SERVER_KIND == "rust":
        return [str(rust_binary()), subcommand], None
    if subcommand == "migrate":
        # alembic.ini's script_location is relative to the cwd. Only the
        # AURUM_POSTGRES_* values matter here and those are always passed
        # explicitly, so backend/.env can't redirect it.
        return [sys.executable, "-m", "alembic", "upgrade", "head"], BACKEND_DIR
    if subcommand == "seed":
        return [sys.executable, "-m", "app.db.seed"], None
    if subcommand == "serve":
        return [
            sys.executable,
            "-m",
            "uvicorn",
            "app.main:app",
            "--app-dir",
            str(BACKEND_DIR),
            "--host",
            env["AURUM_HOST"],
            "--port",
            env["AURUM_PORT"],
        ], None
    raise ValueError(f"unknown server subcommand {subcommand!r}")


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@dataclass
class ServerRunner:
    """Builds the environment for, and runs, every contract subcommand.

    `base_env` is what the session's main server gets; `overrides` on any
    call layer on top of it, where a value of None means "leave the
    variable unset" (e.g. to exercise a shipped default)."""

    workdir: Path
    base_env: dict[str, str]
    _log_counter: int = field(default=0, init=False)

    def env(self, overrides: Mapping[str, str | None] | None = None) -> dict[str, str]:
        # Nothing AURUM_* from the caller's shell reaches the server unless
        # put there on purpose — only the explicit config below does.
        env = {key: value for key, value in os.environ.items() if not key.startswith("AURUM_")}
        env.update(self.base_env)
        for key, value in (overrides or {}).items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = value
        # The fake providers and the health check live on 127.0.0.1 — a
        # proxy configured in the shell must never sit in between.
        env["NO_PROXY"] = env["no_proxy"] = ",".join(
            filter(None, ["127.0.0.1", "localhost", env.get("NO_PROXY") or env.get("no_proxy")])
        )
        if SERVER_KIND == "python":
            env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(BACKEND_DIR), env.get("PYTHONPATH")]))
        return env

    def run(self, subcommand: str, overrides: Mapping[str, str | None] | None = None) -> None:
        """Runs a one-shot subcommand (migrate/seed) to completion."""
        env = self.env(overrides)
        argv, cwd = _command(subcommand, env)
        result = subprocess.run(argv, cwd=cwd or self.workdir, env=env, capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(
                f"`{' '.join(argv)}` failed with exit code {result.returncode}\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )

    def start(self, overrides: Mapping[str, str | None] | None = None) -> "RunningServer":
        """Starts `serve` on a free port and waits until /api/health is up."""
        port = free_port()
        env = self.env({"AURUM_HOST": "127.0.0.1", "AURUM_PORT": str(port), **(overrides or {})})
        argv, cwd = _command("serve", env)
        self._log_counter += 1
        log_path = self.workdir / f"server-{self._log_counter}.log"
        log_file = log_path.open("w")
        process = subprocess.Popen(
            argv, cwd=cwd or self.workdir, env=env, stdout=log_file, stderr=subprocess.STDOUT
        )
        server = RunningServer(port=port, process=process, log_path=log_path, _log_file=log_file)
        try:
            server.wait_until_healthy()
        except BaseException:
            server.stop()
            raise
        return server


@dataclass
class RunningServer:
    port: int
    process: subprocess.Popen
    log_path: Path
    _log_file: object

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}/api"

    def log_tail(self, lines: int = 60) -> str:
        try:
            return "\n".join(self.log_path.read_text(errors="replace").splitlines()[-lines:])
        except OSError:
            return "<no log>"

    def wait_until_healthy(self) -> None:
        deadline = time.monotonic() + STARTUP_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(
                    f"server exited with code {self.process.returncode} before becoming healthy\n{self.log_tail()}"
                )
            try:
                if httpx.get(f"{self.base_url}/health", timeout=1, trust_env=False).status_code == 200:
                    return
            except httpx.HTTPError:
                pass
            time.sleep(0.1)
        raise RuntimeError(f"server not healthy after {STARTUP_TIMEOUT_SECONDS}s\n{self.log_tail()}")

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self._log_file.close()
