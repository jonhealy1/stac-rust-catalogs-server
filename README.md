# stac-rust-catalogs-server
A STAC API Opensearch server built with Rust

## Getting Started (coming from Python?)

Rust projects don't use `pip` or virtualenvs — dependencies live in `Cargo.toml` (like `pyproject.toml`) and are fetched automatically by `cargo`, the Rust build/package tool.

**1. Install the Rust toolchain** (gives you `cargo`, think "pip + interpreter in one"):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
cargo --version   # verify install
```

**2. Build and run** — no venv needed, `cargo` resolves and downloads crates (e.g. `stac`, `stac-api`) on first build:

```bash
cargo build   # fetch deps + compile (like pip install -r requirements.txt)
cargo run     # starts the server on http://localhost:3000
```

Other handy commands: `cargo check` (fast type-check, no binary), `cargo test` (unit tests), `cargo add <crate>` (add a dependency).
