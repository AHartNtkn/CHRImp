//! Shared relational occurrence state and ordered-port indexes.

mod prune;
pub use prune::Prune;

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
    semantic_debt: usize,
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
            semantic_debt: 0,
            index: Store::default(),
            rows: BTreeMap::new(),
            arities: signatures.iter().map(|s| s.arity).collect(),
            port_bases,
            next_occurrence: 0,
            epoch: 0,
        }
    }
    pub fn index_allocations(&self) -> usize {
        self.index.allocations()
    }
    pub fn release_tick(&mut self) -> bool {
        self.index.release_tick()
    }
    pub(crate) fn semantic_debt(&self) -> usize {
        self.semantic_debt
    }
    pub(crate) fn retire_scope(&mut self) {
        self.index.assert_mutable();
        self.semantic_debt = self.semantic_debt.saturating_add(1);
    }
    pub(crate) fn semantic_collected(&mut self) {
        self.semantic_debt = 0;
    }
    pub(crate) fn occurrence_frontier(&self, root: &Root) -> usize {
        self.index
            .count(root, [0; 4], [INCIDENCE, u64::MAX, u64::MAX, u64::MAX])
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
        if self.index.contains(&root) {
            Ok(())
        } else {
            Err(GraphError::InvalidRoot)
        }
    }
    pub fn fact(&self, root: Root, id: u64) -> Option<Fact<'_>> {
        let support = self.index.get(&root, &[FACT, id, 0, 0])?;
        let row = self.rows.get(&id).expect("live occurrence payload");
        Some(Fact {
            id,
            relation: row.relation,
            args: row.args.as_slice(),
            support,
        })
    }

    pub(crate) fn arguments(&self, id: u64) -> Arc<Vec<u64>> {
        self.rows
            .get(&id)
            .expect("live occurrence payload")
            .args
            .clone()
    }

    pub fn post(
        &mut self,
        root: Root,
        relation: usize,
        args: Vec<u64>,
        support: Condition,
    ) -> Result<Update, GraphError> {
        self.index.assert_mutable();
        self.valid_root(root.clone())?;
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
        self.index.assert_mutable();
        self.valid_root(root.clone())?;
        if self.index.get(&root, &[FACT, id, 0, 0]).is_none() {
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
        let staged = self.write(base.clone(), [FACT, id, 0, 0], support);
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

    pub(crate) fn write(&mut self, root: Root, key: Key, support: Condition) -> Root {
        self.index.assert_mutable();
        let previous = self.index.get(&root, &key);
        if previous.unwrap_or(Condition::FALSE) != support
            && (previous.is_some() || key[0] > INCIDENCE)
        {
            self.retire_scope();
        }
        if support == Condition::FALSE {
            self.index.remove(root, &key)
        } else {
            self.index.insert(root, key, support)
        }
    }

    pub fn relation(&self, root: Root, relation: usize) -> Result<Occurrences, GraphError> {
        self.valid_root(root.clone())?;
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
        self.valid_root(root.clone())?;
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
    /// The nested index lease freezes graph writes through metadata sweep and
    /// is released only when this token is dropped (finished or aborted).
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<I> {
        self.index.assert_mutable();
        let epoch = self
            .epoch
            .checked_add(1)
            .expect("graph collection epoch exhausted");
        let index = self.index.collect(roots);
        self.epoch = epoch;
        Collector {
            index,
            epoch,
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
        [self.base.clone(), self.staged.clone()]
    }
    pub fn tick(&mut self, graph: &mut Graph) -> UpdateStatus {
        graph.index.assert_mutable();
        if self.position >= 2 + 2 * self.args.len() {
            return UpdateStatus::Complete(self.staged.clone());
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
        self.staged = graph.write(std::mem::take(&mut self.staged), key, self.support);
        self.position += 1;
        if self.position == 2 + 2 * self.args.len() {
            UpdateStatus::Complete(self.staged.clone())
        } else {
            UpdateStatus::Pending
        }
    }
}

pub struct Collector<I> {
    index: store::Collector<I>,
    epoch: u64,
    sweep: Option<u64>,
    done: bool,
}
impl<I: Iterator<Item = Root>> Collector<I> {
    pub fn done(&self) -> bool {
        self.done
    }
    pub fn tick(&mut self, graph: &mut Graph) -> Option<Condition> {
        self.index.validate(&graph.index);
        assert_eq!(self.epoch, graph.epoch, "stale graph collector");
        if self.done {
            return None;
        }
        if !self.index.done() {
            if let Some((key, support)) = self.index.tick(&mut graph.index) {
                if key[0] == FACT {
                    graph
                        .rows
                        .get_mut(&key[1])
                        .expect("live occurrence payload")
                        .marked = self.epoch;
                }
                return Some(support);
            }
        } else {
            let next = match self.sweep {
                Some(id) => graph.rows.range((Excluded(id), Unbounded)).next(),
                None => graph.rows.first_key_value(),
            };
            if let Some((&id, row)) = next {
                if row.marked != self.epoch {
                    graph.rows.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}

#[cfg(test)]
mod pressure_checks {
    use super::*;
    fn post(g: &mut Graph, r: Root, n: u64) -> Root {
        let mut p = g.post(r, 0, vec![n], Condition::TRUE).unwrap();
        loop {
            if let UpdateStatus::Complete(r) = p.tick(g) {
                return r;
            }
        }
    }
    #[test]
    fn semantic_debt_counts_support_changes_independently_of_cow() {
        let sig = [Signature {
            name: "p".into(),
            arity: 1,
        }];
        let mut a = Graph::new(&sig);
        let mut b = Graph::new(&sig);
        let mut ra = a.empty();
        let mut rb = b.empty();
        let mut pins = vec![];
        for i in 0..64 {
            ra = post(&mut a, ra, i);
            pins.push(rb.clone());
            rb = post(&mut b, rb, i);
        }
        assert_eq!(a.semantic_debt(), 0);
        assert_eq!(b.semantic_debt(), 0);
        assert_eq!(a.occurrence_frontier(&ra), 256);
        for i in 0..64 {
            ra = a.write(ra, [4, 1000 + i, 999, 0], Condition::TRUE);
            pins.push(rb.clone());
            rb = b.write(rb, [4, 1000 + i, 999, 0], Condition::TRUE);
        }
        assert_ne!(a.index.mutation_counts(), b.index.mutation_counts());
        for id in 0..32 {
            let mut ua = a.set_liveness(ra, id, Condition::FALSE).unwrap();
            let mut ub = b.set_liveness(rb, id, Condition::FALSE).unwrap();
            ra = loop {
                if let UpdateStatus::Complete(r) = ua.tick(&mut a) {
                    break r;
                }
            };
            rb = loop {
                if let UpdateStatus::Complete(r) = ub.tick(&mut b) {
                    break r;
                }
            };
            pins.push(rb.clone());
        }
        assert_eq!(a.semantic_debt(), 192);
        assert_eq!(a.semantic_debt(), b.semantic_debt());
        assert_eq!(a.occurrence_frontier(&ra), 128);
        let debt = a.semantic_debt();
        let mut gc = a.collect([ra.clone()].into_iter());
        while !gc.done() {
            gc.tick(&mut a);
        }
        drop(gc);
        assert_eq!(
            a.semantic_debt(),
            debt,
            "physical tracing cannot discharge semantic obsolescence"
        );
        let frozen = a.collect([ra.clone()].into_iter());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.retire_scope())).is_err()
        );
        assert_eq!(a.semantic_debt(), debt);
        drop(frozen);
        a.semantic_collected();
        assert_eq!(a.semantic_debt(), 0);
    }
}
