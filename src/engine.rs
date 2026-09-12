//! Cooperative execution of supported bodies, indexed discovery and CHR commits.
mod cancel;
mod collection;
mod inspection;
mod obligations;
mod step;
pub use collection::Memory;
pub use inspection::{InspectionError, InspectionStatus, SnapshotInfo, SnapshotKind, ViewId};
pub use step::StepStatus;

use crate::commit::{Commit, CommitStatus, FreshIds, StateRoot};
use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::graph::{Graph, Update, UpdateStatus};
use crate::history::History;
use crate::identity::Merge;
use crate::matching::{Match, MatchStatus, Matches};
use crate::observe::{Observe, ObserveStatus, Output};
use crate::program::{Instruction, Prepared};
use crate::store::{Cursor, Root, Store};
use crate::wake::{Wake, WakeStatus};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

pub struct Birth {
    pub event: u64,
    pub instruction: usize,
    pub arm: usize,
    pub support: Condition,
    pub decision: Condition,
}
pub struct Completion {
    pub id: u64,
    pub support: Condition,
    pub state: StateRoot,
    pub last_choice: Option<u64>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    Task(u64),
    Completion,
    Collection,
}
struct Scheduled {
    id: u64,
    scope: Condition,
    task: Task,
}
enum Task {
    Init(Vec<u64>),
    Body(Box<Body>),
    Activate {
        root: Root,
        occurrence: u64,
        next: usize,
    },
    Search(Box<Search>),
    Wake(Box<Wake>),
}
struct Search {
    rule: usize,
    matches: Matches,
    candidate: Option<Match>,
    commit: Option<Commit>,
}
#[derive(Clone, Copy)]
enum BodyPhase {
    Dispatch,
    Arguments,
    Expand,
    Acquire,
    Filter,
    Apply,
    Post,
    Merge,
    Fail,
    Failed,
    Choice,
    Left,
    Right,
}
struct Body {
    event: u64,
    instruction: usize,
    variables: Arc<Vec<u64>>,
    scope: Condition,
    phase: BodyPhase,
    index: usize,
    args: Vec<u64>,
    job: Option<Job>,
    update: Option<Update>,
    merge: Option<Merge>,
    decision: Condition,
    pending_active: Condition,
}
impl Body {
    fn new(event: u64, instruction: usize, variables: Arc<Vec<u64>>, scope: Condition) -> Self {
        Self {
            event,
            instruction,
            variables,
            scope,
            phase: BodyPhase::Dispatch,
            index: 0,
            args: Vec::new(),
            job: None,
            update: None,
            merge: None,
            decision: Condition::FALSE,
            pending_active: Condition::FALSE,
        }
    }
}
#[derive(Clone, Copy)]
enum ReadyPhase {
    Scan,
    Difference,
    Acquire,
    Filter,
    Publish,
}
struct Ready {
    cursor: Cursor,
    blocked: Condition,
    scope: Condition,
    job: Option<Job>,
    phase: ReadyPhase,
}

