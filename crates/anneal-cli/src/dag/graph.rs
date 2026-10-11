//! The ticket graph: tickets as nodes, "blocked by" as edges.
//!
//! The graph is an arena: tickets sorted by [`TicketId`] order, and each ticket's blockers as
//! indices into that list. Sorting once makes every listing (frontier, dangling refs, the
//! reported cycle) come out in the same order on every run.
//!
//! ```
//! use anneal_cli::dag::graph::Graph;
//! use anneal_cli::dag::ticket::{Status, Ticket, TicketId};
//!
//! let ticket = |id: &str, blocked_by: &[&str], status| Ticket {
//!     id: TicketId::new(id),
//!     title: id.to_owned(),
//!     body: String::new(),
//!     blocked_by: blocked_by.iter().copied().map(TicketId::new).collect(),
//!     files: Vec::new(),
//!     status,
//! };
//! let graph = Graph::new(vec![
//!     ticket("1", &[], Status::Done),
//!     ticket("2", &["1"], Status::Open),
//!     ticket("3", &["2"], Status::Open),
//! ])?;
//!
//! assert!(graph.check().code().is_success());
//! let ready: Vec<&str> = graph.frontier().map(|t| t.id.as_str()).collect();
//! assert_eq!(ready, ["2"]);
//! # Ok::<(), anneal_cli::dag::graph::GraphError>(())
//! ```

use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::exit_codes::{self, Code};

/// A ticket's blocker that names no ticket in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dangling {
    /// The ticket that names it.
    pub ticket: TicketId,
    /// The id it names.
    pub missing: TicketId,
}

/// Why the tickets do not form a graph.
#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    /// Two tickets share an id, so a reference to it is ambiguous.
    #[error("two tickets have the id {id}")]
    DuplicateId {
        /// The shared id.
        id: TicketId,
    },
}

/// The tickets and their resolved blocking edges.
#[derive(Debug)]
pub struct Graph {
    /// Sorted by id; indices below point into it.
    tickets: Vec<Ticket>,
    /// `blockers[i]`: the indices of the tickets `tickets[i]` is blocked by, ascending, once.
    blockers: Vec<Vec<usize>>,
    /// Every reference to a missing ticket, by ticket and then missing id.
    dangling: Vec<Dangling>,
}

/// What `dag check` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check<'g> {
    /// Every dangling reference.
    pub dangling: &'g [Dangling],
    /// One cycle, if any: each id is blocked by the next and the last by the first, starting
    /// at the smallest id.
    pub cycle: Option<Vec<&'g TicketId>>,
}

impl Check<'_> {
    /// The exit code: [`exit_codes::DAG_DANGLING`] when any reference dangles (a cycle through
    /// a missing ticket cannot be judged, so dangling comes first), else
    /// [`exit_codes::DAG_CYCLE`] for a cycle, else [`exit_codes::OK`].
    #[must_use]
    pub const fn code(&self) -> Code {
        if !self.dangling.is_empty() {
            exit_codes::DAG_DANGLING
        } else if self.cycle.is_some() {
            exit_codes::DAG_CYCLE
        } else {
            exit_codes::OK
        }
    }
}

/// Where the depth-first search stands with a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    OnPath,
    Finished,
}

impl Graph {
    /// The graph of `tickets`, which it keeps. Blockers that name a ticket outside `tickets`
    /// are kept as [`Dangling`] references rather than edges.
    ///
    /// # Errors
    /// [`GraphError::DuplicateId`] when two tickets share an id.
    pub fn new(mut tickets: Vec<Ticket>) -> Result<Self, GraphError> {
        tickets.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some([first, _]) = tickets
            .windows(2)
            .find(|w| matches!(w, [a, b] if a.id == b.id))
        {
            // The error outlives the borrowed window, so it owns a copy of the id.
            return Err(GraphError::DuplicateId {
                id: first.id.clone(),
            });
        }
        let mut blockers = Vec::with_capacity(tickets.len());
        let mut dangling = Vec::new();
        for ticket in &tickets {
            let mut resolved = Vec::new();
            let mut missing = Vec::new();
            for blocker in &ticket.blocked_by {
                match tickets.binary_search_by(|t| t.id.cmp(blocker)) {
                    Ok(index) => resolved.push(index),
                    Err(_) => missing.push(blocker),
                }
            }
            resolved.sort_unstable();
            resolved.dedup();
            missing.sort();
            missing.dedup();
            blockers.push(resolved);
            // Each report owns its ids: the tickets keep theirs for the listings.
            dangling.extend(missing.into_iter().map(|id| Dangling {
                ticket: ticket.id.clone(),
                missing: id.clone(),
            }));
        }
        Ok(Self {
            tickets,
            blockers,
            dangling,
        })
    }

    /// Every ticket, in id order.
    #[must_use]
    pub fn tickets(&self) -> &[Ticket] {
        &self.tickets
    }

    /// The number of blocking edges between tickets in the graph (dangling refs excluded).
    #[must_use]
    pub fn edges(&self) -> usize {
        self.blockers.iter().map(Vec::len).sum()
    }

    /// Checks for dangling references and cycles.
    #[must_use]
    pub fn check(&self) -> Check<'_> {
        let cycle = self.find_cycle().map(|indices| {
            indices
                .into_iter()
                .filter_map(|index| self.tickets.get(index).map(|t| &t.id))
                .collect()
        });
        Check {
            dangling: &self.dangling,
            cycle,
        }
    }

    /// The ready tickets, in id order: open, with every blocker done. Meaningful only when
    /// [`Graph::check`] is clean.
    pub fn frontier(&self) -> impl Iterator<Item = &Ticket> + '_ {
        self.tickets
            .iter()
            .zip(&self.blockers)
            .filter(|(ticket, blockers)| {
                ticket.status == Status::Open
                    && blockers.iter().all(|&index| {
                        self.tickets
                            .get(index)
                            .is_some_and(|blocker| blocker.status == Status::Done)
                    })
            })
            .map(|(ticket, _)| ticket)
    }

    /// The first cycle a depth-first search meets, starting from tickets and blockers in id
    /// order, as indices rotated to start at the smallest. Iterative, with an explicit stack of
    /// (ticket, next blocker to follow), so a long chain cannot overflow the call stack.
    fn find_cycle(&self) -> Option<Vec<usize>> {
        let mut visit = vec![Visit::New; self.tickets.len()];
        let mut path: Vec<(usize, usize)> = Vec::new();
        for start in 0..self.tickets.len() {
            if !mark(&mut visit, start, Visit::New, Visit::OnPath) {
                continue;
            }
            path.push((start, 0));
            while let Some((node, next)) = path.last_mut() {
                let node = *node;
                let Some(&blocker) = self.blockers.get(node).and_then(|b| b.get(*next)) else {
                    mark(&mut visit, node, Visit::OnPath, Visit::Finished);
                    path.pop();
                    continue;
                };
                *next = next.saturating_add(1);
                match visit.get(blocker) {
                    Some(Visit::New) => {
                        mark(&mut visit, blocker, Visit::New, Visit::OnPath);
                        path.push((blocker, 0));
                    }
                    Some(Visit::OnPath) => return Some(cycle_from(&path, blocker)),
                    Some(Visit::Finished) | None => {}
                }
            }
        }
        None
    }
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
/// start at its smallest index (the smallest id, as tickets are sorted).
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
