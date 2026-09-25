# stac-rust-catalogs-server

![StacLabs](https://github.com/StacLabs/.github/raw/main/profile/staclabs-orange-banner.png)

A STAC API Opensearch server built with Rust

## Contents

- [What is this?](#what-is-this)
- [API Routes](#api-routes)
- [Getting Started (coming from Python?)](#getting-started-coming-from-python)

## What is this?

A **hybrid** of the STAC API core spec and the [multi-tenant-catalogs extension](https://github.com/StacLabs/multi-tenant-catalogs): catalogs form a poly-hierarchy (a DAG — a catalog or collection can live under multiple parents), and every STAC capability is exposed *scoped* under `/catalogs/{catalog_id}` rather than only at the API root. See [API Routes](#api-routes) for the full surface.

The catalog DAG is stored in OpenSearch as one `{kind, parents}` document per node (`stac-hierarchy` index) — children are derived via `term` queries on `parents`, so linking a shared resource is a single-document write, and orphans are automatically adopted under `root`.

**Beyond the spec:** this project also exposes full STAC *transaction* capabilities inside the `/catalogs` scope (create/update/delete catalogs, collections, and items). That goes beyond what the multi-tenant-catalogs extension currently specifies — we're prototyping it here with the intent of feeding it back into the spec. All write endpoints are gated behind a flag:

```bash
ENABLE_TRANSACTIONS_EXTENSIONS=true   # unsets → every mutating route returns 405
```

When unset, only the read surface is mounted and the landing page `conformsTo` omits the transaction conformance URIs.

## API Routes

### Read surface — always mounted

| Method | Path | Description |
|---|---|---|
| GET | `/` | Landing page (`conformsTo`, links) |
| GET | `/catalogs` | List top-level catalogs |
| GET | `/catalogs/{catalog_id}` | Fetch a catalog |
| GET | `/catalogs/{catalog_id}/children` | List children (`?type=Catalog\|Collection`) |
| GET | `/catalogs/{catalog_id}/collections` | List collections in scope |
| GET | `/catalogs/{catalog_id}/collections/{collection_id}` | Fetch a scoped collection |
| GET | `/catalogs/{catalog_id}/collections/{collection_id}/items` | List items in a scoped collection |
| GET | `/catalogs/{catalog_id}/collections/{collection_id}/items/{item_id}` | Fetch a scoped item |
| GET/POST | `/catalogs/search` | Search across the whole catalogs registry (scope = `root`) |
| GET/POST | `/catalogs/{catalog_id}/search` | Scoped search (`Search` body intersected with descendants) |

### Transactions — require `ENABLE_TRANSACTIONS_EXTENSIONS`

| Method | Path | Description |
|---|---|---|
| POST | `/catalogs` | Create a root-level catalog |
| PUT | `/catalogs/{catalog_id}` | Update a catalog |
| DELETE | `/catalogs/{catalog_id}` | Disband (children adopted by `root`) |
| POST | `/catalogs/{catalog_id}/catalogs` | Create (Mode A) or link (Mode B `{"id": ...}`) a sub-catalog |
| DELETE | `/catalogs/{catalog_id}/catalogs/{sub_id}` | Unlink a sub-catalog |
| POST | `/catalogs/{catalog_id}/collections` | Create (Mode A) or link (Mode B) a collection |
| PUT | `/catalogs/{catalog_id}/collections/{collection_id}` | Update a collection |
| DELETE | `/catalogs/{catalog_id}/collections/{collection_id}` | Unlink a collection |
| POST | `/catalogs/{catalog_id}/collections/{collection_id}/items` | Create an item |
| PUT | `/catalogs/{catalog_id}/collections/{collection_id}/items/{item_id}` | Update an item |
| DELETE | `/catalogs/{catalog_id}/collections/{collection_id}/items/{item_id}` | Delete an item |

Scoped reads 404 when the target collection isn't inside the catalog's DAG. Item payloads whose `collection` field contradicts the path get a 400.

## Getting Started (coming from Python?)

Rust projects don't use `pip` or virtualenvs — dependencies live in `Cargo.toml` (like `pyproject.toml`) and are fetched automatically by `cargo`, the Rust build/package tool.

**1. Install the Rust toolchain** (gives you `cargo`, think "pip + interpreter in one"):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
cargo --version   # verify install
```

**2. Start the stack** — `compose.yml` runs the API, a single-node OpenSearch dev cluster, and Dashboards:

```bash
docker compose up -d          # API :3000, OpenSearch :9200, Dashboards :5601
```

**3. Or run the API locally** — iterate on Rust code while OpenSearch stays in Docker:

```bash
docker compose up -d opensearch
cargo run     # API on http://localhost:3000
```

Config via env vars: `OPENSEARCH_URL` (default `http://localhost:9200`), `ENABLE_TRANSACTIONS_EXTENSIONS` (enables all write endpoints; set in `compose.yml` by default).

Other handy commands: `cargo check` (fast type-check, no binary), `cargo test` (unit + integration tests — integration tests need OpenSearch running, and skip automatically if it's not), `cargo add <crate>` (add a dependency).