pub struct Engine {
    code: Arc<Prepared>,
    graph: Graph,
    arena: Arena,
    history: History,
    state: StateRoot,
    ids: FreshIds,
    variables: Arc<Vec<u64>>,
    active: Condition,
    failed: Condition,
    queue: VecDeque<Scheduled>,
    parked: BTreeMap<u64, Scheduled>,
    next_task: u64,
    lane: Option<Owner>,
    waiting: VecDeque<Owner>,
    requested: BTreeSet<Owner>,
    pending_root: Root,
    obligations: obligations::Obligations,
    ready: Option<Ready>,
    output: Option<Output>,
    observer: Option<Observe>,
    births: BTreeMap<u64, Birth>,
    applications: u64,
    ticks: u64,
    collector: Option<collection::Collection>,
    collection_requested: bool,
    collections: u64,
    collection_limit: usize,
    record_history: bool,
    snapshots: BTreeMap<u64, inspection::Snapshot>,
    inspections: BTreeMap<u64, inspection::Inspection>,
    last_inspection: Option<u64>,
    inspection_round: Option<u64>,
    latest_inspection: Option<u64>,
    cancellation: cancel::Cancellation,
    rule_step: Option<step::RuleStep>,
}
impl Engine {
    pub fn new(code: Arc<Prepared>) -> Self {
        Self::with_history(code, false)
    }
    pub fn with_history(code: Arc<Prepared>, record_history: bool) -> Self {
        let graph = Graph::new(&code.signatures);
        let history = History::default();
        let state = StateRoot {
            graph: graph.empty(),
            history: history.empty(),
        };
        let obligations = obligations::Obligations::default();
        let pending_root = obligations.empty();
        let mut e = Self {
            code,
            graph,
            history,
            state,
            pending_root,
            obligations,
            arena: Arena::default(),
            ids: FreshIds::default(),
            variables: Arc::new(vec![]),
            active: Condition::TRUE,
            failed: Condition::FALSE,
            queue: VecDeque::new(),
            parked: BTreeMap::new(),
            next_task: 0,
            lane: None,
            waiting: VecDeque::new(),
            requested: BTreeSet::new(),
            ready: None,
            output: None,
            observer: None,
            births: BTreeMap::new(),
            applications: 0,
            ticks: 0,
            collector: None,
            collection_requested: false,
            collections: 0,
            collection_limit: 4096,
            record_history,
            snapshots: BTreeMap::new(),
            inspections: BTreeMap::new(),
            last_inspection: None,
            inspection_round: None,
            latest_inspection: None,
            cancellation: cancel::Cancellation::default(),
            rule_step: None,
        };
        e.spawn(Condition::TRUE, Task::Init(vec![]));
        e
    }
    pub fn program(&self) -> &Prepared {
        &self.code
    }
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
    pub fn arena(&self) -> &Arena {
        &self.arena
    }
    pub fn state(&self) -> StateRoot {
        self.state
    }
    pub fn query_variables(&self) -> &[u64] {
        &self.variables
    }
    pub fn choices(&self) -> impl DoubleEndedIterator<Item = (&u64, &Birth)> {
        self.births.iter()
    }
    pub fn applications(&self) -> u64 {
        self.applications
    }
    pub fn failed(&self) -> Condition {
        self.failed
    }
    pub fn exhausted(&self) -> bool {
        self.active == Condition::FALSE
    }
    pub fn pending_tasks(&self) -> usize {
        self.queue.len() + self.parked.len()
    }
    /// Output is a stream of owned scalar events; taking an event retains no
    /// execution snapshot. The receiver builds or stores the requested graph.
    pub fn take_output(&mut self) -> Option<Output> {
        self.output.take()
    }
    pub fn delivery_done(&self) -> bool {
        self.exhausted() && self.observer.is_none() && self.output.is_none()
    }
    fn spawn(&mut self, scope: Condition, task: Task) {
        if scope == Condition::FALSE {
            return;
        }
        let id = self.next_task;
        self.next_task = id.checked_add(1).expect("task identity exhausted");
        let key = [id, 0, 0, 0];
        let pending = self.pending_task(scope, &task);
        self.pending_root = self
            .obligations
            .index
            .insert(self.pending_root, key, pending);
        self.queue.push_back(Scheduled { id, scope, task });
    }
    fn body(&mut self, event: u64, instruction: usize, variables: Arc<Vec<u64>>, scope: Condition) {
        self.spawn(
            scope,
            Task::Body(Box::new(Body::new(event, instruction, variables, scope))),
        );
    }
    fn acquire(&mut self, owner: Owner) -> bool {
        if self.lane == Some(owner) {
            return true;
        }
        if self.requested.insert(owner) {
            self.waiting.push_back(owner);
        }
        if self.lane.is_none() && self.waiting.front() == Some(&owner) {
            self.waiting.pop_front();
            self.requested.remove(&owner);
            self.lane = Some(owner);
            return true;
        }
        false
    }
    fn release_lane(&mut self) {
        self.lane = self.waiting.pop_front();
        if let Some(owner) = self.lane {
            self.requested.remove(&owner);
            if let Owner::Task(id) = owner {
                self.queue
                    .push_back(self.parked.remove(&id).expect("waiting task is suspended"));
            }
        }
    }
    /// A budget counts finite continuation steps, not source answers. Zero is a no-op.
    pub fn advance(&mut self, budget: usize) {
        for _ in 0..budget {
            if self.collect_heap() {
                continue;
            }
            if !self.canceled() {
                match self.step_gate() {
                    step::Gate::Run => {}
                    step::Gate::Yield => continue,
                    step::Gate::Stop => break,
                }
            }
            if self.cancellation.requested {
                self.cancel_tick();
            } else if !self.inspections.is_empty() && self.ticks % 4 == 3 {
                self.service_inspection();
            } else if self.ticks.is_multiple_of(3) {
                if let Some(mut task) = self.queue.pop_front() {
                    if self.task(&mut task) {
                        self.pending_root = self
                            .obligations
                            .index
                            .remove(self.pending_root, &[task.id, 0, 0, 0]);
                        if self.lane == Some(Owner::Task(task.id)) {
                            self.release_lane();
                        }
                    } else if self.requested.contains(&Owner::Task(task.id)) {
                        self.parked.insert(task.id, task);
                    } else {
                        self.queue.push_back(task);
                    }
                }
            } else if self.ticks % 3 == 1 {
                self.completion();
            } else {
                self.observation();
            }
            self.ticks = self.ticks.wrapping_add(1);
        }
    }
    fn task(&mut self, s: &mut Scheduled) -> bool {
        match &mut s.task {
            Task::Init(vars) => {
                if vars.len() < self.code.query_variables.len() {
                    vars.push(self.ids.variable());
                    false
                } else {
                    self.variables = Arc::new(std::mem::take(vars));
                    let event = self.ids.event();
                    self.body(event, self.code.query, self.variables.clone(), s.scope);
                    self.record(SnapshotKind::Initial, self.active);
                    true
                }
            }
            Task::Body(b) => {
                let done = self.body_tick(s.id, b);
                if !done {
                    self.sync_obligation(s.id, b);
                }
                done
            }
            Task::Activate {
                root,
                occurrence,
                next,
            } => {
                let Some(fact) = self.graph.fact(*root, *occurrence) else {
                    return true;
                };
                let triggers = &self.code.triggers[fact.relation];
                if *next == triggers.len() {
                    true
                } else {
                    let (rule, head) = triggers[*next];
                    *next += 1;
                    let matches = Matches::new(
                        &self.graph,
                        *root,
                        self.code.clone(),
                        rule,
                        s.scope,
                        Some((head, *occurrence)),
                    )
                    .expect("prepared trigger");
                    self.spawn(
                        s.scope,
                        Task::Search(Box::new(Search {
                            rule,
                            matches,
                            candidate: None,
                            commit: None,
                        })),
                    );
                    false
                }
            }
            Task::Search(search) => {
                if let Some(commit) = &mut search.commit {
                    match commit.tick(
                        &mut self.graph,
                        &mut self.arena,
                        &mut self.history,
                        &mut self.ids,
                    ) {
                        CommitStatus::Applied(c) => {
                            self.state = c.state;
                            self.applications += 1;
                            let app = c.application;
                            self.step_application(app.id, search.rule, app.support);
                            self.body(app.id, app.body, app.variables, app.support);
                            self.record(
                                SnapshotKind::Application {
                                    rule: search.rule,
                                    event: app.id,
                                },
                                self.active,
                            );
                            search.commit = None;
                            self.release_lane();
                        }
                        CommitStatus::Rejected => {
                            search.commit = None;
                            self.release_lane();
                        }
                        CommitStatus::Pending => {}
                        CommitStatus::Done => unreachable!(),
                    }
                    false
                } else if search.candidate.is_some() {
                    if self.acquire(Owner::Task(s.id)) {
                        search.commit = Some(
                            Commit::new(
                                &self.graph,
                                &self.history,
                                self.state,
                                self.code.clone(),
                                search.rule,
                                search.candidate.take().unwrap(),
                                self.active,
                            )
                            .expect("internal candidate"),
                        );
                    }
                    false
                } else {
                    match search.matches.tick(&self.graph, &mut self.arena) {
                        MatchStatus::Done => true,
                        MatchStatus::Pending => false,
                        MatchStatus::Found(candidate) => {
                            search.candidate = Some(candidate);
                            false
                        }
                    }
                }
            }
            Task::Wake(w) => match w.tick(&self.graph, &mut self.arena) {
                WakeStatus::Done => true,
                WakeStatus::Pending => false,
                WakeStatus::Found {
                    occurrence,
                    support,
                } => {
                    self.spawn(
                        support,
                        Task::Activate {
                            root: w.root(),
                            occurrence,
                            next: 0,
                        },
                    );
                    false
                }
            },
        }
    }
    fn body_tick(&mut self, id: u64, b: &mut Body) -> bool {
        if b.scope == Condition::FALSE {
            return true;
        }
        match b.phase {
            BodyPhase::Dispatch => {
                b.phase = match &self.code.instructions[b.instruction] {
                    Instruction::True => return true,
                    Instruction::And(_) => BodyPhase::Expand,
                    Instruction::Post(_) => BodyPhase::Arguments,
                    _ => BodyPhase::Acquire,
                };
            }
            BodyPhase::Arguments => {
                let Instruction::Post(atom) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                if b.index == atom.args.len() {
                    b.phase = BodyPhase::Acquire;
                } else {
                    b.args.push(b.variables[atom.args[b.index]]);
                    b.index += 1;
                }
            }
            BodyPhase::Expand => {
                let Instruction::And(items) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                if b.index == items.len() {
                    return true;
                }
                let instruction = items[b.index];
                b.index += 1;
                self.body(b.event, instruction, b.variables.clone(), b.scope);
            }
            BodyPhase::Acquire => {
                if self.acquire(Owner::Task(id)) {
                    b.phase = BodyPhase::Filter;
                    b.job = Some(self.arena.start(Operation::And(b.scope, self.active)));
                }
            }
            BodyPhase::Filter => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    b.scope = c;
                    b.phase = BodyPhase::Apply;
                }
            }
            BodyPhase::Apply => match &self.code.instructions[b.instruction] {
                Instruction::Post(atom) => {
                    b.update = Some(
                        self.graph
                            .post(
                                self.state.graph,
                                atom.relation,
                                std::mem::take(&mut b.args),
                                b.scope,
                            )
                            .expect("prepared post"),
                    );
                    b.phase = BodyPhase::Post;
                }
                Instruction::Equal(x, y) => {
                    b.merge = Some(Merge::new(
                        &self.graph,
                        self.state.graph,
                        b.variables[*x],
                        b.variables[*y],
                        b.scope,
                    ));
                    b.phase = BodyPhase::Merge;
                }
                Instruction::Fail => {
                    b.job = Some(
                        self.arena
                            .start(Operation::Difference(self.active, b.scope)),
                    );
                    b.phase = BodyPhase::Fail;
                }
                Instruction::Or(_) => {
                    b.index = 0;
                    b.phase = BodyPhase::Choice;
                }
                _ => unreachable!(),
            },
            BodyPhase::Post => {
                let update = b.update.as_mut().unwrap();
                if let UpdateStatus::Complete(root) = update.tick(&mut self.graph) {
                    self.state.graph = root;
                    let occurrence = update.occurrence();
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Post { occurrence }, self.active);
                    self.spawn(
                        b.scope,
                        Task::Activate {
                            root,
                            occurrence,
                            next: 0,
                        },
                    );
                    return true;
                }
            }
            BodyPhase::Merge => {
                let merge = b.merge.as_mut().unwrap();
                if let Some(root) = merge.tick(&mut self.graph, &mut self.arena) {
                    self.state.graph = root;
                    let scope = merge.changed_support();
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Merge, self.active);
                    let Instruction::Equal(x, _) = self.code.instructions[b.instruction] else {
                        unreachable!()
                    };
                    if scope != Condition::FALSE {
                        self.spawn(
                            scope,
                            Task::Wake(Box::new(Wake::new(
                                &self.graph,
                                root,
                                b.variables[x],
                                scope,
                            ))),
                        );
                    }
                    return true;
                }
            }
            BodyPhase::Fail => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    b.pending_active = c;
                    b.job = Some(self.arena.start(Operation::Or(self.failed, b.scope)));
                    b.phase = BodyPhase::Failed;
                }
            }
            BodyPhase::Failed => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    self.active = b.pending_active;
                    self.failed = c;
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Failure, self.active);
                    return true;
                }
            }
            BodyPhase::Choice => {
                let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                if b.index + 1 == items.len() {
                    let instruction = items[b.index];
                    self.body(b.event, instruction, b.variables.clone(), b.scope);
                    return true;
                }
                let (choice, decision) = self.arena.fresh_choice();
                b.decision = decision;
                self.births.insert(
                    choice,
                    Birth {
                        event: b.event,
                        instruction: b.instruction,
                        arm: b.index,
                        support: b.scope,
                        decision,
                    },
                );
                b.job = Some(self.arena.start(Operation::And(b.scope, decision)));
                b.phase = BodyPhase::Left;
                self.sync_obligation(id, b);
                self.record(SnapshotKind::Choice, self.active);
            }
            BodyPhase::Left => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                        unreachable!()
                    };
                    let instruction = items[b.index];
                    self.body(b.event, instruction, b.variables.clone(), c);
                    b.job = Some(self.arena.start(Operation::And(b.scope, b.decision.not())));
                    b.phase = BodyPhase::Right;
                }
            }
            BodyPhase::Right => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    b.scope = c;
                    b.index += 1;
                    b.phase = BodyPhase::Choice;
                }
            }
        }
        false
    }
    fn observation(&mut self) {
        if self.output.is_some() {
            return;
        }
        if let Some(observer) = &mut self.observer {
            match observer.tick(&self.graph, &mut self.arena, &self.births) {
                ObserveStatus::Pending => {}
                ObserveStatus::Event(event) => self.output = Some(event),
                ObserveStatus::Done => self.observer = None,
            }
        }
    }
    // A frozen pending root is a conservative completion certificate: every new
    // obligation is born within its parent's recorded scope, and parent removal
    // and child admission happen in one service step. Thus no descendant can
    // enter the complement of this snapshot's union. Failure only shrinks active
    // scope. Checking current active scope in the mutation lane finishes the
    // certificate without invalidation by unrelated ongoing updates.
    fn completion(&mut self) {
        if self.observer.is_some()
            || self.output.is_some()
            || (self.active == Condition::FALSE && self.ready.is_none())
        {
            return;
        }
        let mut ready = self.ready.take().unwrap_or_else(|| Ready {
            cursor: self
                .obligations
                .index
                .range(self.pending_root, [0; 4], [u64::MAX; 4]),
            blocked: Condition::FALSE,
            scope: self.active,
            job: None,
            phase: ReadyPhase::Scan,
        });
        match ready.phase {
            ReadyPhase::Scan => {
                if ready.job.is_some() {
                    if let Some(c) = poll(&mut ready.job, &mut self.arena) {
                        // Once the captured scope is fully blocked, no later
                        // row can produce a completion from this certificate.
                        // Release its frozen ownership root and recapture next
                        // time, so completed bodies do not remain pinned here.
                        if c == Condition::TRUE || c == ready.scope {
                            return;
                        }
                        ready.blocked = c;
                    }
                } else if let Some((_, c)) = ready.cursor.next(&self.obligations.index) {
                    ready.job = Some(self.arena.start(Operation::Or(ready.blocked, c.scope)));
                } else {
                    ready.job = Some(
                        self.arena
                            .start(Operation::Difference(ready.scope, ready.blocked)),
                    );
                    ready.phase = ReadyPhase::Difference;
                }
            }
            ReadyPhase::Difference => {
                if let Some(c) = poll(&mut ready.job, &mut self.arena) {
                    if c == Condition::FALSE {
                        return;
                    }
                    ready.scope = c;
                    ready.phase = ReadyPhase::Acquire;
                }
            }
            ReadyPhase::Acquire => {
                if self.acquire(Owner::Completion) {
                    ready.job = Some(self.arena.start(Operation::And(ready.scope, self.active)));
                    ready.phase = ReadyPhase::Filter;
                }
            }
            ReadyPhase::Filter => {
                if let Some(c) = poll(&mut ready.job, &mut self.arena) {
                    if c == Condition::FALSE {
                        self.release_lane();
                        return;
                    }
                    ready.scope = c;
                    ready.job = Some(self.arena.start(Operation::Difference(self.active, c)));
                    ready.phase = ReadyPhase::Publish;
                }
            }
            ReadyPhase::Publish => {
                if let Some(c) = poll(&mut ready.job, &mut self.arena) {
                    self.record(SnapshotKind::NormalForm, ready.scope);
                    self.active = c;
                    self.observer = Some(Observe::new(
                        Completion {
                            id: self.ids.event(),
                            support: ready.scope,
                            state: self.state,
                            last_choice: self.births.last_key_value().map(|(&id, _)| id),
                        },
                        self.code.clone(),
                        self.variables.clone(),
                    ));
                    self.release_lane();
                    return;
                }
            }
        }
        self.ready = Some(ready);
    }
}
fn poll(job: &mut Option<Job>, a: &mut Arena) -> Option<Condition> {
    match job.as_mut().expect("pending Boolean operation").tick(a) {
        Progress::Pending => None,
        Progress::Complete(c) => {
            *job = None;
            Some(c)
        }
    }
}
