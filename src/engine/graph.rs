use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::engine::context::{EdgeKind, MockDeclaration, MockKind};
use crate::rule::Hazard;

/// A node in the module graph.
#[derive(Debug, Clone)]
pub struct ModuleNode {
    pub path: PathBuf,
    pub hazards: Vec<Hazard>,
    pub is_test_file: bool,
}

/// An edge in the module graph.
#[derive(Debug, Clone)]
pub struct ModuleEdge {
    pub source: PathBuf,
    pub target: PathBuf,
    pub kind: EdgeKind,
    pub is_type_only: bool,
}

/// The module reference graph.
#[derive(Debug, Default)]
pub struct ModuleGraph {
    pub nodes: HashMap<PathBuf, ModuleNode>,
    pub edges: Vec<ModuleEdge>,
    /// Adjacency list: source → list of edge indices.
    adjacency: HashMap<PathBuf, Vec<usize>>,
}

impl ModuleGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, path: PathBuf, is_test_file: bool) {
        self.nodes.entry(path.clone()).or_insert_with(|| ModuleNode {
            path,
            hazards: Vec::new(),
            is_test_file,
        });
    }

    pub fn set_hazards(&mut self, path: &Path, hazards: Vec<Hazard>) {
        if let Some(node) = self.nodes.get_mut(path) {
            node.hazards = hazards;
        }
    }

    pub fn add_edge(&mut self, source: PathBuf, target: PathBuf, kind: EdgeKind, is_type_only: bool) {
        let idx = self.edges.len();
        self.edges.push(ModuleEdge {
            source: source.clone(),
            target,
            kind,
            is_type_only,
        });
        self.adjacency.entry(source).or_default().push(idx);
    }

    /// Get outgoing edges from a node.
    pub fn outgoing_edges(&self, source: &Path) -> Vec<&ModuleEdge> {
        self.adjacency
            .get(source)
            .map(|indices| indices.iter().map(|&i| &self.edges[i]).collect())
            .unwrap_or_default()
    }

    /// Compute the effective subgraph for a test file given its mock map.
    /// Returns the set of module paths reachable from the test file after mock overlay.
    pub fn effective_subgraph(
        &self,
        test_file: &Path,
        mocks: &[MockDeclaration],
    ) -> HashSet<PathBuf> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(test_file.to_path_buf());

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }

            for edge in self.outgoing_edges(&current) {
                // Skip type-only imports
                if edge.is_type_only {
                    continue;
                }

                let target = &edge.target;

                // Check if this target is fully mocked
                let is_fully_mocked = mocks.iter().any(|m| {
                    m.kind == MockKind::Full
                        && m.resolved_path.as_deref() == Some(target.as_path())
                });

                if is_fully_mocked {
                    // Edge cut — don't traverse
                    continue;
                }

                // Partially mocked or unmocked — add to subgraph and continue
                queue.push_back(target.clone());
            }
        }

        visited
    }

    /// Find shortest path that doesn't traverse fully-mocked modules.
    /// Same BFS as `shortest_path` but skips fully-mocked intermediates
    /// (mirrors `effective_subgraph` logic).
    pub fn shortest_unmocked_path(
        &self,
        source: &Path,
        target: &Path,
        mocks: &[MockDeclaration],
    ) -> Option<Vec<PathBuf>> {
        if source == target {
            return Some(vec![source.to_path_buf()]);
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut parent: HashMap<PathBuf, PathBuf> = HashMap::new();

        queue.push_back(source.to_path_buf());
        visited.insert(source.to_path_buf());

        while let Some(current) = queue.pop_front() {
            for edge in self.outgoing_edges(&current) {
                if edge.is_type_only {
                    continue;
                }
                let next = &edge.target;

                // Skip fully-mocked intermediates (but allow target itself)
                if next != target {
                    let is_fully_mocked = mocks.iter().any(|m| {
                        m.kind == MockKind::Full
                            && m.resolved_path.as_deref() == Some(next.as_path())
                    });
                    if is_fully_mocked {
                        continue;
                    }
                }

                if !visited.contains(next) {
                    visited.insert(next.clone());
                    parent.insert(next.clone(), current.clone());
                    if next == target {
                        let mut path = vec![target.to_path_buf()];
                        let mut cur = target;
                        while let Some(p) = parent.get(cur) {
                            path.push(p.clone());
                            cur = p;
                        }
                        path.reverse();
                        return Some(path);
                    }
                    queue.push_back(next.clone());
                }
            }
        }

        None
    }

    /// Find shortest path from source to target using BFS.
    pub fn shortest_path(&self, source: &Path, target: &Path) -> Option<Vec<PathBuf>> {
        if source == target {
            return Some(vec![source.to_path_buf()]);
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut parent: HashMap<PathBuf, PathBuf> = HashMap::new();

        queue.push_back(source.to_path_buf());
        visited.insert(source.to_path_buf());

        while let Some(current) = queue.pop_front() {
            for edge in self.outgoing_edges(&current) {
                if edge.is_type_only {
                    continue;
                }
                let next = &edge.target;
                if !visited.contains(next) {
                    visited.insert(next.clone());
                    parent.insert(next.clone(), current.clone());
                    if next == target {
                        // Reconstruct path
                        let mut path = vec![target.to_path_buf()];
                        let mut cur = target;
                        while let Some(p) = parent.get(cur) {
                            path.push(p.clone());
                            cur = p;
                        }
                        path.reverse();
                        return Some(path);
                    }
                    queue.push_back(next.clone());
                }
            }
        }

        None
    }
}
