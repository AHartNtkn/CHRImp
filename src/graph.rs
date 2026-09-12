//! Shared relational occurrence state and ordered-port indexes.

use crate::condition::Condition;
use crate::program::Signature;
use crate::store::{self, Key, Root, Store};
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

const FACT: u64 = 0;
const RELATION: u64 = 1;
const PORT: u64 = 2;
const INCIDENCE: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    InvalidRoot,
    InvalidRelation,
    InvalidPort,
    Arity { expected: usize, actual: usize },
    MissingOccurrence,
}
impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRoot => f.write_str("stale or foreign graph root"),
            Self::InvalidRelation => f.write_str("unknown relation signature"),
            Self::InvalidPort => f.write_str("port is outside relation arity"),
            Self::Arity { expected, actual } => {
                write!(f, "expected {expected} variable ports, found {actual}")
            }
            Self::MissingOccurrence => f.write_str("occurrence is not live in this graph root"),
        }
    }
}
impl std::error::Error for GraphError {}

struct Row {
    relation: usize,
    args: Arc<Vec<u64>>,
    marked: u64,
}

pub struct Fact<'a> {
    pub id: u64,
    pub relation: usize,
    pub args: &'a [u64],
    pub support: Condition,
}

/// Immutable occurrence payloads and a persistent supported index. The executor
/// owns the current root; updates return a new root for atomic publication.
pub struct Graph {
    pub(crate) index: Store<Condition>,
    rows: BTreeMap<u64, Row>,
    arities: Vec<usize>,
    port_bases: Vec<u64>,
    next_occurrence: u64,
    epoch: u64,
}

impl Graph {
    pub fn new(signatures: &[Signature]) -> Self {
        let mut next = 0_u64;
        let port_bases = signatures
            .iter()
            .map(|signature| {
                let base = next;
                next = next
                    .checked_add(signature.arity as u64)
                    .expect("port identity exhausted");
                base
            })
            .collect();
        Self {
            index: Store::default(),
            rows: BTreeMap::new(),
            arities: signatures.iter().map(|s| s.arity).collect(),
            port_bases,
            next_occurrence: 0,
            epoch: 0,
        }
    }
    pub fn empty(&self) -> Root {
        self.index.empty()
    }
    pub fn occurrence_count(&self) -> usize {
        self.rows.len()
    }
    pub fn index_node_count(&self) -> usize {
        self.index.node_count()
    }

    fn valid_root(&self, root: Root) -> Result<(), GraphError> {
        if self.index.contains(root) {
            Ok(())
        } else {
            Err(GraphError::InvalidRoot)
        }
    }
    pub fn fact(&self, root: Root, id: u64) -> Option<Fact<'_>> {
        let support = self.index.get(root, &[FACT, id, 0, 0])?;
        let row = self.rows.get(&id).expect("live occurrence payload");
        Some(Fact {
            id,
            relation: row.relation,
            args: row.args.as_slice(),
            support,
        })
    }

    pub fn post(
        &mut self,
        root: Root,
        relation: usize,
        args: Vec<u64>,
        support: Condition,
    ) -> Result<Update, GraphError> {
        self.valid_root(root)?;
        let &arity = self
            .arities
            .get(relation)
            .ok_or(GraphError::InvalidRelation)?;
        if arity != args.len() {
            return Err(GraphError::Arity {
                expected: arity,
                actual: args.len(),
            });
        }
        let id = self.next_occurrence;
        self.next_occurrence = id.checked_add(1).expect("occurrence identity exhausted");
        let args = Arc::new(args);
        self.rows.insert(
            id,
            Row {
                relation,
                args: args.clone(),
                marked: 0,
            },
        );
        Ok(self.update(root, id, relation, args, support))
    }

    /// Low-level staged liveness replacement. CHR commitment computes and
    /// validates the surviving support before calling this method.
    pub fn set_liveness(
        &mut self,
        root: Root,
        id: u64,
        support: Condition,
    ) -> Result<Update, GraphError> {
        self.valid_root(root)?;
        if self.index.get(root, &[FACT, id, 0, 0]).is_none() {
            return Err(GraphError::MissingOccurrence);
        }
        let row = self.rows.get(&id).expect("live occurrence");
        let relation = row.relation;
        let args = row.args.clone();
        Ok(self.update(root, id, relation, args, support))
    }

    fn update(
        &mut self,
        base: Root,
        id: u64,
        relation: usize,
        args: Arc<Vec<u64>>,
        support: Condition,
    ) -> Update {
        // Staging the fact root immediately makes a pending new occurrence
        // traceable. All remaining port/index updates yield separately.
        let staged = self.write(base, [FACT, id, 0, 0], support);
        let position = if staged == base {
            2 + 2 * args.len()
        } else {
            1
        };
        Update {
            base,
            staged,
            id,
            relation,
            port_base: self.port_bases[relation],
            args,
            support,
            position,
        }
    }

    fn write(&mut self, root: Root, key: Key, support: Condition) -> Root {
        if support == Condition::FALSE {
            self.index.remove(root, &key)
        } else {
            self.index.insert(root, key, support)
        }
    }

    pub fn relation(&self, root: Root, relation: usize) -> Result<Occurrences, GraphError> {
        self.valid_root(root)?;
        if relation >= self.arities.len() {
            return Err(GraphError::InvalidRelation);
        }
        Ok(Occurrences {
            cursor: self.index.range(
                root,
                [RELATION, relation as u64, 0, 0],
                [RELATION, relation as u64, u64::MAX, 0],
            ),
            id_word: 2,
        })
    }

    /// Raw variable port lookup. Identity-aware matching combines supported
    /// equivalence members; this index does not bind or merge variables.
    pub fn port(
        &self,
        root: Root,
        relation: usize,
        port: usize,
        variable: u64,
    ) -> Result<Occurrences, GraphError> {
        self.valid_root(root)?;
        let &arity = self
            .arities
            .get(relation)
            .ok_or(GraphError::InvalidRelation)?;
        if port >= arity {
            return Err(GraphError::InvalidPort);
        }
        let port = self.port_bases[relation] + port as u64;
        Ok(Occurrences {
            cursor: self.index.range(
                root,
                [PORT, port, variable, 0],
                [PORT, port, variable, u64::MAX],
            ),
            id_word: 3,
        })
    }

    pub fn incidence(&self, root: Root, variable: u64) -> Occurrences {
        Occurrences {
            cursor: self.index.range(
                root,
                [INCIDENCE, variable, 0, 0],
                [INCIDENCE, variable, u64::MAX, 0],
            ),
            id_word: 2,
        }
    }

    /// Include current roots, staged updates and explicit inspections. Emit
    /// supports for the condition collector while marking payload ownership.
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<'_, I> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("graph collection epoch exhausted");
        Collector {
            index: self.index.collect(roots),
            rows: &mut self.rows,
            epoch: self.epoch,
            sweep: None,
            done: false,
        }
    }
}

