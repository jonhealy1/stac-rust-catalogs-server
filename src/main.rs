// src/main.rs
mod dto;
mod handlers;
mod hierarchy;
mod links;

use axum::{
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use handlers::*;
use hierarchy::HierarchyIndex;
use links::LinkEngine;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let base_url = "http://localhost:3000";
    let state = Arc::new(AppState {
        base_url: base_url.to_string(),
        hierarchy: RwLock::new(HierarchyIndex::new()),
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

async fn create_root_catalog() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn get_catalog() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn update_catalog() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn unlink_sub_catalog() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn list_scoped_collections() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn link_or_create_scoped_collection() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn get_scoped_collection() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn update_scoped_collection() -> Json<serde_json::Value> {
    Json(json!({}))
}
async fn unlink_scoped_collection() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn scoped_search_get() -> Json<serde_json::Value> {
    Json(json!({}))
}
