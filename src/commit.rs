//! Revalidate a candidate and stage its CHR application under one mutation owner.
//! The executor publishes the returned state and body obligation together.
use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::graph::{Graph, Update, UpdateStatus};
use crate::history::History;
use crate::identity::Equal;
use crate::matching::Match;
use crate::program::Prepared;
use crate::store::Root;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Default)]
pub struct FreshIds {
    variables: u64,
    events: u64,
}
impl FreshIds {
    pub fn variable(&mut self) -> u64 {
        let id = self.variables;
        self.variables = id.checked_add(1).expect("variable identities exhausted");
        id
    }
    pub fn event(&mut self) -> u64 {
        let id = self.events;
        self.events = id.checked_add(1).expect("event identities exhausted");
        id
    }
    pub fn variable_count(&self) -> u64 {
        self.variables
    }
    pub fn event_count(&self) -> u64 {
        self.events
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateRoot {
    pub graph: Root,
    pub history: Root,
}
pub struct Application {
    pub id: u64,
    pub rule: usize,
    pub body: usize,
    pub variables: Arc<Vec<u64>>,
    pub heads: Arc<Vec<u64>>,
    pub support: Condition,
}
pub struct Committed {
    pub state: StateRoot,
    pub application: Application,
}
pub enum CommitStatus {
    Pending,
    Rejected,
    Applied(Committed),
    Done,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CommitError {
    Root,
    Rule,
    Shape,
}
#[derive(Clone, Copy)]
enum Phase {
    Start,
    Head,
    Port,
    History,
    Record,
    Consume,
    Fresh,
    Done,
}
pub struct Commit {
    base: StateRoot,
    staged: StateRoot,
    code: Arc<Prepared>,
    rule: usize,
    heads: Arc<Vec<u64>>,
    variables: Vec<u64>,
    scope: Condition,
    active: Condition,
    phase: Phase,
    head: usize,
    port: usize,
    seen: HashSet<u64>,
    job: Option<Job>,
    equal: Option<Equal>,
    update: Option<Update>,
    discard: u8,
}
impl Commit {
    pub fn new(
        g: &Graph,
        h: &History,
        state: StateRoot,
        code: Arc<Prepared>,
        rule: usize,
        candidate: Match,
        active: Condition,
    ) -> Result<Self, CommitError> {
        if !g.index.contains(&state.graph) || !h.contains(state.history.clone()) {
            return Err(CommitError::Root);
        }
        let plan = code.rules.get(rule).ok_or(CommitError::Rule)?;
        if candidate.occurrences.len() != plan.heads.len()
            || candidate.bindings.len() != plan.head_variables
        {
            return Err(CommitError::Shape);
        }
        Ok(Self {
            discard: 0,
            base: state.clone(),
            staged: state,
            code,
            rule,
            heads: Arc::new(candidate.occurrences),
            variables: candidate.bindings,
            scope: candidate.support,
            active,
            phase: Phase::Start,
            head: 0,
            port: 0,
            seen: HashSet::new(),
            job: None,
            equal: None,
            update: None,
        })
    }
    pub fn graph_roots(&self) -> impl Iterator<Item = Root> + '_ {
        [self.base.graph.clone(), self.staged.graph.clone()]
            .into_iter()
            .chain(self.update.iter().flat_map(Update::roots))
    }
    pub fn history_roots(&self) -> [Root; 2] {
        [self.base.history.clone(), self.staged.history.clone()]
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.scope, self.active]
            .into_iter()
            .chain(self.job.iter().flat_map(Job::roots))
            .chain(self.equal.iter().flat_map(Equal::condition_roots))
    }
    pub fn variable_roots(&self) -> impl Iterator<Item = u64> + '_ {
        self.variables.iter().copied()
    }
    fn reject(&mut self) -> CommitStatus {
        self.phase = Phase::Done;
        CommitStatus::Rejected
    }
    fn boolean(&mut self, a: &mut Arena, op: Operation) -> Option<Condition> {
        let job = self.job.get_or_insert_with(|| a.start(op));
        match job.tick(a) {
            Progress::Pending => None,
            Progress::Complete(c) => {
                self.job = None;
                Some(c)
            }
        }
    }
    /// The caller must retain the mutation lane until this transaction finishes.
    /// Other readers may inspect its base root; no staged root is publishable.
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.scope = Condition::FALSE;
            self.active = Condition::FALSE;
            self.variables = Vec::new();
            self.heads = Arc::new(Vec::new());
            self.seen = HashSet::new();
            self.update = None;
        }
        match self.discard {
            1 => {
                if let Some(j) = &mut self.job {
                    if j.discard_tick() {
                        self.job = None;
                    }
                } else {
                    self.discard = 2;
                }
            }
            2 => {
                if let Some(e) = &mut self.equal {
                    if e.discard_tick() {
                        self.equal = None;
                    }
                } else {
                    self.discard = 3;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(
        &mut self,
        g: &mut Graph,
        a: &mut Arena,
        h: &mut History,
        ids: &mut FreshIds,
    ) -> CommitStatus {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        if matches!(self.phase, Phase::Done) {
            return CommitStatus::Done;
        }
        if self.scope == Condition::FALSE {
            return self.reject();
        }
        match self.phase {
            Phase::Start => {
                if let Some(c) = self.boolean(a, Operation::And(self.scope, self.active)) {
                    self.scope = c;
                    self.phase = Phase::Head;
                }
            }
            Phase::Head => {
                if self.head == self.heads.len() {
                    self.phase = Phase::History;
                } else {
                    let id = self.heads[self.head];
                    let Some(fact) = g.fact(self.base.graph.clone(), id) else {
                        return self.reject();
                    };
                    let head = &self.code.rules[self.rule].heads[self.head];
                    if fact.relation != head.relation || fact.args.len() != head.args.len() {
                        return self.reject();
                    }
                    if let Some(c) = self.boolean(a, Operation::And(self.scope, fact.support)) {
                        if !self.seen.insert(id) {
                            return self.reject();
                        }
                        self.scope = c;
                        self.port = 0;
                        self.phase = Phase::Port;
                    }
                }
            }
            Phase::Port => {
                let head = &self.code.rules[self.rule].heads[self.head];
                if self.port == head.args.len() {
                    self.head += 1;
                    self.phase = Phase::Head;
                } else {
                    let actual = g
                        .fact(self.base.graph.clone(), self.heads[self.head])
                        .expect("retained base")
                        .args[self.port];
                    let expected = self.variables[head.args[self.port]];
                    if actual == expected {
                        self.port += 1;
                    } else {
                        let equal = self.equal.get_or_insert_with(|| {
                            Equal::new(g, self.base.graph.clone(), actual, expected, self.scope)
                        });
                        if let Some(c) = equal.tick(g, a) {
                            self.scope = c;
                            self.equal = None;
                            self.port += 1;
                        }
                    }
                }
            }
            Phase::History => {
                let plan = &self.code.rules[self.rule];
                if plan.kept == plan.heads.len() {
                    let old = h.support(self.base.history.clone(), self.rule, &self.heads);
                    if let Some(c) = self.boolean(a, Operation::Difference(self.scope, old)) {
                        self.scope = c;
                        self.phase = Phase::Record;
                    }
                } else {
                    self.head = plan.kept;
                    self.phase = Phase::Consume;
                }
            }
            Phase::Record => {
                let old = h.support(self.base.history.clone(), self.rule, &self.heads);
                if let Some(c) = self.boolean(a, Operation::Or(old, self.scope)) {
                    self.staged.history =
                        h.set_support(self.base.history.clone(), self.rule, self.heads.clone(), c);
                    self.phase = Phase::Fresh;
                }
            }
            Phase::Consume => {
                if let Some(update) = &mut self.update {
                    if let UpdateStatus::Complete(root) = update.tick(g) {
                        self.staged.graph = root;
                        self.update = None;
                        self.head += 1;
                    }
                } else if self.head == self.heads.len() {
                    self.phase = Phase::Fresh;
                } else {
                    let id = self.heads[self.head];
                    let old = g
                        .fact(self.staged.graph.clone(), id)
                        .expect("verified distinct head")
                        .support;
                    if let Some(c) = self.boolean(a, Operation::Difference(old, self.scope)) {
                        self.update = Some(
                            g.set_liveness(self.staged.graph.clone(), id, c)
                                .expect("verified head"),
                        );
                    }
                }
            }
            Phase::Fresh => {
                let plan = &self.code.rules[self.rule];
                if self.variables.len() < plan.variables.len() {
                    self.variables.push(ids.variable());
                } else {
                    self.phase = Phase::Done;
                    return CommitStatus::Applied(Committed {
                        state: self.staged.clone(),
                        application: Application {
                            id: ids.event(),
                            rule: self.rule,
                            body: plan.body,
                            variables: Arc::new(std::mem::take(&mut self.variables)),
                            heads: self.heads.clone(),
                            support: self.scope,
                        },
                    });
                }
            }
            Phase::Done => unreachable!(),
        }
        CommitStatus::Pending
    }
}

impl Trace for Commit {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.scope, self.active]),
            1 => cursor.optional(self.job.as_ref()),
            2 => cursor.optional(self.equal.as_ref()),
            _ => Step::Done,
        }
    }
}
