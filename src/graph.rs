//! Shared relational occurrence state and ordered-port indexes.

pub(crate) mod constructors;
mod prune;
pub(crate) mod restriction;
pub use prune::Prune;
#[cfg(feature = "diagnostics")]
pub use restriction::RestrictionDiagnostics;

use crate::condition::Condition;
use crate::identity::{CHILD, PARENT};
use crate::program::Signature;
use crate::store::{self, Key, Root, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

const FACT: u64 = 0;
const RELATION: u64 = 1;
const PORT: u64 = 2;
const INCIDENCE: u64 = 3;

// Hash collisions only broaden a candidate bucket; matching checks every port.
pub(crate) fn tuple_hash(hash: u64, variable: u64) -> u64 {
    hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(variable)
}

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

#[derive(Clone, Copy)]
enum IndexWrite {
    Relation,
    Port(usize),
    Incidence(usize),
    Tuple,
}

/// Immutable graph-update code shared with the prepared source. Every physical
/// write is explicit; omitted field indexes incur neither writes nor skip jobs.
pub(crate) struct UpdatePlan {
    ports: Vec<bool>,
    writes: Vec<IndexWrite>,
}
#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Default, Debug, serde::Serialize)]
pub struct FieldUpdateDiagnostics {
    /// All calls to Graph::write, including identities and attachments.
    pub graph_writes: u64,
    /// Executed writes from a compiler-certified sparse update plan.
    pub specialized_writes: u64,
    /// Missing port writes paired with executed incidence writes; cancellation
    /// before reaching that field contributes nothing.
    pub avoided_port_writes: u64,
    pub fallback_lookups: u64,
    pub fallback_candidates: u64,
}
impl UpdatePlan {
    #[cfg(feature = "diagnostics")]
    pub(crate) fn statistics(plans: Option<&Arc<[Option<Self>]>>) -> (usize, usize, usize, usize) {
        let Some(plans) = plans else {
            return (0, 0, 0, 0);
        };
        let mut result = (0, 0, 0, std::mem::size_of_val(plans.as_ref()));
        for plan in plans.iter().flatten() {
            result.0 += 1;
            result.1 += plan.ports.iter().filter(|p| !**p).count();
            result.2 += plan.writes.len();
            result.3 +=
                plan.ports.capacity() + plan.writes.capacity() * std::mem::size_of::<IndexWrite>();
        }
        result
    }
    pub(crate) fn prepare(
        signatures: &[Signature],
        tuples: &[bool],
        ports: Option<Vec<Vec<bool>>>,
    ) -> Option<Arc<[Option<Self>]>> {
        let ports = ports?;
        if ports.iter().flatten().all(|p| *p) {
            return None;
        }
        Some(
            signatures
                .iter()
                .enumerate()
                .map(|(r, s)| {
                    let ports = ports[r].clone();
                    debug_assert_eq!(ports.len(), s.arity);
                    if ports.iter().all(|p| *p) {
                        return None;
                    }
                    let mut writes = vec![IndexWrite::Relation];
                    for (port, &indexed) in ports.iter().enumerate() {
                        if indexed {
                            writes.push(IndexWrite::Port(port));
                        }
                        writes.push(IndexWrite::Incidence(port));
                    }
                    if tuples[r] {
                        writes.push(IndexWrite::Tuple);
                    }
                    Some(Self { ports, writes })
                })
                .collect(),
        )
    }
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
    #[cfg(feature = "diagnostics")]
    field_work: std::cell::Cell<FieldUpdateDiagnostics>,
    pub(crate) shared_restrictions: restriction::Cache,
    semantic_debt: usize,
    pub(crate) index: Store<Condition>,
    rows: BTreeMap<u64, Row>,
    unprotected: Option<BTreeSet<u64>>,
    arities: Vec<usize>,
    tuple_indexes: Vec<bool>,
    updates: Option<Arc<[Option<UpdatePlan>]>>,
    port_bases: Vec<u64>,
    next_occurrence: u64,
    epoch: u64,
}

impl Graph {
    /// Standalone graphs index every whole tuple with at least two ports.
    pub fn new(signatures: &[Signature]) -> Self {
        Self::with_tuple_indexes(
            signatures,
            &signatures.iter().map(|s| s.arity >= 2).collect::<Vec<_>>(),
        )
    }

    /// Immutable index configuration: one flag per relation signature. Ordinary
    /// port and incidence indexes always exist; whole-tuple indexes require arity >= 2.
    pub fn with_tuple_indexes(signatures: &[Signature], tuple_indexes: &[bool]) -> Self {
        Self::with_update_plans(signatures, tuple_indexes, None)
    }

    pub(crate) fn with_update_plans(
        signatures: &[Signature],
        tuple_indexes: &[bool],
        updates: Option<Arc<[Option<UpdatePlan>]>>,
    ) -> Self {
        assert_eq!(
            signatures.len(),
            tuple_indexes.len(),
            "one tuple-index flag per relation"
        );
        assert!(
            signatures
                .iter()
                .zip(tuple_indexes)
                .all(|(s, &enabled)| !enabled || s.arity >= 2),
            "whole-tuple indexes require at least two ports"
        );
        let mut next = 0_u64;
        let port_bases = signatures
            .iter()
            .zip(tuple_indexes)
            .map(|(signature, &enabled)| {
                let base = next;
                next = next
                    .checked_add(signature.arity as u64 + u64::from(enabled))
                    .expect("port identity exhausted");
                base
            })
            .collect();
        Self {
            #[cfg(feature = "diagnostics")]
            field_work: std::cell::Cell::default(),
            shared_restrictions: restriction::Cache::default(),
            semantic_debt: 0,
            index: Store::default(),
            rows: BTreeMap::new(),
            unprotected: None,
            arities: signatures.iter().map(|s| s.arity).collect(),
            tuple_indexes: tuple_indexes.to_vec(),
            updates,
            port_bases,
            next_occurrence: 0,
            epoch: 0,
        }
    }
    pub fn index_allocations(&self) -> usize {
        self.index.allocations()
    }
    #[cfg(feature = "diagnostics")]
    pub fn field_update_diagnostics(&self) -> FieldUpdateDiagnostics {
        self.field_work.get()
    }
    fn update_plan(&self, relation: usize) -> Option<&UpdatePlan> {
        self.updates.as_ref().and_then(|p| p[relation].as_ref())
    }
    fn has_port_index(&self, relation: usize, port: usize) -> bool {
        self.update_plan(relation).is_none_or(|p| p.ports[port])
    }
    fn update_length(&self, relation: usize, arity: usize) -> usize {
        self.update_plan(relation).map_or(
            1 + 2 * arity + usize::from(self.tuple_indexes[relation]),
            |p| p.writes.len(),
        )
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
        if let Some(ids) = &mut self.unprotected {
            ids.insert(id);
        }
        self.rows.insert(
            id,
            Row {
                relation,
                args: args.clone(),
                marked: 0,
            },
        );
        Ok(self.update(
            root,
            id,
            relation,
            args,
            support,
            support != Condition::FALSE,
        ))
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
        let old = self
            .index
            .get(&root, &[FACT, id, 0, 0])
            .ok_or(GraphError::MissingOccurrence)?;
        let row = self.rows.get(&id).expect("live occurrence");
        let relation = row.relation;
        let args = row.args.clone();
        Ok(self.update(root, id, relation, args, support, old != support))
    }

    fn update(
        &mut self,
        base: Root,
        id: u64,
        relation: usize,
        args: Arc<Vec<u64>>,
        support: Condition,
        changed: bool,
    ) -> Update {
        // Staging the fact root immediately makes a pending new occurrence
        // traceable. All remaining port/index updates yield separately.
        let staged = self.write(base, [FACT, id, 0, 0], support);
        let position = if !changed {
            self.update_length(relation, args.len())
        } else {
            0
        };
        Update {
            owner: self.index.owner(),
            staged,
            id,
            relation,
            port_base: self.port_bases[relation],
            tuple_hash: 0,
            args,
            support,
            position,
        }
    }

    pub(crate) fn write(&mut self, root: Root, key: Key, support: Condition) -> Root {
        #[cfg(feature = "diagnostics")]
        self.field_work.update(|mut w| {
            w.graph_writes += 1;
            w
        });
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
            filter: None,
        })
    }

    /// Relation cardinality from cached subtree counts at a pinned root.
    pub(crate) fn relation_count(&self, root: &Root, relation: usize) -> usize {
        self.index.count(
            root,
            [RELATION, relation as u64, 0, 0],
            [RELATION, relation as u64, u64::MAX, 0],
        )
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
        if !self.has_port_index(relation, port) {
            #[cfg(feature = "diagnostics")]
            self.field_work.update(|mut w| {
                w.fallback_lookups += 1;
                w
            });
            // Public raw lookup retains exact occurrence/port semantics. Source
            // matching cannot request this field under the whole-program proof.
            // Incidence is retained for identity wakeup and ownership regardless.
            let mut occurrences = self.incidence(root, variable);
            occurrences.filter = Some((relation, port));
            return Ok(occurrences);
        }
        let port = self.port_bases[relation] + port as u64;
        Ok(Occurrences {
            cursor: self.index.range(
                root,
                [PORT, port, variable, 0],
                [PORT, port, variable, u64::MAX],
            ),
            id_word: 3,
            filter: None,
        })
    }

    /// Raw bucket cardinality from cached subtree counts, without visiting rows.
    /// Callers supply a prepared relation and port at a valid pinned root.
    pub(crate) fn port_count(
        &self,
        root: &Root,
        relation: usize,
        port: usize,
        variable: u64,
    ) -> usize {
        assert!(port < self.arities[relation]);
        if !self.has_port_index(relation, port) {
            // Defensive fallback for direct library matchers over this graph.
            // This is an upper bound, so it cannot hide candidate occurrences.
            return self.relation_count(root, relation);
        }
        let port = self.port_bases[relation] + port as u64;
        self.index.count(
            root,
            [PORT, port, variable, 0],
            [PORT, port, variable, u64::MAX],
        )
    }

    /// No support-bearing identity edge touches this variable in this snapshot.
    /// Absence in both directions certifies a singleton on every scope.
    pub(crate) fn singleton(&self, root: &Root, variable: u64) -> bool {
        [PARENT, CHILD].into_iter().all(|namespace| {
            self.index.count(
                root,
                [namespace, variable, 0, 0],
                [namespace, variable, u64::MAX, 0],
            ) == 0
        })
    }

    pub(crate) fn has_tuple_index(&self, relation: usize) -> bool {
        self.tuple_indexes[relation]
    }

    pub(crate) fn tuple(&self, root: Root, relation: usize, hash: u64) -> Occurrences {
        assert!(
            self.tuple_indexes[relation],
            "whole-tuple index is not configured"
        );
        let port = self.port_bases[relation] + self.arities[relation] as u64;
        Occurrences {
            cursor: self
                .index
                .range(root, [PORT, port, hash, 0], [PORT, port, hash, u64::MAX]),
            id_word: 3,
            filter: None,
        }
    }

    pub(crate) fn tuple_count(&self, root: &Root, relation: usize, hash: u64) -> usize {
        assert!(
            self.tuple_indexes[relation],
            "whole-tuple index is not configured"
        );
        let port = self.port_bases[relation] + self.arities[relation] as u64;
        self.index
            .count(root, [PORT, port, hash, 0], [PORT, port, hash, u64::MAX])
    }

    pub fn incidence(&self, root: Root, variable: u64) -> Occurrences {
        Occurrences {
            cursor: self.index.range(
                root,
                [INCIDENCE, variable, 0, 0],
                [INCIDENCE, variable, u64::MAX, 0],
            ),
            id_word: 2,
            filter: None,
        }
    }

    /// Include current roots, staged updates and explicit inspections. Emit
    /// supports for the condition collector while marking payload ownership.
    /// The nested index lease freezes graph writes through metadata sweep and
    /// is released only when this token is dropped (finished or aborted).
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<I> {
        self.collect_archived(roots, vec![], true)
    }
    pub(crate) fn collect_archived<I: Iterator<Item = Root>>(
        &mut self,
        roots: I,
        archived: Vec<Root>,
        mut reset: bool,
    ) -> Collector<I> {
        self.shared_restrictions.clear();
        self.index.assert_mutable();
        let epoch = self
            .epoch
            .checked_add(1)
            .expect("graph collection epoch exhausted");
        let deactivate = reset && archived.is_empty() && self.unprotected.is_some();
        if !archived.is_empty() && self.unprotected.is_none() {
            self.unprotected = Some(BTreeSet::new());
            reset = true;
        }
        let index = self.index.collect_archived(roots, archived, reset);
        self.epoch = epoch;
        Collector {
            index,
            epoch,
            sweep: None,
            deactivate,
            reset: deactivate
                || reset
                    && self
                        .unprotected
                        .as_ref()
                        .is_some_and(|ids| ids.len() != self.rows.len()),
            done: false,
        }
    }
}

