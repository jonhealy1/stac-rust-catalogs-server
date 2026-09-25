// src/main.rs
mod dto;
mod handlers;
mod links;
mod store;

use axum::{
    routing::{delete, get, post},
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

    let state = Arc::new(AppState {
        base_url: base_url.to_string(),
        store,
        links: LinkEngine::new(base_url),
    });

    let app = Router::new()
        // --- Global Landing Page ---
        .route("/", get(root_landing_page))
        // --- Registry & Management Plane ---
        .route("/catalogs", get(list_catalogs).post(create_root_catalog))
        .route(
            "/catalogs/{catalog_id}",
            get(get_catalog)
                .put(update_catalog)
                .delete(disband_catalog),
        )
        // --- Children & Sub-Resources ---
        .route("/catalogs/{catalog_id}/children", get(get_catalog_children))
        .route(
            "/catalogs/{catalog_id}/catalogs",
            post(link_or_create_sub_catalog),
        )
        .route(
            "/catalogs/{catalog_id}/catalogs/{sub_id}",
            delete(unlink_sub_catalog),
        )
        // --- Scoped Collections ---
        .route(
            "/catalogs/{catalog_id}/collections",
            get(list_scoped_collections).post(link_or_create_scoped_collection),
        )
        .route(
            "/catalogs/{catalog_id}/collections/{collection_id}",
            get(get_scoped_collection)
                .put(update_scoped_collection)
                .delete(unlink_scoped_collection),
        )
        // --- Scoped Search Engine ---
        .route(
            "/catalogs/{catalog_id}/search",
            get(scoped_search_get).post(scoped_search_post),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    println!("STAC Multi-Tenant Catalogs API running on http://localhost:3000");

    axum::serve(listener, app).await.unwrap();
}

async fn root_landing_page() -> Json<serde_json::Value> {
    Json(json!({
        "stac_version": "1.0.0",
        "type": "Catalog",
        "id": "stac-multi-tenant-root",
        "title": "STAC API with Multi-Tenant Catalogs",
        "conformsTo": [
            "https://api.stacspec.org/v1.0.0/core",
            "https://api.stacspec.org/v1.0.0/multi-tenant-catalogs",
            "https://api.stacspec.org/v1.0.0/multi-tenant-catalogs/search",
            "https://api.stacspec.org/v1.0.0/multi-tenant-catalogs/transaction",
            "https://api.stacspec.org/v1.0.0/children",
            "https://api.stacspec.org/v1.0.0/children#type-filter"
        ],
        "links": [
            { "rel": "self", "type": "application/json", "href": "http://localhost:3000/" },
            { "rel": "data", "type": "application/json", "href": "http://localhost:3000/collections" },
            { "rel": "catalogs", "type": "application/json", "href": "http://localhost:3000/catalogs", "title": "Multi-Tenant Catalogs Registry" }
        ]
    }))
}

// TODO: map STAC GET-search params (GetSearch) onto the same OpenSearch path
async fn scoped_search_get() -> Json<serde_json::Value> {
    Json(json!({}))
}