pub struct Occurrences {
    cursor: store::Cursor,
    id_word: usize,
}
impl Occurrences {
    pub fn root(&self) -> Root {
        self.cursor.root()
    }
    pub fn visits(&self) -> u64 {
        self.cursor.visits()
    }
    pub fn next(&mut self, graph: &Graph) -> Option<(u64, Condition)> {
        self.cursor
            .next(&graph.index)
            .map(|(key, support)| (key[self.id_word], support))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Pending,
    Complete(Root),
}

/// A mutation's private index root. Only `Complete` may be published as a
/// coherent query state. The base and staged roots are collection dependencies.
pub struct Update {
    base: Root,
    staged: Root,
    id: u64,
    relation: usize,
    port_base: u64,
    args: Arc<Vec<u64>>,
    support: Condition,
    position: usize,
}
impl Update {
    pub fn occurrence(&self) -> u64 {
        self.id
    }
    pub fn roots(&self) -> [Root; 2] {
        [self.base, self.staged]
    }
    pub fn tick(&mut self, graph: &mut Graph) -> UpdateStatus {
        if self.position >= 2 + 2 * self.args.len() {
            return UpdateStatus::Complete(self.staged);
        }
        let key = if self.position == 1 {
            [RELATION, self.relation as u64, self.id, 0]
        } else {
            let port = (self.position - 2) / 2;
            if self.position.is_multiple_of(2) {
                [PORT, self.port_base + port as u64, self.args[port], self.id]
            } else {
                [INCIDENCE, self.args[port], self.id, 0]
            }
        };
        self.staged = graph.write(self.staged, key, self.support);
        self.position += 1;
        if self.position == 2 + 2 * self.args.len() {
            UpdateStatus::Complete(self.staged)
        } else {
            UpdateStatus::Pending
        }
    }
}

pub struct Collector<'a, I> {
    index: store::Collector<'a, Condition, I>,
    rows: &'a mut BTreeMap<u64, Row>,
    epoch: u64,
    sweep: Option<u64>,
    done: bool,
}
impl<I: Iterator<Item = Root>> Collector<'_, I> {
    pub fn done(&self) -> bool {
        self.done
    }
    pub fn tick(&mut self) -> Option<Condition> {
        if self.done {
            return None;
        }
        if !self.index.done() {
            if let Some((key, support)) = self.index.tick() {
                if key[0] == FACT {
                    self.rows
                        .get_mut(&key[1])
                        .expect("live occurrence payload")
                        .marked = self.epoch;
                }
                return Some(support);
            }
        } else {
            let next = match self.sweep {
                Some(id) => self.rows.range((Excluded(id), Unbounded)).next(),
                None => self.rows.first_key_value(),
            };
            if let Some((&id, row)) = next {
                if row.marked != self.epoch {
                    self.rows.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}
