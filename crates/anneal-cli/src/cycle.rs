//! Cycle detection over dependency edges, shared by `dag` (blockers) and `grill`
//! (`depends_on`): iterative, and linear in nodes plus edges.

/// One cycle in the graph whose node `i` depends on the nodes `dependencies[i]` lists, as
/// indices into `dependencies`, or `None` when the graph is acyclic. Each index in the cycle
/// depends on the next and the last on the first; the cycle is not closed (a node depending
/// on itself is `[i]`).
///
/// A depth-first search starts from nodes in index order and follows each node's
/// dependencies in the order given. The first edge back onto the current path closes the
/// cycle, which is then rotated to start at its smallest index. Callers sort their nodes by
/// id, so that is the smallest id and the result is deterministic. The search keeps an
/// explicit stack, so a very long chain cannot overflow the call stack. A dependency index out
/// of range is ignored (callers report those as dangling).
pub(crate) fn find_cycle<D: AsRef<[usize]>>(dependencies: &[D]) -> Option<Vec<usize>> {
    let mut visit = vec![Visit::New; dependencies.len()];
    // Each entry is a node on the current path and the position of its next dependency.
    let mut path: Vec<(usize, usize)> = Vec::new();
    for start in 0..dependencies.len() {
        if !mark(&mut visit, start, Visit::New, Visit::OnPath) {
            continue;
        }
        path.push((start, 0));
        while let Some((node, next)) = path.last_mut() {
            let node = *node;
            let Some(&dependency) = dependencies
                .get(node)
                .and_then(|needs| needs.as_ref().get(*next))
            else {
                mark(&mut visit, node, Visit::OnPath, Visit::Finished);
                path.pop();
                continue;
            };
            *next = next.saturating_add(1);
            match visit.get(dependency) {
                Some(Visit::New) => {
                    mark(&mut visit, dependency, Visit::New, Visit::OnPath);
                    path.push((dependency, 0));
                }
                Some(Visit::OnPath) => return Some(cycle_from(&path, dependency)),
                Some(Visit::Finished) | None => {}
            }
        }
    }
    None
}

/// Where the search stands with a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    OnPath,
    Finished,
}

/// Moves `index` from state `from` to `to`; false when it was not in `from`.
fn mark(visit: &mut [Visit], index: usize, from: Visit, to: Visit) -> bool {
    match visit.get_mut(index) {
        Some(state) if *state == from => {
            *state = to;
            true
        }
        _ => false,
    }
}

/// The cycle closed by an edge back to `entry`: the path from `entry` to its end, rotated to
/// start at its smallest index.
fn cycle_from(path: &[(usize, usize)], entry: usize) -> Vec<usize> {
    let mut cycle: Vec<usize> = path
        .iter()
        .map(|&(node, _)| node)
        .skip_while(|&node| node != entry)
        .collect();
    let smallest = cycle
        .iter()
        .enumerate()
        .min_by_key(|&(_, node)| node)
        .map_or(0, |(position, _)| position);
    cycle.rotate_left(smallest);
    cycle
}

#[cfg(test)]
mod tests;
