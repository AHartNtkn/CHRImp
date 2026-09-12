//! Lazy causal alternatives and scalar projection of a completed graph snapshot.
//!
//! DFS retains only pending scopes, never a Cartesian answer list. Inactive
//! births advance once; active decisions preserve duplicate successful histories.

use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::engine::{Birth, Completion};
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
#[derive(Clone, Copy)]
struct Frame {
    after: Option<u64>,
    scope: Condition,
}
#[derive(Clone, Copy)]
enum Phase {
    History,
    Inactive,
    Active,
    Left,
    Right,
    Variables,
    ResolveVariable,
    Relations,
    Rows,
    Live,
    Ports,
    ResolvePort,
    Done,
}

/// Keep `graph_root()` for graph collection and `condition_roots()` for arena
/// collection while suspended. The caller separately roots the birth table.
/// Graph and birth records through `last_choice` must remain a coherent snapshot.
pub struct Observe {
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
            completion: completion.id,
            root: completion.state.graph,
            last_choice: completion.last_choice,
            code,
            variables: query_variables,
            stack: if completion.support == Condition::FALSE {
                Vec::new()
            } else {
                vec![Frame {
                    after: None,
                    scope: completion.support,
                }]
            },
            current: Frame {
                after: None,
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
            phase: Phase::History,
            discarding: false,
        }
    }
    pub(crate) fn last_choice(&self) -> Option<u64> {
        self.last_choice
    }
    pub fn graph_root(&self) -> Root {
        self.root
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
        .chain(self.stack.iter().map(|f| f.scope))
        .chain(self.boolean.iter().flat_map(Job::roots))
        .chain(self.resolve.iter().flat_map(Resolve::condition_roots))
    }
    fn poll(&mut self, a: &mut Arena) -> Option<Condition> {
        if let Progress::Complete(c) = self
            .boolean
            .as_mut()
            .expect("condition continuation")
            .tick(a)
        {
            self.boolean = None;
            Some(c)
        } else {
            None
        }
    }
    fn push(&mut self, scope: Condition) {
        if scope != Condition::FALSE {
            self.stack.push(Frame {
                after: self.current.after,
                scope,
            });
        }
    }
    fn start_resolve(&mut self, g: &Graph, variable: u64, phase: Phase) {
        self.resolve = Some(Resolve::new(g, self.root, variable, self.current.scope));
        self.representative = None;
        self.phase = phase;
    }
    /// Stop projecting immediately; discard at most one nested continuation
    /// step per call. Scalar stack/cursor/argument backing has no recursive
    /// payload destructors. Keep graph_root() traced until this token is dropped.
    pub fn discard_tick(&mut self) -> bool {
        if !self.discarding {
            self.discarding = true;
            self.stack = Vec::new();
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
        if let Some(job) = self.boolean.as_mut() {
            if job.discard_tick() {
                self.boolean = None;
            }
            return false;
        }
        if let Some(resolve) = self.resolve.as_mut() {
            if resolve.discard_tick() {
                self.resolve = None;
            }
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
            Phase::History => {
                let Some(frame) = self.stack.pop() else {
                    self.stack = Vec::new();
                    self.current.scope = Condition::FALSE;
                    self.phase = Phase::Done;
                    return ObserveStatus::Done;
                };
                self.current = frame;
                let next = self.last_choice.and_then(|last| {
                    births
                        .range((frame.after.map_or(Unbounded, Excluded), Included(last)))
                        .next()
                });
                if let Some((&id, birth)) = next {
                    self.current.after = Some(id);
                    self.birth_support = birth.support;
                    self.decision = birth.decision;
                    self.boolean = Some(a.start(Operation::Difference(frame.scope, birth.support)));
                    self.phase = Phase::Inactive;
                } else {
                    self.slot = 0;
                    self.relation = 0;
                    self.phase = Phase::Variables;
                    return ObserveStatus::Event(Output::Begin {
                        completion: self.completion,
                        alternative: self.alternative,
                    });
                }
            }
            Phase::Inactive => {
                if let Some(c) = self.poll(a) {
                    self.inactive = c;
                    self.boolean =
                        Some(a.start(Operation::And(self.current.scope, self.birth_support)));
                    self.phase = Phase::Active;
                }
            }
            Phase::Active => {
                if let Some(c) = self.poll(a) {
                    self.active = c;
                    self.boolean = Some(a.start(Operation::And(c, self.decision)));
                    self.phase = Phase::Left;
                }
            }
            Phase::Left => {
                if let Some(c) = self.poll(a) {
                    self.left = c;
                    self.boolean = Some(a.start(Operation::And(self.active, self.decision.not())));
                    self.phase = Phase::Right;
                }
            }
            Phase::Right => {
                if let Some(right) = self.poll(a) {
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
                if self.relation == self.code.signatures.len() {
                    self.alternative = self
                        .alternative
                        .checked_add(1)
                        .expect("alternative identity exhausted");
                    self.current.scope = Condition::FALSE;
                    self.phase = Phase::History;
                    return ObserveStatus::Event(Output::End);
                }
                self.rows = Some(
                    g.relation(self.root, self.relation)
                        .expect("prepared relation signature"),
                );
                self.phase = Phase::Rows;
            }
            Phase::Rows => {
                if let Some((occurrence, support)) =
                    self.rows.as_mut().expect("relation cursor").next(g)
                {
                    self.occurrence = occurrence;
                    self.boolean = Some(a.start(Operation::And(self.current.scope, support)));
                    self.phase = Phase::Live;
                } else {
                    self.rows = None;
                    self.relation += 1;
                    self.phase = Phase::Relations;
                }
            }
            Phase::Live => {
                if let Some(c) = self.poll(a) {
                    if c == Condition::FALSE {
                        self.phase = Phase::Rows;
                    } else {
                        self.arguments = Some(g.arguments(self.occurrence));
                        self.port = 0;
                        self.phase = Phase::Ports;
                        return ObserveStatus::Event(Output::Fact {
                            occurrence: self.occurrence,
                            relation: self.relation,
                        });
                    }
                }
            }
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
            _ => Step::Done,
        }
    }
}
