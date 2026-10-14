//! Cycle detection over `depends_on`, through the crate's shared detector
//! ([`crate::cycle::find_cycle`]), which `dag` uses too.

use super::Node;
use crate::cycle;

/// One `depends_on` cycle in `nodes`, as ids from its smallest id back to it
/// (`["a", "c", "b", "a"]`), or `None` when the graph is acyclic.
///
/// `nodes` must be sorted by id with unique ids; dependencies naming no node are ignored (they
/// are reported as dangling elsewhere). Dependencies are followed in id order, as `dag`
/// follows blockers.
pub(super) fn find_cycle(nodes: &[Node]) -> Option<Vec<String>> {
    let cycle = cycle::find_cycle(&edges(nodes))?;
    let ids: Vec<&str> = cycle
        .iter()
        .filter_map(|&at| nodes.get(at).map(|node| node.id.as_str()))
        .collect();
    let first = ids.first()?;
    let path = ids.iter().chain(std::iter::once(first));
    Some(path.map(|&id| id.to_owned()).collect())
}

/// Each node's dependencies as indices into `nodes`, ascending, each once.
fn edges(nodes: &[Node]) -> Vec<Vec<usize>> {
    nodes
        .iter()
        .map(|node| {
            let mut needs: Vec<usize> = node
                .depends_on
                .iter()
                .filter_map(|id| {
                    nodes
                        .binary_search_by(|other| other.id.as_str().cmp(id.as_str()))
                        .ok()
                })
                .collect();
            needs.sort_unstable();
            needs.dedup();
            needs
        })
        .collect()
}
