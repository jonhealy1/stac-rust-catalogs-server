// src/handlers.rs
use crate::dto::CreateOrLinkPayload;
use crate::hierarchy::{HierarchyIndex, NodeKind};
use crate::links::LinkEngine;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use stac::Catalog;
use stac_api::{ItemCollection, Search};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub base_url: String,
    pub hierarchy: RwLock<HierarchyIndex>,
    pub links: LinkEngine,
}

#[derive(Deserialize)]
pub struct ChildrenQuery {
    pub r#type: Option<String>, // Filter by "Catalog" or "Collection"
}

// --- Discovery Handlers ---

pub async fn list_catalogs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "catalogs": [],
        "links": [
            { "rel": "self", "href": format!("{}/catalogs", state.base_url) },
            { "rel": "root", "href": state.base_url }
        ]
    }))
}

pub async fn get_catalog_children(
    Path(catalog_id): Path<String>,
    Query(_query): Query<ChildrenQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let _hierarchy = state.hierarchy.read().await;
    Json(json!({
        "children": [],
        "links": [
            { "rel": "self", "href": format!("{}/catalogs/{catalog_id}/children", state.base_url) },
            { "rel": "root", "href": state.base_url },
            { "rel": "parent", "href": format!("{}/catalogs/{catalog_id}", state.base_url) }
        ]
    }))
}

// --- Scoped Item Search ---

pub async fn scoped_search_post(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(mut search): Json<Search>,
) -> Result<Json<ItemCollection>, ApiError> {
    let hierarchy = state.hierarchy.read().await;

    // 1. Resolve all descendant collection IDs in the sub-catalog DAG
    let allowed_collections = hierarchy.get_descendant_collections(&catalog_id);
    if allowed_collections.is_empty() {
        return Ok(Json(ItemCollection::default()));
    }

    // 2. Security & Intersection: Enforce scope boundaries on search payload
    if search.collections.is_empty() {
        // No filter provided — restrict to all descendants of this catalog
        search.collections = allowed_collections.into_iter().collect();
    } else {
        // Intersect requested collections with catalog's allowed descendants
        search.collections.retain(|c| allowed_collections.contains(c));
        if search.collections.is_empty() {
            return Ok(Json(ItemCollection::default()));
        }
    }

    // 3. Query OpenSearch/Elasticsearch using the resolved `collections` filter list
    // (Database execution logic goes here)

    Ok(Json(ItemCollection::default()))
}

// --- Transaction Handlers ---

pub async fn link_or_create_sub_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrLinkPayload<Catalog>>,
) -> Result<impl IntoResponse, ApiError> {
    let mut hierarchy = state.hierarchy.write().await;

    match payload {
        CreateOrLinkPayload::LinkReference { id } => {
            // Mode B: Link existing sub-catalog
            hierarchy.link(&id, &catalog_id, NodeKind::Catalog);
            Ok((
                StatusCode::OK,
                Json(json!({"message": "Catalog linked successfully"})),
            ))
        }
        CreateOrLinkPayload::FullResource(new_catalog) => {
            // Mode A: Create new sub-catalog
            hierarchy.set_parents(new_catalog.id.clone(), vec![catalog_id], NodeKind::Catalog);
            // Save new_catalog to DB...
            Ok((
                StatusCode::CREATED,
                Json(json!(new_catalog)),
            ))
        }
    }
}

pub async fn disband_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, ApiError> {
    let mut hierarchy = state.hierarchy.write().await;

    // Safety Disband: Unlink direct children only and auto-adopt orphans to Root.
    // Only direct children have `catalog_id` as an actual parent; passing a non-parent
    // to unlink_and_adopt would be a no-op on the parent link and mis-adopt grandchildren.
    let direct_children = hierarchy.get_children(&catalog_id);
    for child in direct_children {
        hierarchy.unlink_and_adopt(&child, &catalog_id);
    }

    // Remove the catalog itself from its own parents (no adoption — it is being deleted)
    hierarchy.remove_node(&catalog_id);

    // Delete catalog metadata from DB (never deletes child collections or items)
    Ok(StatusCode::NO_CONTENT)
}

// --- Error Handling ---

pub enum ApiError {
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound(id) => (StatusCode::NOT_FOUND, format!("Resource '{id}' not found")),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(json!({ "code": status.as_u16(), "description": msg }))).into_response()
    }
}
