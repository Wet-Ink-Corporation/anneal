//! The design tree: the questions a grilling asks, as a JSON file the model keeps.
//!
//! [`TreeFile`] is the file as written (`plugin/playbooks/schemas/design-tree.schema.json`);
//! [`DesignTree::parse`] turns its bytes into a tree that is known to be valid, so the
//! frontier and the confidence summary never meet a cycle or a dangling dependency.

mod check;
mod cycle;

use serde::{Deserialize, Serialize};

/// The design tree format version this build reads.
pub const FORMAT_VERSION: u32 = 1;
/// The largest design tree file read, in bytes (1 MiB).
pub const MAX_TREE_BYTES: usize = 1 << 20;
/// The most nodes a design tree may hold.
pub const MAX_NODES: usize = 512;
/// The most turns a design tree may record.
pub const MAX_TURNS: usize = 256;
/// The longest node id, in bytes.
pub const MAX_ID_LEN: usize = 64;
/// The most dependencies one node may list. With [`MAX_NODES`] it bounds the findings a tree
/// can produce: past it the node gets one finding, not one per dependency.
pub const MAX_DEPENDENCIES: usize = 64;
/// The `is_decision` probability from which a question goes to the human (Clef D7's
/// threshold, #32).
pub const ASK_AT: f64 = 0.5;

/// The design tree file, as written. Every field is required and no other field is allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeFile {
    /// The format version.
    pub version: u32,
    /// What is being grilled, in one line.
    pub goal: String,
    /// The confidence after each round, oldest first.
    pub turns: Vec<Turn>,
    /// The questions.
    pub nodes: Vec<Node>,
}

impl TreeFile {
    /// Reads the bytes of a design tree file as written, its nodes in file order. Only the
    /// size, the format version and the shape are checked; [`DesignTree::validate`] checks
    /// the rest.
    ///
    /// # Errors
    /// [`TreeProblem::TooLarge`] past [`MAX_TREE_BYTES`], [`TreeProblem::Version`] for another
    /// format version, [`TreeProblem::Json`] for bytes that are not a design tree.
    pub fn read(bytes: &[u8]) -> Result<Self, Vec<TreeProblem>> {
        if bytes.len() > MAX_TREE_BYTES {
            return Err(vec![TreeProblem::TooLarge {
                limit: MAX_TREE_BYTES,
            }]);
        }
        // The version is read leniently first, so a tree from another version is reported as
        // such even when its other fields changed. A probe that fails (not JSON, no version)
        // is not lost: the strict parse below fails too and reports why.
        if let Ok(VersionProbe { version }) = serde_json::from_slice(bytes)
            && version != FORMAT_VERSION
        {
            return Err(vec![TreeProblem::Version { found: version }]);
        }
        serde_json::from_slice(bytes).map_err(|error| {
            vec![TreeProblem::Json {
                reason: error.to_string(),
            }]
        })
    }

    /// Sets the `is_decision` of the first node with `id` to `probability` (Clef D7's
    /// estimate) and returns true, or returns false when no node has `id`.
    pub fn set_is_decision(&mut self, id: &str, probability: f64) -> bool {
        let Some(node) = self.nodes.iter_mut().find(|node| node.id == id) else {
            return false;
        };
        node.is_decision = probability;
        true
    }
}

/// Just the format version, read without rejecting unknown fields.
#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

/// One round of questions and the model's confidence in the shared understanding after it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// From 0 to 1.
    pub confidence: f64,
}

/// One question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Unique within the tree.
    pub id: String,
    /// The question, as it would be asked.
    pub question: String,
    /// The ids of the questions whose answers this one needs first.
    pub depends_on: Vec<String>,
    /// Where the question stands.
    pub status: Status,
    /// The recommended answer while open (a "yes" accepts it), the agreed answer once
    /// answered, the assumption carried while deferred.
    pub answer: String,
    /// The probability, from 0 to 1, that only the human can settle this (a decision) rather
    /// than the model looking it up (a fact). From [`ASK_AT`] the question is asked.
    pub is_decision: f64,
    /// The model's confidence in `answer`, from 0 to 1.
    pub confidence: f64,
}

impl Node {
    /// Whether the human is asked: `is_decision` is [`ASK_AT`] or more. Otherwise the model
    /// looks the answer up.
    #[must_use]
    pub fn is_asked(&self) -> bool {
        self.is_decision >= ASK_AT
    }
}

/// Where a question stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not settled yet.
    Open,
    /// Settled: `answer` is agreed.
    Answered,
    /// Set aside, with `answer` as the assumption.
    Deferred,
}

impl Status {
    /// Every status, in the order the schema lists them.
    pub const ALL: [Self; 3] = [Self::Open, Self::Answered, Self::Deferred];
}

