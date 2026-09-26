// src/store.rs
use opensearch::{
    http::{
        transport::{SingleNodeConnectionPool, TransportBuilder},
        StatusCode, Url,
    },
    indices::{IndicesCreateParts, IndicesExistsParts},
    DeleteParts, GetParts, IndexParts, MgetParts, OpenSearch, SearchParts,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const ROOT_CATALOG_ID: &str = "root";

pub const HIERARCHY_INDEX: &str = "stac-hierarchy";
pub const CATALOGS_INDEX: &str = "stac-catalogs";
pub const COLLECTIONS_INDEX: &str = "stac-collections";
pub const ITEMS_INDEX: &str = "stac-items";

/// Safety cap on DAG depth when resolving descendants.
const MAX_DESCENDANT_DEPTH: usize = 25;
/// Cap on children fetched per query (proper pagination is future work).
const MAX_CHILDREN: usize = 10_000;

/// STAC node type tracked in the hierarchy DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Catalog,
    Collection,
}

/// One document per node in `stac-hierarchy`, keyed by resource id.
/// Children are derived via `term` queries on `parents`, so link/unlink
/// only ever touches the child's document — no materialized DAG updates.
#[derive(Debug, Serialize, Deserialize)]
struct HierarchyNode {
    kind: NodeKind,
    #[serde(default)]
    parents: Vec<String>,
}

/// A hierarchy node as returned by children queries (id comes from `_id`).
#[derive(Debug)]
pub struct ChildNode {
    pub id: String,
    pub kind: NodeKind,
    pub parents: Vec<String>,
}

#[derive(Clone)]
pub struct Store {
    client: OpenSearch,
}

impl Store {
    pub fn connect(url: &str) -> Result<Self, opensearch::Error> {
        let url = Url::parse(url).expect("Invalid OPENSEARCH_URL");
        let pool = SingleNodeConnectionPool::new(url);
        let transport = TransportBuilder::new(pool).build()?;
        Ok(Self {
            client: OpenSearch::new(transport),
        })
    }

    /// Create the STAC indices on startup if they don't already exist.
    pub async fn ensure_indices(&self) -> Result<(), opensearch::Error> {
        let indices = [
            (
                HIERARCHY_INDEX,
                json!({"properties": {
                    "kind": {"type": "keyword"},
                    "parents": {"type": "keyword"}
                }}),
            ),
            (
                CATALOGS_INDEX,
                json!({"properties": {"id": {"type": "keyword"}}}),
            ),
            (
                COLLECTIONS_INDEX,
                json!({"properties": {"id": {"type": "keyword"}}}),
            ),
            (
                ITEMS_INDEX,
                json!({"properties": {
                    "id": {"type": "keyword"},
                    "collection": {"type": "keyword"}
                }}),
            ),
        ];

        for (index, mappings) in indices {
            let exists = self
                .client
                .indices()
                .exists(IndicesExistsParts::Index(&[index]))
                .send()
                .await?;
            if !exists.status_code().is_success() {
                self.client
                    .indices()
                    .create(IndicesCreateParts::Index(index))
                    // 0 replicas: dev default is single-node; without this
                    // every index sits yellow with unassigned shards.
                    .body(json!({
                        "settings": {"number_of_replicas": 0},
                        "mappings": mappings
                    }))
                    .send()
                    .await?;
            }
        }
        Ok(())
    }

    // --- Hierarchy DAG ---

    /// Replace a node's parent set (Mode A create / full update).
    /// Orphan safety: an empty parent set is adopted by root.
    pub async fn set_parents(
        &self,
        node_id: &str,
        parent_ids: Vec<String>,
        kind: NodeKind,
    ) -> Result<(), opensearch::Error> {
        let mut parents = parent_ids;
        if parents.is_empty() {
            parents.push(ROOT_CATALOG_ID.to_string());
        }
        self.put_node(node_id, &HierarchyNode { kind, parents })
            .await
    }

    /// Add a single parent to a node (Mode B linking).
    /// Keeps the node's existing kind if it was already registered.
    pub async fn link(
        &self,
        child_id: &str,
        parent_id: &str,
        kind: NodeKind,
    ) -> Result<(), opensearch::Error> {
        let mut node = self.get_node(child_id).await?.unwrap_or(HierarchyNode {
            kind,
            parents: vec![ROOT_CATALOG_ID.to_string()],
        });
        apply_link(&mut node.parents, parent_id);
        self.put_node(child_id, &node).await
    }

    /// Unlink a child from a parent; adopts the child under root if orphaned.
    pub async fn unlink_and_adopt(
        &self,
        child_id: &str,
        parent_id: &str,
    ) -> Result<(), opensearch::Error> {
        if let Some(mut node) = self.get_node(child_id).await? {
            apply_unlink(&mut node.parents, parent_id);
            self.put_node(child_id, &node).await?;
        }
        Ok(())
    }

