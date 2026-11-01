//! The derived graph (original `core/graph.ts`, `query.ts` slices, `validate.ts`): first file wins an
//! id, edges come from `parent` + the list link fields, reverse edges are indexed for lookup.

use std::collections::{HashMap, HashSet};

use super::model::{ID, LINK_KINDS, PARENT, TITLE, TYPE, list, scalar};
use super::store::SpecFile;

pub(super) struct Node<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub title: Option<&'a str>,
    pub file: &'a SpecFile,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Edge<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub kind: &'static str,
}

type Adjacency<'a> = [HashMap<&'a str, Vec<&'a str>>; LINK_KINDS.len()];

pub(super) struct Graph<'a> {
    pub nodes: Vec<Node<'a>>,
    index: HashMap<&'a str, usize>,
    forward: Adjacency<'a>,
    reverse: Adjacency<'a>,
    /// Ids carried by more than one file, with every path, in first-seen order.
    pub duplicates: Vec<(&'a str, Vec<&'a str>)>,
}

pub(super) fn link_targets<'a>(file: &'a SpecFile, kind: &str) -> Vec<&'a str> {
    if kind == PARENT { scalar(&file.fm, PARENT).into_iter().collect() } else { list(&file.fm, kind) }
}

pub(super) fn kind_index(kind: &str) -> Option<usize> {
    LINK_KINDS.iter().position(|k| *k == kind)
}

impl<'a> Graph<'a> {
    pub fn build(files: &'a [SpecFile]) -> Self {
        let mut graph = Graph { nodes: Vec::new(), index: HashMap::new(), forward: Default::default(), reverse: Default::default(), duplicates: Vec::new() };
        let mut paths: Vec<(&'a str, Vec<&'a str>)> = Vec::new();
        let mut path_slot: HashMap<&'a str, usize> = HashMap::new();
        for file in files {
            let (Some(id), Some(kind)) = (scalar(&file.fm, ID), scalar(&file.fm, TYPE)) else { continue };
            let slot = *path_slot.entry(id).or_insert_with(|| {
                paths.push((id, Vec::new()));
                paths.len() - 1
            });
            paths[slot].1.push(&file.rel);
            if !graph.index.contains_key(id) {
                graph.index.insert(id, graph.nodes.len());
                graph.nodes.push(Node { id, kind, title: scalar(&file.fm, TITLE), file });
            }
        }
        for node in &graph.nodes {
            for (k, kind) in LINK_KINDS.iter().enumerate() {
                for to in link_targets(node.file, kind) {
                    graph.forward[k].entry(node.id).or_default().push(to);
                    graph.reverse[k].entry(to).or_default().push(node.id);
                }
            }
        }
        graph.duplicates = paths.into_iter().filter(|(_, p)| p.len() > 1).collect();
        graph
    }

    pub fn node(&self, id: &str) -> Option<&Node<'a>> {
        self.index.get(id).map(|&i| &self.nodes[i])
    }

    pub fn reverse(&self, kind: usize, id: &str) -> &[&'a str] {
        self.reverse[kind].get(id).map_or(&[], Vec::as_slice)
    }

    fn forward(&self, kind: usize, id: &str) -> &[&'a str] {
        self.forward[kind].get(id).map_or(&[], Vec::as_slice)
    }
}

pub(super) struct Slice<'g, 'a> {
    pub nodes: Vec<&'g Node<'a>>,
    pub edges: Vec<Edge<'a>>,
    pub missing: Vec<&'a str>,
}

/// A breadth-first slice `depth` hops out from `root` (which must be a node).
pub(super) fn slice<'g, 'a>(graph: &'g Graph<'a>, root: &'a str, direction: &str, edge: usize, depth: f64) -> Slice<'g, 'a> {
    let parent = 0;
    let next = |id: &'a str| -> Vec<(&'a str, Edge<'a>)> {
        match direction {
            "subtree" => graph.reverse(parent, id).iter().map(|&child| (child, Edge { from: child, to: id, kind: PARENT })).collect(),
            "ancestors" => graph.forward(parent, id).iter().map(|&up| (up, Edge { from: id, to: up, kind: PARENT })).collect(),
            _ => {
                let kind = LINK_KINDS[edge];
                let out = graph.forward(edge, id).iter().map(|&to| (to, Edge { from: id, to, kind }));
                let inc = graph.reverse(edge, id).iter().map(|&from| (from, Edge { from, to: id, kind }));
                out.chain(inc).collect()
            }
        }
    };
    let mut included = vec![root];
    let mut seen_nodes = HashSet::from([root]);
    let mut edges = Vec::new();
    let mut seen_edges = HashSet::new();
    let mut missing = Vec::new();
    let mut frontier = vec![root];
    let mut hop = 0.0;
    while hop < depth && !frontier.is_empty() {
        let mut next_frontier = Vec::new();
        for id in frontier {
            for (to, e) in next(id) {
                if seen_edges.insert(e) {
                    edges.push(e);
                }
                if graph.node(to).is_none() && !missing.contains(&to) {
                    missing.push(to);
                }
                if seen_nodes.insert(to) {
                    included.push(to);
                    next_frontier.push(to);
                }
            }
        }
        frontier = next_frontier;
        hop += 1.0;
    }
    let nodes = included.into_iter().filter_map(|id| graph.node(id)).collect();
    Slice { nodes, edges, missing }
}

pub(super) struct Dangling<'a> {
    pub from: &'a str,
    pub from_path: &'a str,
    pub kind: &'static str,
    pub target: &'a str,
}

pub(super) fn dangling_links<'a>(graph: &Graph<'a>) -> Vec<Dangling<'a>> {
    let mut out = Vec::new();
    for node in &graph.nodes {
        for kind in LINK_KINDS {
            for target in link_targets(node.file, kind) {
                if graph.node(target).is_none() {
                    out.push(Dangling { from: node.id, from_path: &node.file.rel, kind, target });
                }
            }
        }
    }
    out
}

/// Each distinct ring of the parent chain once, starting where the walk entered it.
pub(super) fn parent_cycles<'a>(graph: &Graph<'a>) -> Vec<Vec<&'a str>> {
    let mut cycles: Vec<Vec<&'a str>> = Vec::new();
    let mut seen = HashSet::new();
    for start in &graph.nodes {
        if seen.contains(start.id) {
            continue;
        }
        let mut path: Vec<&'a str> = Vec::new();
        let mut current = Some(start.id);
        while let Some(id) = current {
            let Some(node) = graph.node(id) else { break };
            if let Some(at) = path.iter().position(|p| *p == id) {
                let ring = path[at..].to_vec();
                let mut key = ring.clone();
                key.sort_unstable();
                if !cycles.iter().any(|c| {
                    let mut k = c.clone();
                    k.sort_unstable();
                    k == key
                }) {
                    cycles.push(ring);
                }
                break;
            }
            if seen.contains(id) {
                break;
            }
            path.push(id);
            current = scalar(&node.file.fm, PARENT);
        }
        seen.extend(path);
    }
    cycles
}
