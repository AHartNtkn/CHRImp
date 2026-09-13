//! Lazy causal alternatives and scalar projection of a completed graph snapshot.
//!
//! Snapshot support summaries prune unrelated births and rows. DFS retains only
//! pending scopes, never a Cartesian answer list; decisions preserve duplicate
//! successful histories.

mod index;
use index::{Found, Index, Search};

use crate::condition::{Arena, Condition, Job, Operation, poll};
use crate::engine::{Birth, Completion};
use crate::gc::discard_slot;
use crate::graph::{Graph, Occurrences};
use crate::identity::{Resolve, ResolveStatus};
use crate::program::Prepared;
use crate::store::Root;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Output {
    Begin { completion: u64, alternative: u64 },
    Variable { slot: usize, variable: u64 },
    Fact { occurrence: u64, relation: usize },
    Port { variable: u64 },
    EndFact,
    PendingBegin { event: u64 },
    Expression { operator: ExpressionKind },
    ExpressionRelation { relation: usize },
    ExpressionVariable { variable: u64 },
    ExpressionEnd,
    PendingEnd,
    End,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionKind {
    And,
    Or,
    Equal,
    True,
    Fail,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ObserveStatus {
    Pending,
    Event(Output),
    Done,
}
#[derive(Clone)]
struct Frame {
    pending: Option<Arc<Vec<usize>>>,
    scope: Condition,
}
#[derive(Clone, Copy)]
enum Phase {
    IndexBirths,
    IndexRows,
    BuildBirths,
    BuildRows,
    FindBirth,
    History,
    Inactive,
    Active,
    Left,
    Right,
    Variables,
    ResolveVariable,
    Relations,
    Rows,
    Ports,
    ResolvePort,
    Cleanup,
    Done,
}

/// Keep `graph_root()` for graph collection and `condition_roots()` for arena
/// collection while suspended. The caller separately roots the birth table.
/// Graph and birth records through `last_choice` must remain a coherent snapshot.
pub struct Observe {
    births_index: Index,
    rows_index: Index,
    search: Option<Search>,
    indexed_birth: Option<u64>,
    completion: u64,
    root: Root,
    last_choice: Option<u64>,
    code: Arc<Prepared>,
    variables: Arc<Vec<u64>>,
    stack: Vec<Frame>,
    current: Frame,
    birth_support: Condition,
    decision: Condition,
    inactive: Condition,
    active: Condition,
    left: Condition,
    boolean: Option<Job>,
    alternative: u64,
    slot: usize,
    relation: usize,
    rows: Option<Occurrences>,
    occurrence: u64,
    arguments: Option<Arc<Vec<u64>>>,
    port: usize,
    resolve: Option<Resolve>,
    representative: Option<u64>,
    phase: Phase,
    discarding: bool,
}
impl Observe {
    pub fn new(
        completion: Completion,
        code: Arc<Prepared>,
        query_variables: Arc<Vec<u64>>,
    ) -> Self {
        Self {
            births_index: Index::default(),
            rows_index: Index::default(),
            search: None,
            indexed_birth: None,
            completion: completion.id,
            root: completion.state.graph,
            last_choice: completion.last_choice,
            code,
            variables: query_variables,
            stack: if completion.support == Condition::FALSE {
                Vec::new()
            } else {
                vec![Frame {
                    pending: None,
                    scope: completion.support,
                }]
            },
            current: Frame {
                pending: None,
                scope: Condition::FALSE,
            },
            birth_support: Condition::FALSE,
            decision: Condition::FALSE,
            inactive: Condition::FALSE,
            active: Condition::FALSE,
            left: Condition::FALSE,
            boolean: None,
            alternative: 0,
            slot: 0,
            relation: 0,
            rows: None,
            occurrence: 0,
            arguments: None,
            port: 0,
            resolve: None,
            representative: None,
            phase: if completion.support == Condition::FALSE {
                Phase::History
            } else {
                Phase::IndexBirths
            },
            discarding: false,
        }
    }
    pub(crate) fn last_choice(&self) -> Option<u64> {
        self.last_choice
    }
    pub fn graph_root(&self) -> Root {
        self.root.clone()
    }
    pub(crate) fn current_scope(&self) -> Condition {
        self.current.scope
    }
    pub fn completion_id(&self) -> u64 {
        self.completion
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [
            self.current.scope,
            self.birth_support,
            self.decision,
            self.inactive,
            self.active,
            self.left,
        ]
        .into_iter()
        .chain(self.births_index.roots())
        .chain(self.rows_index.roots())
        .chain(self.search.iter().flat_map(Search::roots))
        .chain(self.stack.iter().map(|f| f.scope))
        .chain(self.boolean.iter().flat_map(Job::roots))
        .chain(self.resolve.iter().flat_map(Resolve::condition_roots))
    }
    fn push(&mut self, scope: Condition) {
        if scope != Condition::FALSE {
            self.stack.push(Frame {
                pending: self.current.pending.clone(),
                scope,
            });
        }
    }
    fn start_resolve(&mut self, g: &Graph, variable: u64, phase: Phase) {
        self.resolve = Some(Resolve::new(
            g,
            self.root.clone(),
            variable,
            self.current.scope,
        ));
        self.representative = None;
        self.phase = phase;
    }
    /// Stop projecting immediately; discard at most one nested continuation
    /// step per call. History frames release their shared cursor backing one
    /// frame at a time; cursor and argument payloads contain only scalars. Keep graph_root() traced until this token is dropped.
    pub fn discard_tick(&mut self) -> bool {
        if !self.discarding {
            self.discarding = true;
            self.current.pending = None;
            self.current.scope = Condition::FALSE;
            self.birth_support = Condition::FALSE;
            self.decision = Condition::FALSE;
            self.inactive = Condition::FALSE;
            self.active = Condition::FALSE;
            self.left = Condition::FALSE;
            self.rows = None;
            self.arguments = None;
            self.variables = Arc::new(Vec::new());
            self.representative = None;
        }
        if self.stack.pop().is_some() {
            return false;
        }
        self.stack = Vec::new();
        if !self.births_index.discard_tick() || !self.rows_index.discard_tick() {
            return false;
        }
        if discard_slot(&mut self.search, |child| child.discard_tick()) {
            return false;
        }
        if discard_slot(&mut self.boolean, |child| child.discard_tick()) {
            return false;
        }
        if discard_slot(&mut self.resolve, |child| child.discard_tick()) {
            return false;
        }
        true
    }

    pub fn tick(
        &mut self,
        g: &Graph,
        a: &mut Arena,
        births: &BTreeMap<u64, Birth>,
    ) -> ObserveStatus {
        assert!(!self.discarding, "observation has been discarded");
        match self.phase {
            Phase::IndexBirths => {
                let next = self.last_choice.and_then(|last| {
                    births
                        .range((
                            self.indexed_birth.map_or(Unbounded, Excluded),
                            Included(last),
                        ))
                        .next()
                });
                if let Some((&id, birth)) = next {
                    self.births_index.insert(id, birth.support);
                    self.indexed_birth = Some(id);
                } else {
                    self.phase = Phase::IndexRows;
                }
            }
            Phase::IndexRows => {
                if let Some(rows) = &mut self.rows {
                    if let Some((id, support)) = rows.next(g) {
                        self.rows_index.insert(id, support);
                    } else {
                        self.rows = None;
                        self.relation += 1;
                    }
                } else if self.relation < self.code.signatures.len() {
                    self.rows = Some(
                        g.relation(self.root.clone(), self.relation)
                            .expect("prepared relation signature"),
                    );
                } else {
                    self.phase = Phase::BuildBirths;
                }
            }
            Phase::BuildBirths => {
                if self.births_index.build(a) {
                    self.phase = Phase::BuildRows;
                }
            }
            Phase::BuildRows => {
                if self.rows_index.build(a) {
                    self.phase = Phase::History;
                }
            }
            Phase::History => {
                let Some(frame) = self.stack.pop() else {
                    self.stack = Vec::new();
                    self.current.scope = Condition::FALSE;
                    self.current.pending = None;
                    self.phase = Phase::Cleanup;
                    return ObserveStatus::Pending;
                };
                self.search = Some(match &frame.pending {
                    Some(pending) => Search::resume(frame.scope, pending),
                    None => self.births_index.search(frame.scope),
                });
                self.current = frame;
                self.phase = Phase::FindBirth;
            }
            Phase::FindBirth => match self.search.as_mut().unwrap().tick(&self.births_index, a) {
                Found::Pending => {}
                Found::Value(id) => {
                    self.current.pending = Some(self.search.as_ref().unwrap().continuation());
                    self.search = None;
                    let birth = &births[&id];
                    self.birth_support = birth.support;
                    self.decision = birth.decision;
                    self.boolean =
                        Some(a.start(Operation::Difference(self.current.scope, birth.support)));
                    self.phase = Phase::Inactive;
                }
                Found::Done => {
                    self.search = None;
                    self.slot = 0;
                    self.relation = 0;
                    self.phase = Phase::Variables;
                    return ObserveStatus::Event(Output::Begin {
                        completion: self.completion,
                        alternative: self.alternative,
                    });
                }
            },
            Phase::Inactive => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.inactive = c;
                    self.boolean =
                        Some(a.start(Operation::And(self.current.scope, self.birth_support)));
                    self.phase = Phase::Active;
                }
            }
            Phase::Active => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.active = c;
                    self.boolean = Some(a.start(Operation::And(c, self.decision)));
                    self.phase = Phase::Left;
                }
            }
            Phase::Left => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.left = c;
                    self.boolean = Some(a.start(Operation::And(self.active, self.decision.not())));
                    self.phase = Phase::Right;
                }
            }
            Phase::Right => {
                if let Some(right) = poll(&mut self.boolean, a) {
                    self.push(right);
                    self.push(self.left);
                    self.push(self.inactive);
                    self.birth_support = Condition::FALSE;
                    self.decision = Condition::FALSE;
                    self.inactive = Condition::FALSE;
                    self.active = Condition::FALSE;
                    self.left = Condition::FALSE;
                    self.current.scope = Condition::FALSE;
                    self.phase = Phase::History;
                }
            }
            Phase::Variables => {
                if let Some(&variable) = self.variables.get(self.slot) {
                    self.start_resolve(g, variable, Phase::ResolveVariable);
                } else {
                    self.phase = Phase::Relations;
                }
            }
            Phase::ResolveVariable | Phase::ResolvePort => {
                match self.resolve.as_mut().expect("variable resolver").tick(g, a) {
                    ResolveStatus::Found { variable, .. } => {
                        assert!(
                            self.representative.replace(variable).is_none(),
                            "multiple representatives in one causal history"
                        );
                    }
                    ResolveStatus::Pending => {}
                    ResolveStatus::Done => {
                        let variable = self
                            .representative
                            .take()
                            .expect("missing representative in nonempty history");
                        self.resolve = None;
                        let output = if matches!(self.phase, Phase::ResolveVariable) {
                            let slot = self.slot;
                            self.slot += 1;
                            self.phase = Phase::Variables;
                            Output::Variable { slot, variable }
                        } else {
                            self.port += 1;
                            self.phase = Phase::Ports;
                            Output::Port { variable }
                        };
                        return ObserveStatus::Event(output);
                    }
                }
            }
            Phase::Relations => {
                self.search = Some(self.rows_index.search(self.current.scope));
                self.phase = Phase::Rows;
            }
            Phase::Rows => match self.search.as_mut().unwrap().tick(&self.rows_index, a) {
                Found::Pending => {}
                Found::Value(occurrence) => {
                    self.occurrence = occurrence;
                    self.relation = g.fact(self.root.clone(), occurrence).unwrap().relation;
                    self.arguments = Some(g.arguments(occurrence));
                    self.port = 0;
                    self.phase = Phase::Ports;
                    return ObserveStatus::Event(Output::Fact {
                        occurrence,
                        relation: self.relation,
                    });
                }
                Found::Done => {
                    self.search = None;
                    self.alternative = self
                        .alternative
                        .checked_add(1)
                        .expect("alternative identity exhausted");
                    self.current.scope = Condition::FALSE;
                    self.phase = Phase::History;
                    return ObserveStatus::Event(Output::End);
                }
            },
            Phase::Ports => {
                if let Some(&variable) = self
                    .arguments
                    .as_ref()
                    .expect("fact arguments")
                    .get(self.port)
                {
                    self.start_resolve(g, variable, Phase::ResolvePort);
                } else {
                    self.arguments = None;
                    self.phase = Phase::Rows;
                    return ObserveStatus::Event(Output::EndFact);
                }
            }
            Phase::Cleanup => {
                if self.births_index.discard_tick() && self.rows_index.discard_tick() {
                    self.phase = Phase::Done;
                    return ObserveStatus::Done;
                }
            }
            Phase::Done => return ObserveStatus::Done,
        }
        ObserveStatus::Pending
    }
}

impl Trace for Frame {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.scope]),
            _ => Step::Done,
        }
    }
}

impl Trace for Observe {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[
                self.current.scope,
                self.birth_support,
                self.decision,
                self.inactive,
                self.active,
                self.left,
            ]),
            1 => cursor.vector(self.stack.len(), |i, child| self.stack[i].trace(child)),
            2 => cursor.optional(self.boolean.as_ref()),
            3 => cursor.optional(self.resolve.as_ref()),
            4 => cursor.optional(Some(&self.births_index)),
            5 => cursor.optional(Some(&self.rows_index)),
            6 => cursor.optional(self.search.as_ref()),
            _ => Step::Done,
        }
    }
}