    /// Delete a node outright (disband). Since children are derived from
    /// `parents` queries, deleting the doc detaches it from every parent.
    /// Callers must unlink the node's own children first.
    pub async fn remove_node(&self, node_id: &str) -> Result<(), opensearch::Error> {
        self.client
            .delete(DeleteParts::IndexId(HIERARCHY_INDEX, node_id))
            .refresh(opensearch::params::Refresh::WaitFor)
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_parents(&self, node_id: &str) -> Result<Vec<String>, opensearch::Error> {
        Ok(self
            .get_node(node_id)
            .await?
            .map(|n| n.parents)
            .unwrap_or_default())
    }

    /// Direct child ids of a node (reverse `term` lookup on `parents`).
    pub async fn get_children(&self, node_id: &str) -> Result<Vec<String>, opensearch::Error> {
        Ok(self
            .search_children(node_id, None)
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect())
    }

    /// Direct child ids of a node filtered by kind.
    pub async fn get_children_by_kind(
        &self,
        node_id: &str,
        kind: NodeKind,
    ) -> Result<Vec<String>, opensearch::Error> {
        Ok(self
            .search_children(node_id, Some(kind))
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect())
    }

    /// Direct children with kind + parents — enough to render child links
    /// without a second round trip.
    pub async fn get_child_nodes(
        &self,
        node_id: &str,
        kind: Option<NodeKind>,
    ) -> Result<Vec<ChildNode>, opensearch::Error> {
        self.search_children(node_id, kind).await
    }

    async fn search_children(
        &self,
        node_id: &str,
        kind: Option<NodeKind>,
    ) -> Result<Vec<ChildNode>, opensearch::Error> {
        let mut must = vec![json!({"term": {"parents": node_id}})];
        if let Some(kind) = kind {
            must.push(json!({"term": {"kind": kind}}));
        }
        let resp = self
            .client
            .search(SearchParts::Index(&[HIERARCHY_INDEX]))
            .body(json!({
                "size": MAX_CHILDREN,
                "query": {"bool": {"must": must}}
            }))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        Ok(body["hits"]["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|hit| {
                let node: HierarchyNode =
                    serde_json::from_value(hit["_source"].clone()).ok()?;
                Some(ChildNode {
                    id: hit["_id"].as_str()?.to_string(),
                    kind: node.kind,
                    parents: node.parents,
                })
            })
            .collect())
    }

