// src/main.rs
mod dto;
mod handlers;
mod links;
mod store;

use axum::{
    extract::State,
    routing::{delete, get, post, put},
    Json, Router,
};
use handlers::*;
use links::LinkEngine;
use serde_json::json;
use std::sync::Arc;
use store::Store;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let base_url = "http://localhost:3000";
    let opensearch_url =
        std::env::var("OPENSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_string());
    let store = Store::connect(&opensearch_url).expect("Failed to connect to OpenSearch");
    store
        .ensure_indices()
        .await
        .expect("Failed to create STAC indices");

    let enable_transactions = std::env::var("ENABLE_TRANSACTIONS_EXTENSIONS")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false);

    let state = Arc::new(AppState {
        base_url: base_url.to_string(),
        store,
        links: LinkEngine::new(base_url),
        enable_transactions,
    });

    // Read-only surface — always registered
    let mut app = Router::new()
        // --- Global Landing Page ---
        .route("/", get(root_landing_page))
        // --- Registry & Management Plane ---
        .route("/catalogs", get(list_catalogs))
        .route("/catalogs/{catalog_id}", get(get_catalog))
        // --- Children & Sub-Resources ---
        .route("/catalogs/{catalog_id}/children", get(get_catalog_children))
        // --- Scoped Collections & Items ---
        .route(
            "/catalogs/{catalog_id}/collections",
            get(list_scoped_collections),
        )
        .route(
            "/catalogs/{catalog_id}/collections/{collection_id}",
            get(get_scoped_collection),
        )
        .route(
            "/catalogs/{catalog_id}/collections/{collection_id}/items",
            get(list_scoped_items),
        )
        .route(
            "/catalogs/{catalog_id}/collections/{collection_id}/items/{item_id}",
            get(get_scoped_item),
        )
        // --- Scoped Search Engine ---
        // /catalogs/search = whole-registry scope (everything under root)
        .route(
            "/catalogs/search",
            get(catalogs_search_get).post(catalogs_search_post),
        )
        .route(
            "/catalogs/{catalog_id}/search",
            get(scoped_search_get).post(scoped_search_post),
        );

    // Transaction surface — hybrid extension beyond the multi-tenant-catalogs
    // spec; only mounted when ENABLE_TRANSACTIONS_EXTENSIONS is set.
    if enable_transactions {
        app = app.merge(
            Router::new()
                .route("/catalogs", post(create_root_catalog))
                .route(
                    "/catalogs/{catalog_id}",
                    put(update_catalog).delete(disband_catalog),
                )
                .route(
                    "/catalogs/{catalog_id}/catalogs",
                    post(link_or_create_sub_catalog),
                )
                .route(
                    "/catalogs/{catalog_id}/catalogs/{sub_id}",
                    delete(unlink_sub_catalog),
                )
                .route(
                    "/catalogs/{catalog_id}/collections",
                    post(link_or_create_scoped_collection),
                )
                .route(
                    "/catalogs/{catalog_id}/collections/{collection_id}",
                    put(update_scoped_collection).delete(unlink_scoped_collection),
                )
                .route(
                    "/catalogs/{catalog_id}/collections/{collection_id}/items",
                    post(create_scoped_item),
                )
                .route(
                    "/catalogs/{catalog_id}/collections/{collection_id}/items/{item_id}",
                    put(update_scoped_item).delete(delete_scoped_item),
                ),
        );
        println!("Transaction extension ENABLED (ENABLE_TRANSACTIONS_EXTENSIONS)");
    }

    let app = app.with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    println!("STAC Multi-Tenant Catalogs API running on http://localhost:3000");

    axum::serve(listener, app).await.unwrap();
}

async fn root_landing_page(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut conforms_to = vec![
        "https://api.stacspec.org/v1.0.0/core",
        "https://api.stacspec.org/v1.0.0/multi-tenant-catalogs",
        "https://api.stacspec.org/v1.0.0/multi-tenant-catalogs/search",
        "https://api.stacspec.org/v1.0.0/children",
        "https://api.stacspec.org/v1.0.0/children#type-filter",
    ];
    if state.enable_transactions {
        conforms_to.push("https://api.stacspec.org/v1.0.0/multi-tenant-catalogs/transaction");
        conforms_to.push("https://api.stacspec.org/v1.0.0/ogcapi-features/extensions/transaction");
    }
    Json(json!({
        "stac_version": "1.0.0",
        "type": "Catalog",
        "id": "stac-multi-tenant-root",
        "title": "STAC API with Multi-Tenant Catalogs",
        "conformsTo": conforms_to,
        "links": [
            { "rel": "self", "type": "application/json", "href": "http://localhost:3000/" },
            { "rel": "data", "type": "application/json", "href": "http://localhost:3000/collections" },
            { "rel": "catalogs", "type": "application/json", "href": "http://localhost:3000/catalogs", "title": "Multi-Tenant Catalogs Registry" }
        ]
    }))
}
