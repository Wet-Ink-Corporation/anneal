//! Cycle detection over `depends_on`, iterative and linear in nodes plus edges.

use super::Node;

/// One `depends_on` cycle in `nodes`, as ids from its smallest id back to it
/// (`["a", "c", "b", "a"]`), or `None` when the graph is acyclic.
///
/// `nodes` must be sorted by id with unique ids; dependencies naming no node are ignored (they
/// are reported as dangling elsewhere).
pub(super) fn find_cycle(nodes: &[Node]) -> Option<Vec<String>> {
    let dependencies = edges(nodes);
    let pending = unresolved(&dependencies);
    let start = pending.iter().position(|&count| count > 0)?;
    let cycle = walk(start, &dependencies, &pending)?;
    let ids: Vec<&str> = cycle
        .iter()
        .filter_map(|&at| nodes.get(at).map(|node| node.id.as_str()))
        .collect();
    let smallest = ids
        .iter()
        .enumerate()
        .min_by_key(|&(_, id)| *id)
        .map_or(0, |(at, _)| at);
    let (tail, head) = ids.split_at_checked(smallest)?;
    let first = head.first()?;
    let path = head.iter().chain(tail).chain(std::iter::once(first));
    Some(path.map(|&id| id.to_owned()).collect())
}

/// Each node's dependencies as indices into `nodes`.
fn edges(nodes: &[Node]) -> Vec<Vec<usize>> {
    nodes
        .iter()
        .map(|node| {
            node.depends_on
                .iter()
                .filter_map(|id| {
                    nodes
                        .binary_search_by(|other| other.id.as_str().cmp(id.as_str()))
                        .ok()
                })
                .collect()
        })
        .collect()
}

/// Kahn's algorithm: for each node, how many of its dependency edges were never resolved. A
/// node is left with a count above zero exactly when it is on a cycle or waits on one.
fn unresolved(dependencies: &[Vec<usize>]) -> Vec<usize> {
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); dependencies.len()];
    for (node, needs) in dependencies.iter().enumerate() {
        for &dependency in needs {
            if let Some(list) = dependents.get_mut(dependency) {
                list.push(node);
            }
        }
    }
    let mut pending: Vec<usize> = dependencies.iter().map(Vec::len).collect();
    let mut ready: Vec<usize> = (0..pending.len())
        .filter(|&node| pending.get(node) == Some(&0))
        .collect();
    while let Some(node) = ready.pop() {
        for &dependent in dependents.get(node).map_or(&[][..], Vec::as_slice) {
            if let Some(count) = pending.get_mut(dependent) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push(dependent);
                }
            }
        }
    }
    pending
}

/// Follows unresolved dependencies from `start` until a node repeats, and returns the nodes of
/// that loop in walk order. Every unresolved node has an unresolved dependency, so the walk
/// cannot stop early; it is bounded by the node count all the same.
fn walk(start: usize, dependencies: &[Vec<usize>], pending: &[usize]) -> Option<Vec<usize>> {
    let mut seen_at: Vec<Option<usize>> = vec![None; dependencies.len()];
    let mut path = Vec::new();
    let mut node = start;
    for step in 0..=dependencies.len() {
        if let Some(Some(first)) = seen_at.get(node) {
            return path.get(*first..).map(<[usize]>::to_vec);
        }
        *seen_at.get_mut(node)? = Some(step);
        path.push(node);
        node = dependencies
            .get(node)?
            .iter()
            .copied()
            .find(|&dependency| pending.get(dependency).is_some_and(|&count| count > 0))?;
    }
    None
}