    /// Parents of many nodes in one mget (for link rendering on lists).
    pub async fn get_parents_many(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, Vec<String>>, opensearch::Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let resp = self
            .client
            .mget(MgetParts::Index(HIERARCHY_INDEX))
            .body(json!({"ids": ids}))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        Ok(body["docs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["found"] == true)
            .filter_map(|d| {
                let id = d["_id"].as_str()?.to_string();
                let node: HierarchyNode = serde_json::from_value(d["_source"].clone()).ok()?;
                Some((id, node.parents))
            })
            .collect())
    }

    /// Resolve descendant COLLECTION ids for scoped search via level-wise
    /// BFS over `parents`. Depth-capped and cycle-safe.
    pub async fn get_descendant_collections(
        &self,
        root_id: &str,
    ) -> Result<HashSet<String>, opensearch::Error> {
        let mut visited: HashSet<String> = [root_id.to_string()].into_iter().collect();
        let mut frontier = vec![root_id.to_string()];
        let mut collections = HashSet::new();

        for _ in 0..MAX_DESCENDANT_DEPTH {
            if frontier.is_empty() {
                break;
            }
            let resp = self
                .client
                .search(SearchParts::Index(&[HIERARCHY_INDEX]))
                .body(json!({
                    "size": MAX_CHILDREN,
                    "query": {"terms": {"parents": frontier}}
                }))
                .send()
                .await?;
            let body = resp.json::<Value>().await?;
            let mut next = Vec::new();
            if let Some(hits) = body["hits"]["hits"].as_array() {
                for hit in hits {
                    let id = hit["_id"].as_str().unwrap_or_default().to_string();
                    if !visited.insert(id.clone()) {
                        continue;
                    }
                    let kind =
                        serde_json::from_value::<NodeKind>(hit["_source"]["kind"].clone()).ok();
                    if kind == Some(NodeKind::Collection) {
                        collections.insert(id.clone());
                    }
                    next.push(id);
                }
            }
            frontier = next;
        }
        Ok(collections)
    }

    // --- STAC document storage ---

    pub async fn index_document(
        &self,
        index: &str,
        id: &str,
        doc: impl Serialize,
    ) -> Result<(), opensearch::Error> {
        self.client
            .index(IndexParts::IndexId(index, id))
            .body(doc)
            .refresh(opensearch::params::Refresh::WaitFor)
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_document(
        &self,
        index: &str,
        id: &str,
    ) -> Result<Option<Value>, opensearch::Error> {
        let resp = self
            .client
            .get(GetParts::IndexId(index, id))
            .send()
            .await?;
        if resp.status_code() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = resp.json::<Value>().await?;
        Ok(Some(body["_source"].clone()))
    }

    pub async fn delete_document(&self, index: &str, id: &str) -> Result<(), opensearch::Error> {
        self.client
            .delete(DeleteParts::IndexId(index, id))
            .refresh(opensearch::params::Refresh::WaitFor)
            .send()
            .await?;
        Ok(())
    }

    /// Fetch many documents by id (missing ids are skipped).
    pub async fn get_documents(
        &self,
        index: &str,
        ids: &[String],
    ) -> Result<Vec<Value>, opensearch::Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self
            .client
            .mget(MgetParts::Index(index))
            .body(json!({"ids": ids}))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        Ok(body["docs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["found"] == true)
            .map(|d| d["_source"].clone())
            .collect())
    }

    /// Item search scoped to a set of collections.
    /// Returns (item docs, total_matched) — sources pass through verbatim
    /// since ItemCollection supports the fields extension. Full Search->DSL
    /// translation (bbox, datetime, intersects, sortby) is future work.
    pub async fn search_items(
        &self,
        collections: &[String],
        limit: u64,
    ) -> Result<(Vec<serde_json::Map<String, Value>>, u64), opensearch::Error> {
        let resp = self
            .client
            .search(SearchParts::Index(&[ITEMS_INDEX]))
            .body(json!({
                "size": limit,
                "track_total_hits": true,
                "query": {"terms": {"collection": collections}}
            }))
            .send()
            .await?;
        let body = resp.json::<Value>().await?;
        let total = body["hits"]["total"]["value"].as_u64().unwrap_or(0);
        let items = body["hits"]["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h["_source"].as_object().cloned())
            .collect();
        Ok((items, total))
    }

    // --- node doc helpers ---

    async fn get_node(&self, node_id: &str) -> Result<Option<HierarchyNode>, opensearch::Error> {
        let resp = self
            .client
            .get(GetParts::IndexId(HIERARCHY_INDEX, node_id))
            .send()
            .await?;
        if resp.status_code() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = resp.json::<Value>().await?;
        Ok(serde_json::from_value(body["_source"].clone()).ok())
    }

    async fn put_node(&self, node_id: &str, node: &HierarchyNode) -> Result<(), opensearch::Error> {
        self.client
            .index(IndexParts::IndexId(HIERARCHY_INDEX, node_id))
            .body(node)
            .refresh(opensearch::params::Refresh::WaitFor)
            .send()
            .await?;
        Ok(())
    }
}

/// Mode B link semantics on a node's parent list: linking to a real
/// catalog drops the implicit root parent; duplicates are ignored.
fn apply_link(parents: &mut Vec<String>, parent_id: &str) {
    if parent_id != ROOT_CATALOG_ID {
        parents.retain(|p| p != ROOT_CATALOG_ID);
    }
    if !parents.iter().any(|p| p == parent_id) {
        parents.push(parent_id.to_string());
    }
}

/// Unlink semantics: removing the last parent triggers root adoption.
fn apply_unlink(parents: &mut Vec<String>, parent_id: &str) {
    parents.retain(|p| p != parent_id);
    if parents.is_empty() {
        parents.push(ROOT_CATALOG_ID.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_drops_implicit_root() {
        let mut parents = vec![ROOT_CATALOG_ID.to_string()];
        apply_link(&mut parents, "real-parent");
        assert_eq!(parents, vec!["real-parent"]);
    }

    #[test]
    fn test_link_keeps_explicit_parents() {
        let mut parents = vec!["a".to_string(), ROOT_CATALOG_ID.to_string()];
        apply_link(&mut parents, "b");
        assert_eq!(parents, vec!["a", "b"]);
    }

    #[test]
    fn test_unlink_adopts_orphan_to_root() {
        let mut parents = vec!["parent".to_string()];
        apply_unlink(&mut parents, "parent");
        assert_eq!(parents, vec![ROOT_CATALOG_ID]);
    }

    #[test]
    fn test_unlink_keeps_remaining_parents() {
        let mut parents = vec!["a".to_string(), "b".to_string()];
        apply_unlink(&mut parents, "a");
        assert_eq!(parents, vec!["b"]);
    }
}
