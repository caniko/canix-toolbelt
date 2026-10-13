//! Cargo metadata comparison. Target expressions describe graph edges, not target compilation.

use super::*;
use std::collections::{BTreeSet, VecDeque};

/// Stable package identity including its distribution source.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageIdentity {
    /// Cargo package name.
    pub name: String,
    /// Full package version.
    pub version: String,
    /// Registry/Git source; absent for local paths.
    pub source: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Package {
    id: String,
    #[serde(flatten)]
    identity: PackageIdentity,
}
#[derive(Clone, Debug, Deserialize)]
struct Edge {
    pkg: String,
    dep_kinds: Vec<serde_json::Value>,
}
#[derive(Clone, Debug, Deserialize)]
struct Node {
    id: String,
    features: Vec<String>,
    deps: Vec<Edge>,
}
#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Document {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Option<Resolve>,
}

/// Validated resolved Cargo metadata, preserving aliases and target/kind evidence.
pub struct Metadata {
    packages: BTreeMap<String, Package>,
    nodes: BTreeMap<String, Node>,
    roots: Vec<String>,
}

/// Immediate producer and shortest workspace-to-producer path, in the retained Python format.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Parent {
    /// Immediate dependent package name.
    pub parent: String,
    /// Immediate dependent version.
    pub version: String,
    /// Features enabled on that dependent.
    pub features: Vec<String>,
    /// Cargo dependency kind/target expressions; not execution evidence.
    pub dependency_kinds: Vec<serde_json::Value>,
    /// Shortest name@version path ending at the parent.
    pub path: Vec<String>,
}

/// Distribution-aware graph package delta.
#[derive(Debug, Serialize)]
pub struct Delta {
    /// Package identities no longer present.
    pub removed: Vec<PackageIdentity>,
    /// Newly selected package identities.
    pub added: Vec<PackageIdentity>,
}

impl Metadata {
    /// Parse metadata with a complete resolve graph. Missing and ambiguous nodes fail closed.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let document: Document = serde_json::from_slice(bytes).map_err(invalid_json)?;
        let resolve = document
            .resolve
            .ok_or_else(|| invalid("Cargo metadata has no resolve graph"))?;
        let count = document.packages.len();
        let packages: BTreeMap<_, _> = document
            .packages
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let node_count = resolve.nodes.len();
        let nodes: BTreeMap<_, _> = resolve
            .nodes
            .into_iter()
            .map(|n| (n.id.clone(), n))
            .collect();
        if packages.len() != count
            || nodes.len() != node_count
            || document.workspace_members.is_empty()
        {
            return Err(invalid(
                "metadata contains duplicate IDs or no workspace roots",
            ));
        }
        for node in nodes.values() {
            if !packages.contains_key(&node.id)
                || node
                    .deps
                    .iter()
                    .any(|edge| !packages.contains_key(&edge.pkg) || !nodes.contains_key(&edge.pkg))
            {
                return Err(invalid("resolve graph references an unknown package/node"));
            }
        }
        if document
            .workspace_members
            .iter()
            .any(|id| !nodes.contains_key(id))
        {
            return Err(invalid("workspace root is not resolved"));
        }
        // Preserve Cargo's root/edge order for the historical Python BFS tie-break.
        let roots = document.workspace_members;
        Ok(Self {
            packages,
            nodes,
            roots,
        })
    }

    fn paths(&self) -> BTreeMap<String, Vec<String>> {
        let mut paths: BTreeMap<_, _> = self
            .roots
            .iter()
            .map(|id| (id.clone(), vec![id.clone()]))
            .collect();
        let mut queue: VecDeque<_> = self.roots.iter().cloned().collect();
        while let Some(id) = queue.pop_front() {
            for child in self.nodes[&id].deps.iter().map(|edge| &edge.pkg) {
                if !paths.contains_key(child) {
                    let mut path = paths[&id].clone();
                    path.push(child.clone());
                    paths.insert(child.clone(), path);
                    queue.push_back(child.clone());
                }
            }
        }
        paths
    }

    /// Find all direct parents of selected name/version, retaining kind and target expressions.
    pub fn parents(&self, name: &str, version: Option<&str>) -> io::Result<Vec<Parent>> {
        let targets: BTreeSet<_> = self
            .packages
            .values()
            .filter(|p| p.identity.name == name && version.is_none_or(|v| p.identity.version == v))
            .map(|p| &p.id)
            .collect();
        let paths = self.paths();
        let mut parents = Vec::new();
        for node in self.nodes.values() {
            for edge in &node.deps {
                if targets.contains(&edge.pkg) {
                    let parent = &self.packages[&node.id].identity;
                    let path = paths
                        .get(&node.id)
                        .map(|path| {
                            path.iter()
                                .map(|id| {
                                    let package = &self.packages[id].identity;
                                    format!("{}@{}", package.name, package.version)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    parents.push(Parent {
                        parent: parent.name.clone(),
                        version: parent.version.clone(),
                        features: node.features.clone(),
                        dependency_kinds: edge.dep_kinds.clone(),
                        path,
                    });
                }
            }
        }
        Ok(parents)
    }

    /// Compare selected package identities, including source changes at the same version.
    pub fn delta(&self, after: &Self) -> Delta {
        let before: BTreeSet<_> = self.packages.values().map(|p| p.identity.clone()).collect();
        let after: BTreeSet<_> = after
            .packages
            .values()
            .map(|p| p.identity.clone())
            .collect();
        Delta {
            removed: before.difference(&after).cloned().collect(),
            added: after.difference(&before).cloned().collect(),
        }
    }
}
