// src/handlers.rs
use crate::dto::CreateOrLinkPayload;
use crate::links::LinkEngine;
use crate::store::{NodeKind, Store, CATALOGS_INDEX, COLLECTIONS_INDEX, ROOT_CATALOG_ID};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use stac::{Catalog, Collection};
use stac_api::{ItemCollection, Search};
use std::sync::Arc;

pub struct AppState {
    pub base_url: String,
    pub store: Store,
    pub links: LinkEngine,
}

#[derive(Deserialize)]
pub struct ChildrenQuery {
    pub r#type: Option<String>, // Filter by "Catalog" or "Collection"
}

fn parse_kind(raw: Option<&str>) -> Option<NodeKind> {
    match raw {
        Some("Catalog") => Some(NodeKind::Catalog),
        Some("Collection") => Some(NodeKind::Collection),
        _ => None,
    }
}

// --- Discovery Handlers ---

pub async fn list_catalogs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let ids = state
        .store
        .get_children_by_kind(ROOT_CATALOG_ID, NodeKind::Catalog)
        .await?;
    let catalogs = state.store.get_documents(CATALOGS_INDEX, &ids).await?;
    Ok(Json(json!({
        "catalogs": catalogs,
        "links": [
            { "rel": "self", "href": format!("{}/catalogs", state.base_url) },
            { "rel": "root", "href": state.base_url }
        ]
    })))
}

pub async fn create_root_catalog(
    State(state): State<Arc<AppState>>,
    Json(catalog): Json<Catalog>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .set_parents(&catalog.id, vec![ROOT_CATALOG_ID.to_string()], NodeKind::Catalog)
        .await?;
    state
        .store
        .index_document(CATALOGS_INDEX, &catalog.id, &catalog)
        .await?;
    Ok((StatusCode::CREATED, Json(json!(catalog))))
}

pub async fn get_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .get_document(CATALOGS_INDEX, &catalog_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(catalog_id))
}

pub async fn update_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(catalog): Json<Catalog>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .index_document(CATALOGS_INDEX, &catalog_id, &catalog)
        .await?;
    Ok(Json(json!(catalog)))
}

pub async fn get_catalog_children(
    Path(catalog_id): Path<String>,
    Query(query): Query<ChildrenQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let children = match parse_kind(query.r#type.as_deref()) {
        Some(kind) => state.store.get_children_by_kind(&catalog_id, kind).await?,
        None => state.store.get_children(&catalog_id).await?,
    };
    Ok(Json(json!({
        "children": children,
        "links": [
            { "rel": "self", "href": format!("{}/catalogs/{catalog_id}/children", state.base_url) },
            { "rel": "root", "href": state.base_url },
            { "rel": "parent", "href": format!("{}/catalogs/{catalog_id}", state.base_url) }
        ]
    })))
}

// --- Scoped Item Search ---

pub async fn scoped_search_post(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(mut search): Json<Search>,
) -> Result<Json<ItemCollection>, ApiError> {
    // 1. Resolve all descendant collection IDs in the sub-catalog DAG
    let allowed_collections = state.store.get_descendant_collections(&catalog_id).await?;
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

    // 3. Query OpenSearch using the resolved `collections` filter list
    let limit = search.items.limit.unwrap_or(100);
    let (items, matched) = state.store.search_items(&search.collections, limit).await?;
    let mut collection = ItemCollection::default();
    collection.number_matched = Some(matched);
    collection.number_returned = Some(items.len() as u64);
    collection.items = items;
    Ok(Json(collection))
}

// --- Transaction Handlers ---

pub async fn link_or_create_sub_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrLinkPayload<Catalog>>,
) -> Result<impl IntoResponse, ApiError> {
    match payload {
        CreateOrLinkPayload::LinkReference { id } => {
            // Mode B: Link existing sub-catalog
            state.store.link(&id, &catalog_id, NodeKind::Catalog).await?;
            Ok((
                StatusCode::OK,
                Json(json!({"message": "Catalog linked successfully"})),
            ))
        }
        CreateOrLinkPayload::FullResource(new_catalog) => {
            // Mode A: Create new sub-catalog
            state
                .store
                .set_parents(&new_catalog.id, vec![catalog_id], NodeKind::Catalog)
                .await?;
            state
                .store
                .index_document(CATALOGS_INDEX, &new_catalog.id, &new_catalog)
                .await?;
            Ok((StatusCode::CREATED, Json(json!(new_catalog))))
        }
    }
}

pub async fn unlink_sub_catalog(
    Path((catalog_id, sub_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, ApiError> {
    // Unlink only; orphans are adopted by root rather than deleted
    state.store.unlink_and_adopt(&sub_id, &catalog_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_scoped_collections(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let ids = state
        .store
        .get_children_by_kind(&catalog_id, NodeKind::Collection)
        .await?;
    let collections = state.store.get_documents(COLLECTIONS_INDEX, &ids).await?;
    Ok(Json(json!({
        "collections": collections,
        "links": [
            { "rel": "self", "href": format!("{}/catalogs/{catalog_id}/collections", state.base_url) },
            { "rel": "root", "href": state.base_url },
            { "rel": "parent", "href": format!("{}/catalogs/{catalog_id}", state.base_url) }
        ]
    })))
}

pub async fn link_or_create_scoped_collection(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrLinkPayload<Collection>>,
) -> Result<impl IntoResponse, ApiError> {
    match payload {
        CreateOrLinkPayload::LinkReference { id } => {
            state
                .store
                .link(&id, &catalog_id, NodeKind::Collection)
                .await?;
            Ok((
                StatusCode::OK,
                Json(json!({"message": "Collection linked successfully"})),
            ))
        }
        CreateOrLinkPayload::FullResource(collection) => {
            state
                .store
                .set_parents(&collection.id, vec![catalog_id], NodeKind::Collection)
                .await?;
            state
                .store
                .index_document(COLLECTIONS_INDEX, &collection.id, &collection)
                .await?;
            Ok((StatusCode::CREATED, Json(json!(collection))))
        }
    }
}

pub async fn get_scoped_collection(
    Path((catalog_id, collection_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    // Scope enforcement: the collection must live inside this catalog's DAG
    let allowed = state.store.get_descendant_collections(&catalog_id).await?;
    if !allowed.contains(&collection_id) {
        return Err(ApiError::NotFound(collection_id));
    }
    state
        .store
        .get_document(COLLECTIONS_INDEX, &collection_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(collection_id))
}

pub async fn update_scoped_collection(
    Path((_catalog_id, collection_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    Json(collection): Json<Collection>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .index_document(COLLECTIONS_INDEX, &collection_id, &collection)
        .await?;
    Ok(Json(json!(collection)))
}

pub async fn unlink_scoped_collection(
    Path((catalog_id, collection_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .unlink_and_adopt(&collection_id, &catalog_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn disband_catalog(
    Path(catalog_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, ApiError> {
    // Safety Disband: Unlink direct children only and auto-adopt orphans to Root.
    let direct_children = state.store.get_children(&catalog_id).await?;
    for child in direct_children {
        state.store.unlink_and_adopt(&child, &catalog_id).await?;
    }

    // Deleting the node's doc detaches it from all parents (children are
    // derived from `parents` lookups — no reverse cleanup needed)
    state.store.remove_node(&catalog_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Error Handling ---

pub enum ApiError {
    NotFound(String),
    Internal(String),
}

impl From<opensearch::Error> for ApiError {
    fn from(e: opensearch::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
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