/// Why a design tree is not valid.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TreeProblem {
    /// The bytes are not a design tree: bad JSON, a missing or unknown field, a bad status.
    #[error("not a design tree: {reason}")]
    Json {
        /// serde's reason, with its line and column.
        reason: String,
    },
    /// The file is larger than [`MAX_TREE_BYTES`]; it was not parsed.
    #[error("the design tree is over {limit} bytes")]
    TooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// `version` is not [`FORMAT_VERSION`].
    #[error("format version {found} is not supported (expected {FORMAT_VERSION})")]
    Version {
        /// The version the file names.
        found: u32,
    },
    /// `goal` is empty or blank.
    #[error("the goal is empty")]
    EmptyGoal,
    /// More than [`MAX_TURNS`] turns.
    #[error("{count} turns; at most {MAX_TURNS}")]
    TooManyTurns {
        /// How many the file has.
        count: usize,
    },
    /// More than [`MAX_NODES`] nodes.
    #[error("{count} nodes; at most {MAX_NODES}")]
    TooManyNodes {
        /// How many the file has.
        count: usize,
    },
    /// A turn's confidence is outside 0..=1.
    #[error("turn {turn}: confidence {value} is outside 0 to 1")]
    TurnConfidence {
        /// The turn, counted from 1.
        turn: usize,
        /// The confidence given.
        value: f64,
    },
    /// An id is empty, too long, or has a character outside `A-Za-z0-9._-` (or a leading
    /// `.`, `_` or `-`).
    #[error("id {id:?} must match ^[A-Za-z0-9][A-Za-z0-9._-]{{0,63}}$")]
    BadId {
        /// The id.
        id: String,
    },
    /// Two or more nodes share an id.
    #[error("id {id:?} is used by more than one node")]
    DuplicateId {
        /// The id.
        id: String,
    },
    /// A node's question is empty or blank.
    #[error("{id}: the question is empty")]
    EmptyQuestion {
        /// The node.
        id: String,
    },
    /// A node's answer is empty or blank: an open question carries no recommended answer.
    #[error("{id}: the answer is empty; an open question needs its recommended answer")]
    EmptyAnswer {
        /// The node.
        id: String,
    },
    /// A node's confidence is outside 0..=1.
    #[error("{id}: confidence {value} is outside 0 to 1")]
    NodeConfidence {
        /// The node.
        id: String,
        /// The confidence given.
        value: f64,
    },
    /// A node's `is_decision` probability is outside 0..=1.
    #[error("{id}: is_decision {value} is outside 0 to 1")]
    IsDecision {
        /// The node.
        id: String,
        /// The probability given.
        value: f64,
    },
    /// A node lists more than [`MAX_DEPENDENCIES`] dependencies; they are not checked one by
    /// one.
    #[error("{id}: {count} dependencies; at most {MAX_DEPENDENCIES}")]
    TooManyDependencies {
        /// The node.
        id: String,
        /// How many it lists.
        count: usize,
    },
    /// A dependency names no node.
    #[error("{id}: depends on {dependency:?}, which is not a node")]
    DanglingDependency {
        /// The node.
        id: String,
        /// The id it depends on.
        dependency: String,
    },
    /// A dependency is listed more than once.
    #[error("{id}: lists {dependency:?} more than once in depends_on")]
    DuplicateDependency {
        /// The node.
        id: String,
        /// The repeated id.
        dependency: String,
    },
    /// `depends_on` loops back on itself.
    #[error("depends_on has a cycle: {}", path.join(" -> "))]
    Cycle {
        /// The ids on the cycle, from its smallest id back to that id.
        path: Vec<String>,
    },
}

impl TreeProblem {
    /// A stable finding key: `grill/<kind>` plus the id or turn it concerns.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Json { .. } => "grill/json".to_owned(),
            Self::TooLarge { .. } => "grill/too-large".to_owned(),
            Self::Version { .. } => "grill/version".to_owned(),
            Self::EmptyGoal => "grill/empty-goal".to_owned(),
            Self::TooManyTurns { .. } => "grill/too-many-turns".to_owned(),
            Self::TooManyNodes { .. } => "grill/too-many-nodes".to_owned(),
            Self::TurnConfidence { turn, .. } => format!("grill/turn-confidence:{turn}"),
            Self::BadId { id } => format!("grill/bad-id:{id}"),
            Self::DuplicateId { id } => format!("grill/duplicate-id:{id}"),
            Self::EmptyQuestion { id } => format!("grill/empty-question:{id}"),
            Self::EmptyAnswer { id } => format!("grill/empty-answer:{id}"),
            Self::NodeConfidence { id, .. } => format!("grill/node-confidence:{id}"),
            Self::IsDecision { id, .. } => format!("grill/is-decision:{id}"),
            Self::TooManyDependencies { id, .. } => format!("grill/too-many-dependencies:{id}"),
            Self::DanglingDependency { id, dependency } => {
                format!("grill/dangling:{id}:{dependency}")
            }
            Self::DuplicateDependency { id, dependency } => {
                format!("grill/duplicate-dependency:{id}:{dependency}")
            }
            // Ids may hold `-`, so the path is joined with `->`, as `dag` does.
            Self::Cycle { path } => format!("grill/cycle:{}", path.join("->")),
        }
    }
}

