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
    /// `scope[i]`: whether `tickets[i]` is in scope or an external blocker.
    scope: Vec<Scope>,
    /// `blockers[i]`: the indices of the tickets `tickets[i]` is blocked by, ascending, once.
    /// Always empty for an external ticket.
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

/// Whether a ticket is one the graph is about, or only a blocker from outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Checked, counted, and listed in the frontier when ready.
    InScope,
    /// Resolves references to it; never counted or ready; a leaf.
    External,
}

/// An open external ticket and the open tickets in scope it holds up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBlocker<'g> {
    /// The external ticket.
    pub ticket: &'g Ticket,
    /// The open in-scope tickets blocked by it, in id order.
    pub blocks: Vec<&'g TicketId>,
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
    pub fn new(tickets: Vec<Ticket>) -> Result<Self, GraphError> {
        Self::with_external(tickets, Vec::new())
    }

    /// The graph of `tickets`, with `external` tickets from outside the scope that resolve
    /// references to them: a done one is a satisfied blocker, an open one blocks. External
    /// tickets are leaves (their own `blocked_by` is ignored) and are never counted, checked
    /// for dangling references or ready.
    ///
    /// # Errors
    /// [`GraphError::DuplicateId`] when two tickets, in scope or external, share an id.
    pub fn with_external(tickets: Vec<Ticket>, external: Vec<Ticket>) -> Result<Self, GraphError> {
        let mut entries: Vec<(Ticket, Scope)> = tickets
            .into_iter()
            .map(|ticket| (ticket, Scope::InScope))
            .chain(external.into_iter().map(|ticket| (ticket, Scope::External)))
            .collect();
        entries.sort_by(|(a, _), (b, _)| a.id.cmp(&b.id));
        if let Some([(first, _), _]) = entries
            .windows(2)
            .find(|w| matches!(w, [(a, _), (b, _)] if a.id == b.id))
        {
            // The error outlives the borrowed window, so it owns a copy of the id.
            return Err(GraphError::DuplicateId {
                id: first.id.clone(),
            });
        }
        let (tickets, scope): (Vec<Ticket>, Vec<Scope>) = entries.into_iter().unzip();
        let mut blockers = Vec::with_capacity(tickets.len());
        let mut dangling = Vec::new();
        for (ticket, &ticket_scope) in tickets.iter().zip(&scope) {
            if ticket_scope == Scope::External {
                blockers.push(Vec::new());
                continue;
            }
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
            scope,
            blockers,
            dangling,
        })
    }

    /// The tickets in scope, in id order (external tickets excluded).
    pub fn tickets(&self) -> impl Iterator<Item = &Ticket> + '_ {
        self.in_scope().map(|(ticket, _)| ticket)
    }

    /// Each open external ticket that holds up an open ticket in scope, in id order.
    pub fn external_blockers(&self) -> impl Iterator<Item = ExternalBlocker<'_>> + '_ {
        self.tickets
            .iter()
            .enumerate()
            .zip(&self.scope)
            .filter(|((_, ticket), scope)| {
                **scope == Scope::External && ticket.status == Status::Open
            })
            .filter_map(|((index, ticket), _)| {
                let blocks: Vec<&TicketId> = self
                    .in_scope()
                    .filter(|(dependant, blockers)| {
                        dependant.status == Status::Open && blockers.contains(&index)
                    })
                    .map(|(dependant, _)| &dependant.id)
                    .collect();
                (!blocks.is_empty()).then_some(ExternalBlocker { ticket, blocks })
            })
    }

    /// The tickets in scope with their blockers' indices.
    fn in_scope(&self) -> impl Iterator<Item = (&Ticket, &Vec<usize>)> + '_ {
        self.tickets
            .iter()
            .zip(&self.blockers)
            .zip(&self.scope)
            .filter(|(_, scope)| **scope == Scope::InScope)
            .map(|(pair, _)| pair)
    }

    /// The number of resolved blocking edges, including those to external tickets (dangling
    /// refs excluded).
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

    /// The ready tickets in scope, in id order: open, with every blocker (in scope or
    /// external) done. Meaningful only when [`Graph::check`] is clean.
    pub fn frontier(&self) -> impl Iterator<Item = &Ticket> + '_ {
        self.in_scope()
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