pub struct Occurrences {
    cursor: store::Cursor,
    id_word: usize,
    filter: Option<(usize, usize)>,
}
impl Occurrences {
    pub fn root(&self) -> Root {
        self.cursor.root()
    }
    pub fn visits(&self) -> u64 {
        self.cursor.visits()
    }
    pub fn next(&mut self, graph: &Graph) -> Option<(u64, Condition)> {
        while let Some((key, support)) = self.cursor.next(&graph.index) {
            let id = key[self.id_word];
            if let Some((relation, port)) = self.filter {
                #[cfg(feature = "diagnostics")]
                graph.field_work.update(|mut w| {
                    w.fallback_candidates += 1;
                    w
                });
                let row = graph.rows.get(&id).expect("pinned incidence payload");
                if row.relation != relation || row.args[port] != key[1] {
                    continue;
                }
            }
            return Some((id, support));
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Pending,
    Complete(Root),
}

/// A mutation's private index root. Only `Complete` may be published as a
/// coherent query state. Trace the staged root; callers own earlier snapshots.
pub struct Update {
    owner: u32,
    staged: Root,
    id: u64,
    relation: usize,
    port_base: u64,
    tuple_hash: u64,
    args: Arc<Vec<u64>>,
    support: Condition,
    position: usize,
}
impl Update {
    pub fn occurrence(&self) -> u64 {
        self.id
    }
    pub fn roots(&self) -> [Root; 1] {
        [self.staged.clone()]
    }
    pub fn tick(&mut self, graph: &mut Graph) -> UpdateStatus {
        graph.index.assert_mutable();
        assert_eq!(self.owner, graph.index.owner(), "foreign graph update");
        let end = graph.update_length(self.relation, self.args.len());
        if self.position >= end {
            return UpdateStatus::Complete(self.staged.clone());
        }
        let instruction = if let Some(plan) = graph.update_plan(self.relation) {
            #[cfg(feature = "diagnostics")]
            graph.field_work.update(|mut w| {
                w.specialized_writes += 1;
                if let IndexWrite::Incidence(port) = plan.writes[self.position] {
                    w.avoided_port_writes += u64::from(!plan.ports[port]);
                }
                w
            });
            plan.writes[self.position]
        } else if self.position == 0 {
            IndexWrite::Relation
        } else if self.position == 1 + 2 * self.args.len() {
            IndexWrite::Tuple
        } else if self.position % 2 == 1 {
            IndexWrite::Port((self.position - 1) / 2)
        } else {
            IndexWrite::Incidence((self.position - 1) / 2)
        };
        let key = match instruction {
            IndexWrite::Relation => [RELATION, self.relation as u64, self.id, 0],
            IndexWrite::Tuple => [
                PORT,
                self.port_base + self.args.len() as u64,
                self.tuple_hash,
                self.id,
            ],
            IndexWrite::Port(port) => {
                [PORT, self.port_base + port as u64, self.args[port], self.id]
            }
            IndexWrite::Incidence(port) => {
                if graph.tuple_indexes[self.relation] {
                    self.tuple_hash = tuple_hash(self.tuple_hash, self.args[port]);
                }
                [INCIDENCE, self.args[port], self.id, 0]
            }
        };
        self.staged = graph.write(std::mem::take(&mut self.staged), key, self.support);
        self.position += 1;
        if self.position == end {
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
    reset: bool,
    deactivate: bool,
    done: bool,
}
impl<I: Iterator<Item = Root>> Collector<I> {
    pub fn done(&self) -> bool {
        self.done
    }
    pub(crate) fn archiving(&self) -> bool {
        self.index.archiving()
    }
    pub fn tick(&mut self, graph: &mut Graph) -> Option<Condition> {
        self.index.validate(&graph.index);
        assert_eq!(self.epoch, graph.epoch, "stale graph collector");
        if self.done {
            return None;
        }
        if self.reset {
            if self.deactivate {
                if graph.unprotected.as_mut().unwrap().pop_first().is_none() {
                    graph.unprotected = None;
                    self.reset = false;
                }
                return None;
            }
            let next = match self.sweep {
                Some(id) => graph.rows.range((Excluded(id), Unbounded)).next(),
                None => graph.rows.first_key_value(),
            }
            .map(|(&id, _)| id);
            if let Some(id) = next {
                graph.unprotected.as_mut().unwrap().insert(id);
                self.sweep = Some(id);
            } else {
                self.reset = false;
                self.sweep = None;
            }
            return None;
        }
        if !self.index.done() {
            if let Some((key, support)) = self.index.tick(&mut graph.index) {
                if key[0] == FACT {
                    if self.index.archiving() {
                        graph.unprotected.as_mut().unwrap().remove(&key[1]);
                    }
                    graph
                        .rows
                        .get_mut(&key[1])
                        .expect("live occurrence payload")
                        .marked = self.epoch;
                }
                return Some(support);
            }
        } else {
            let next = if let Some(ids) = &graph.unprotected {
                match self.sweep {
                    Some(id) => ids.range((Excluded(id), Unbounded)).next(),
                    None => ids.first(),
                }
                .copied()
            } else {
                match self.sweep {
                    Some(id) => graph.rows.range((Excluded(id), Unbounded)).next(),
                    None => graph.rows.first_key_value(),
                }
                .map(|(&id, _)| id)
            };
            if let Some(id) = next {
                let row = &graph.rows[&id];
                if row.marked != self.epoch {
                    graph.rows.remove(&id);
                    if let Some(ids) = &mut graph.unprotected {
                        ids.remove(&id);
                    }
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

#[cfg(test)]
mod update_ownership_tests {
    use super::*;
    use crate::condition::Arena;
    fn graph(arity: usize) -> Graph {
        Graph::new(&[Signature {
            name: "p".into(),
            arity,
        }])
    }
    fn finish(g: &mut Graph, mut u: Update) -> Root {
        loop {
            if let UpdateStatus::Complete(r) = u.tick(g) {
                return r;
            }
        }
    }
    #[test]
    fn certified_field_lookup_filters_incidence_and_retains_old_conditional_occurrences() {
        use crate::{
            program::prepare,
            syntax::{parse_program, parse_query},
        };
        let code = prepare(
            &parse_program("app(K,A,B) \\ app(K,C,D) <=> A=C,B=D.").unwrap(),
            &parse_query("other(A)").unwrap(),
        )
        .unwrap();
        let mut g =
            Graph::with_update_plans(&code.signatures, &code.tuple_indexes, code.graph_updates);
        let mut a = Arena::default();
        let c = a.fresh_choice().1;
        let mut root = g.empty();
        // Shared raw identities in a different relation or wrong port are not
        // hits. Equal rows still have distinct occurrence identities.
        let mut ids = vec![];
        for (relation, args) in [
            (0, vec![1, 7, 8]),
            (0, vec![2, 7, 8]),
            (0, vec![7, 9, 7]),
            (1, vec![7]),
        ] {
            let update = g.post(root, relation, args, c).unwrap();
            ids.push(update.occurrence());
            root = finish(&mut g, update);
        }
        assert_eq!(
            g.index
                .count(&root, [PORT, 1, 0, 0], [PORT, 2, u64::MAX, u64::MAX]),
            0
        );
        let mut old = g.port(root.clone(), 0, 1, 7).unwrap();
        assert_eq!(old.next(&g), Some((ids[0], c)));
        let update = g
            .set_liveness(root.clone(), ids[1], Condition::FALSE)
            .unwrap();
        let current = finish(&mut g, update);
        let mut gc = g.collect([root, current.clone(), old.root()].into_iter());
        while !gc.done() {
            gc.tick(&mut g);
        }
        drop(gc);
        assert_eq!(old.next(&g), Some((ids[1], c)));
        assert_eq!(old.next(&g), None);
        let mut now = g.port(current.clone(), 0, 1, 7).unwrap();
        assert_eq!(now.next(&g), Some((ids[0], c)));
        assert_eq!(now.next(&g), None);
        assert_eq!(
            g.port(current.clone(), 0, 2, 7).unwrap().next(&g),
            Some((ids[2], c))
        );
        assert_eq!(g.port(current, 0, 1, 100).unwrap().next(&g), None);
        drop(old);
        drop(now);
        let mut gc = g.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut g);
        }
        drop(gc);
        while !g.release_tick() {}
        assert_eq!((g.occurrence_count(), g.index_node_count()), (0, 0));
    }
    #[test]
    fn unique_changed_support_updates_all_indexes_without_copying() {
        let mut g = graph(2);
        let mut a = Arena::default();
        let c = a.fresh_choice().1;
        let u = g.post(g.empty(), 0, vec![7, 7], Condition::TRUE).unwrap();
        let id = u.occurrence();
        let root = finish(&mut g, u);
        let before = g.index.mutation_counts();
        let u = g.set_liveness(root, id, c).unwrap();
        let root = finish(&mut g, u);
        assert_eq!(g.fact(root.clone(), id).unwrap().support, c);
        assert_eq!(g.relation(root.clone(), 0).unwrap().next(&g), Some((id, c)));
        for port in 0..2 {
            assert_eq!(
                g.port(root.clone(), 0, port, 7).unwrap().next(&g),
                Some((id, c))
            );
        }
        assert_eq!(
            g.tuple(root.clone(), 0, tuple_hash(7, 7)).next(&g),
            Some((id, c))
        );
        assert_eq!(g.incidence(root, 7).next(&g), Some((id, c)));
        assert_eq!(
            g.index.mutation_counts().1,
            before.1,
            "sole owner copied index paths"
        );
    }
    #[test]
    fn staged_fact_or_owned_arguments_suffice_through_collection_each_tick() {
        for arity in [0, 1, 3] {
            for remove in [false, true] {
                for false_post in [false, true] {
                    let mut g = graph(arity);
                    let mut a = Arena::default();
                    let c = a.fresh_choice().1;
                    let args = vec![7; arity];
                    let support = if false_post { Condition::FALSE } else { c };
                    let mut u = g.post(g.empty(), 0, args.clone(), support).unwrap();
                    let id = u.occurrence();
                    if remove && !false_post {
                        let root = finish(&mut g, u);
                        u = g.set_liveness(root, id, Condition::FALSE).unwrap();
                    }
                    let live = !remove && !false_post;
                    loop {
                        let staged = u.roots().last().unwrap().clone();
                        let mut gc = g.collect([staged].into_iter());
                        let mut conditions = vec![];
                        while !gc.done() {
                            if let Some(c) = gc.tick(&mut g) {
                                conditions.push(c)
                            }
                        }
                        drop(gc);
                        assert_eq!(g.rows.contains_key(&id), live);
                        let mut gc = a.collect(conditions.into_iter());
                        while !gc.tick(&mut a) {}
                        drop(gc);
                        if let UpdateStatus::Complete(root) = u.tick(&mut g) {
                            assert_eq!(g.fact(root.clone(), id).is_some(), live);
                            assert_eq!(
                                g.relation(root.clone(), 0).unwrap().next(&g).is_some(),
                                live
                            );
                            for port in 0..arity {
                                assert_eq!(
                                    g.port(root.clone(), 0, port, 7).unwrap().next(&g).is_some(),
                                    live
                                );
                            }
                            if arity >= 2 {
                                let hash = args.iter().fold(0, |h, &v| tuple_hash(h, v));
                                assert_eq!(g.tuple(root.clone(), 0, hash).next(&g).is_some(), live);
                                assert_eq!(g.tuple_count(&root, 0, hash), usize::from(live));
                                assert!(matches!(
                                    g.port(root.clone(), 0, arity, hash),
                                    Err(GraphError::InvalidPort)
                                ));
                            }
                            if live {
                                assert_eq!(g.fact(root, id).unwrap().args, args);
                                assert!(a.contains(c));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn empty_update_preserves_owner_and_collector_authority() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut g = graph(0);
        let mut other = graph(0);
        let mut u = g.post(g.empty(), 0, vec![], Condition::FALSE).unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| u.tick(&mut other))).is_err());
        let gc = g.collect(u.roots().into_iter());
        assert!(catch_unwind(AssertUnwindSafe(|| u.tick(&mut g))).is_err());
        drop(gc);
        assert!(matches!(u.tick(&mut g), UpdateStatus::Complete(_)));
    }
}

#[cfg(test)]
mod tuple_index_tests {
    use super::*;
    use crate::condition::Arena;
    fn finish(g: &mut Graph, mut u: Update) -> Root {
        loop {
            if let UpdateStatus::Complete(r) = u.tick(g) {
                return r;
            }
        }
    }
    #[test]
    fn tuple_support_tracks_updates_pruning_and_reclamation_without_language_ports() {
        let mut g = Graph::new(&[Signature {
            name: "p".into(),
            arity: 2,
        }]);
        let mut a = Arena::default();
        let c = a.fresh_choice().1;
        let u = g.post(g.empty(), 0, vec![7, 9], Condition::TRUE).unwrap();
        let id = u.occurrence();
        let original = finish(&mut g, u);
        let hash = tuple_hash(7, 9);
        let u = g.set_liveness(original.clone(), id, c).unwrap();
        let root = finish(&mut g, u);
        assert_eq!(
            g.tuple(original.clone(), 0, hash).next(&g),
            Some((id, Condition::TRUE))
        );
        assert_eq!(g.tuple(root.clone(), 0, hash).next(&g), Some((id, c)));
        assert!(matches!(
            g.port(root.clone(), 0, 2, hash),
            Err(GraphError::InvalidPort)
        ));
        assert!(g.incidence(root.clone(), hash).next(&g).is_none());
        let mut prune = g.prune(root.clone(), c.not());
        let empty = (0..10000).find_map(|_| prune.tick(&mut g, &mut a)).unwrap();
        drop(prune);
        assert_eq!(empty, g.empty());
        assert_eq!(g.tuple(root.clone(), 0, hash).next(&g), Some((id, c)));
        let u = g.set_liveness(root, id, Condition::FALSE).unwrap();
        let empty = finish(&mut g, u);
        assert_eq!(g.tuple_count(&empty, 0, hash), 0);
        drop(original);
        let mut gc = g.collect([empty].into_iter());
        while !gc.done() {
            gc.tick(&mut g);
        }
        drop(gc);
        assert_eq!(g.occurrence_count(), 0);
        assert_eq!(g.index_node_count(), 0);
    }
}