/// The confidence a turn must reach for the restate gate to fire.
pub const RESTATE_CONFIDENCE: f64 = 0.8;
/// How many turns in a row without a gain fire the restate gate.
pub const RESTATE_TURNS_WITHOUT_GAIN: usize = 3;

/// The per-turn confidence, summed up for the restate gate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ConfidenceSummary {
    /// How many turns are recorded.
    pub turns: usize,
    /// The last turn's confidence.
    pub latest: Option<f64>,
    /// The highest confidence so far.
    pub best: Option<f64>,
    /// The latest turns in a row that did not beat the best confidence before them.
    pub turns_without_gain: usize,
    /// The restate gate: `latest` reached [`RESTATE_CONFIDENCE`], or
    /// [`RESTATE_TURNS_WITHOUT_GAIN`] turns in a row gained nothing.
    pub restate: bool,
}

impl ConfidenceSummary {
    /// The summary of `turns`, oldest first.
    fn of(turns: &[Turn]) -> Self {
        let mut best: Option<f64> = None;
        let mut turns_without_gain = 0_usize;
        for turn in turns {
            if best.is_some_and(|best| turn.confidence <= best) {
                turns_without_gain = turns_without_gain.saturating_add(1);
            } else {
                turns_without_gain = 0;
                best = Some(turn.confidence);
            }
        }
        let latest = turns.last().map(|turn| turn.confidence);
        Self {
            turns: turns.len(),
            latest,
            best,
            turns_without_gain,
            restate: latest.is_some_and(|latest| latest >= RESTATE_CONFIDENCE)
                || turns_without_gain >= RESTATE_TURNS_WITHOUT_GAIN,
        }
    }
}

/// A design tree known to be valid. Its nodes are sorted by id.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignTree {
    file: TreeFile,
}

impl DesignTree {
    /// Parses and validates the bytes of a design tree file: [`TreeFile::read`], then
    /// [`DesignTree::validate`].
    ///
    /// # Errors
    /// Every [`TreeProblem`] found; validation goes on past the first one where it can.
    pub fn parse(bytes: &[u8]) -> Result<Self, Vec<TreeProblem>> {
        Self::validate(TreeFile::read(bytes)?)
    }

    /// Validates a design tree file read by [`TreeFile::read`], sorting its nodes by id.
    ///
    /// # Errors
    /// Every [`TreeProblem`] found; validation goes on past the first one where it can.
    pub fn validate(mut file: TreeFile) -> Result<Self, Vec<TreeProblem>> {
        file.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut problems = check::problems(&file);
        // Only a well-formed graph is walked: ids unique, every dependency a node.
        if problems.is_empty()
            && let Some(path) = cycle::find_cycle(&file.nodes)
        {
            problems.push(TreeProblem::Cycle { path });
        }
        if problems.is_empty() {
            Ok(Self { file })
        } else {
            Err(problems)
        }
    }

    /// What is being grilled.
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.file.goal
    }

    /// The per-turn confidence and the restate gate.
    #[must_use]
    pub fn confidence(&self) -> ConfidenceSummary {
        ConfidenceSummary::of(&self.file.turns)
    }

    /// The nodes, sorted by id.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.file.nodes
    }

    /// The node with `id`, if any.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        let nodes = self.nodes();
        nodes
            .binary_search_by(|node| node.id.as_str().cmp(id))
            .ok()
            .and_then(|at| nodes.get(at))
    }

    /// The nodes with `status`, sorted by id.
    pub fn with_status(&self, status: Status) -> impl Iterator<Item = &Node> {
        self.nodes()
            .iter()
            .filter(move |node| node.status == status)
    }

    /// The open nodes whose dependencies are all answered, sorted by id: the questions to
    /// settle next.
    pub fn frontier(&self) -> impl Iterator<Item = &Node> {
        self.nodes().iter().filter(|node| {
            node.status == Status::Open
                && node.depends_on.iter().all(|dependency| {
                    self.node(dependency)
                        .is_some_and(|it| it.status == Status::Answered)
                })
        })
    }
}

#[cfg(test)]
mod tests;
