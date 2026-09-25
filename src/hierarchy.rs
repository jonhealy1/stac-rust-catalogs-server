// src/hierarchy.rs
use std::collections::{HashMap, HashSet};

pub const ROOT_CATALOG_ID: &str = "root";

/// STAC node type tracked in the hierarchy DAG
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Catalog,
    Collection,
}

#[derive(Default, Debug)]
pub struct HierarchyIndex {
    // child_id -> set of parent_ids
    parents: HashMap<String, HashSet<String>>,
    // parent_id -> set of child_ids
    children: HashMap<String, HashSet<String>>,
    // node_id -> STAC resource type
    kinds: HashMap<String, NodeKind>,
}

impl HierarchyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update parent-child links for a given resource ID
    pub fn set_parents(&mut self, node_id: String, parent_ids: Vec<String>, kind: NodeKind) {
        self.kinds.insert(node_id.clone(), kind);
        // Clear previous relationships
        if let Some(old_parents) = self.parents.remove(&node_id) {
            for p in old_parents {
                if let Some(c) = self.children.get_mut(&p) {
                    c.remove(&node_id);
                }
            }
        }

        let mut parent_set: HashSet<String> = parent_ids.into_iter().collect();
        // Orphan Safety: If no parents exist, default adoption to ROOT
        if parent_set.is_empty() {
            parent_set.insert(ROOT_CATALOG_ID.to_string());
        }

        for parent_id in &parent_set {
            self.children
                .entry(parent_id.clone())
                .or_default()
                .insert(node_id.clone());
        }
        self.parents.insert(node_id, parent_set);
    }

    /// Add a single parent to a node (Mode B Linking)
    pub fn link(&mut self, child_id: &str, parent_id: &str, kind: NodeKind) {
        // Don't clobber an existing type on re-link
        self.kinds.entry(child_id.to_string()).or_insert(kind);
        let parents = self.parents.entry(child_id.to_string()).or_default();
        // Remove default root parent if present and linking to a real sub-catalog
        if parent_id != ROOT_CATALOG_ID && parents.remove(ROOT_CATALOG_ID) {
            if let Some(root_children) = self.children.get_mut(ROOT_CATALOG_ID) {
                root_children.remove(child_id);
            }
        }
        parents.insert(parent_id.to_string());

        self.children
            .entry(parent_id.to_string())
            .or_default()
            .insert(child_id.to_string());
    }

    /// Remove a node from the index entirely: unlinks it from all of its
    /// parents WITHOUT triggering root adoption (used when a catalog is
    /// disbanded). Direct children are untouched — unlink them first.
    pub fn remove_node(&mut self, node_id: &str) {
        self.kinds.remove(node_id);
        if let Some(parents) = self.parents.remove(node_id) {
            for parent_id in parents {
                if let Some(children) = self.children.get_mut(&parent_id) {
                    children.remove(node_id);
                }
            }
        }
    }

    /// Unlink a child from a parent and apply Root Adoption if orphaned
    pub fn unlink_and_adopt(&mut self, child_id: &str, parent_id: &str) {
        if let Some(parents) = self.parents.get_mut(child_id) {
            parents.remove(parent_id);
            // Automatic Adoption by Root if zero parents remain
            if parents.is_empty() {
                parents.insert(ROOT_CATALOG_ID.to_string());
                self.children
                    .entry(ROOT_CATALOG_ID.to_string())
                    .or_default()
                    .insert(child_id.to_string());
            }
        }
        if let Some(children) = self.children.get_mut(parent_id) {
            children.remove(child_id);
        }
    }

    /// Resolve ALL descendant node IDs recursively (catalogs AND collections)
    pub fn get_descendants(&self, root_id: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut queue = vec![root_id.to_string()];

        while let Some(current) = queue.pop() {
            if let Some(children) = self.children.get(&current) {
                for child in children {
                    if visited.insert(child.clone()) {
                        queue.push(child.clone());
                    }
                }
            }
        }
        visited
    }

    /// Resolve descendant COLLECTION ids only — the ids that are valid in a
    /// scoped search `collections` filter. Sub-catalog ids are excluded.
    pub fn get_descendant_collections(&self, root_id: &str) -> HashSet<String> {
        self.get_descendants(root_id)
            .into_iter()
            .filter(|id| self.kinds.get(id) == Some(&NodeKind::Collection))
            .collect()
    }

    pub fn get_parents(&self, node_id: &str) -> Vec<String> {
        self.parents
            .get(node_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Returns only the direct children of a given node
    pub fn get_children(&self, node_id: &str) -> Vec<String> {
        self.children
            .get(node_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_parents_orphan_adoption() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents("child".to_string(), vec![], NodeKind::Catalog);
        let parents = idx.get_parents("child");
        assert!(parents.contains(&ROOT_CATALOG_ID.to_string()));
    }

    #[test]
    fn test_link_and_descendants() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents("child1".to_string(), vec!["parent".to_string()], NodeKind::Catalog);
        idx.set_parents(
            "child2".to_string(),
            vec!["child1".to_string()],
            NodeKind::Catalog,
        );
        let descendants = idx.get_descendants("parent");
        assert!(descendants.contains("child1"));
        assert!(descendants.contains("child2"));
    }

    #[test]
    fn test_unlink_and_adopt() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents(
            "child".to_string(),
            vec!["parent".to_string()],
            NodeKind::Catalog,
        );
        idx.unlink_and_adopt("child", "parent");
        let parents = idx.get_parents("child");
        assert!(parents.contains(&ROOT_CATALOG_ID.to_string()));
    }

    #[test]
    fn test_link_removes_root_ghost_child() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents("child".to_string(), vec![], NodeKind::Catalog);
        assert!(idx.get_children(ROOT_CATALOG_ID).contains(&"child".to_string()));
        idx.link("child", "real-parent", NodeKind::Catalog);
        assert!(!idx.get_children(ROOT_CATALOG_ID).contains(&"child".to_string()));
        assert!(idx.get_children("real-parent").contains(&"child".to_string()));
    }

    #[test]
    fn test_remove_node_does_not_adopt() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents(
            "child".to_string(),
            vec!["parent".to_string()],
            NodeKind::Catalog,
        );
        idx.remove_node("child");
        assert!(idx.get_parents("child").is_empty());
        assert!(!idx.get_children("parent").contains(&"child".to_string()));
        assert!(!idx.get_children(ROOT_CATALOG_ID).contains(&"child".to_string()));
    }

    #[test]
    fn test_descendant_collections_excludes_catalogs() {
        let mut idx = HierarchyIndex::new();
        idx.set_parents(
            "sub-catalog".to_string(),
            vec!["root".to_string()],
            NodeKind::Catalog,
        );
        idx.set_parents(
            "col-1".to_string(),
            vec!["sub-catalog".to_string()],
            NodeKind::Collection,
        );
        let collections = idx.get_descendant_collections("root");
        assert!(collections.contains("col-1"));
        assert!(!collections.contains("sub-catalog"));
    }
}
