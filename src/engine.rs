//! Cooperative execution of supported bodies, indexed discovery and CHR commits.
// Measurement expressions disappear without diagnostics. Sample work before
// releasing a completed job, including its final nonzero work step.
macro_rules! measured_tick {
    ($job:expr, $arena:expr, $counter:expr) => {{
        let job = $job;
        #[cfg(feature = "diagnostics")]
        let before = job.work();
        let result = job.tick($arena);
        #[cfg(feature = "diagnostics")]
        {
            $counter.calls += 1;
            $counter.work += job.work() - before;
        }
        result
    }};
}
macro_rules! measured_poll {
    ($slot:expr, $arena:expr, $counter:expr) => {{
        let slot = $slot;
        match measured_tick!(
            slot.as_mut().expect("pending conditional operation"),
            $arena,
            $counter
        ) {
            Progress::Pending => None,
            Progress::Complete(value) => {
                *slot = None;
                Some(value)
            }
        }
    }};
}
mod cancel;
mod collection;
mod compact;
mod coordinates;
#[cfg(feature = "diagnostics")]
pub mod diagnostics;
#[cfg(feature = "diagnostics")]
pub use diagnostics::Diagnostics;
mod discovery;
mod dispatch;
mod normalization;
mod producer;
pub use normalization::NormalizationStats;
use normalization::Normalizer;
mod inspection;
mod obligations;
mod restriction;
mod step;
pub use collection::Memory;
pub use inspection::{InspectionError, InspectionStatus, SnapshotInfo, SnapshotKind, ViewId};
pub use step::StepStatus;

use crate::commit::{Commit, CommitStatus, FreshIds, StateRoot};
use crate::condition::{Arena, Condition, Job, Operation, Progress, poll};
use crate::graph::{Graph, Update, UpdateStatus};
use crate::history::History;
use crate::identity::Merge;
use crate::matching::{Match, MatchStatus, Matches};
use crate::observe::{Observe, ObserveStatus, Output};
use crate::program::{Instruction, Prepared};
use crate::store::{Root, Store};
type PendingRoot = crate::store::Root<obligations::Pending>;
type Cursor = crate::store::Cursor<obligations::Pending>;
use crate::wake::{Wake, WakeStatus};
use coordinates::{Coordinates, Epoch, Transport};
use discovery::Discovery;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

pub struct Birth {
    pub event: u64,
    pub instruction: usize,
    /// Original flat disjunction: true selects [start, split), false [split, end).
    pub start: usize,
    pub split: usize,
    pub end: usize,
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
    epoch: Option<Epoch>,
    id: u64,
    scope: Condition,
    task: Task,
}
enum Task {
    Init(Vec<u64>),
    Body(Box<Body>),
    Activate {
        identity_only: bool,
        root: Option<Root>,
        relation: usize,
        arguments: Arc<Vec<u64>>,
        occurrence: u64,
        reader_scope: Condition,
        next: usize,
    },
    Search(Box<Search>),
    Wake(Box<Wake>),
}
struct Search {
    transport: Option<Transport>,
    rule: usize,
    matches: Discovery,
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
    Choice,
    Guard,
    GuardLeft,
    GuardRight,
    Left,
    Right,
}
struct Body {
    normalizer: Option<Box<Normalizer>>,
    dispatch: Option<Box<dispatch::Dispatch>>,
    dispatch_checked: bool,
    rejection: Option<Box<producer::Split>>,
    split_scopes: [Condition; 2],
    source_complete: bool,
    event: u64,
    instruction: usize,
    variables: Arc<Vec<u64>>,
    scope: Condition,
    phase: BodyPhase,
    index: usize,
    // Only disjunction continuations restrict the original instruction range.
    end: Option<usize>,
    args: Vec<u64>,
    job: Option<Job>,
    update: Option<Update>,
    merge: Option<Merge>,
    decision: Condition,
}
impl Body {
    fn new(event: u64, instruction: usize, variables: Arc<Vec<u64>>, scope: Condition) -> Self {
        Self {
            normalizer: None,
            dispatch: None,
            dispatch_checked: false,
            rejection: None,
            split_scopes: [Condition::FALSE; 2],
            source_complete: false,
            event,
            instruction,
            variables,
            scope,
            phase: BodyPhase::Dispatch,
            index: 0,
            end: None,
            args: Vec::new(),
            job: None,
            update: None,
            merge: None,
            decision: Condition::FALSE,
        }
    }
}
#[derive(Clone, Copy)]
enum ReadyPhase {
    Scan,
    Difference,
    Acquire,
    Transport,
    Filter,
    Publish,
}
struct Ready {
    epoch: Epoch,
    transport: Option<Transport>,
    cursor: Cursor,
    blocked: Condition,
    scope: Condition,
    job: Option<Job>,
    phase: ReadyPhase,
}

pub struct Engine {
    normalization: Option<normalization::Configuration>,
    normalization_stats: NormalizationStats,
    #[cfg(feature = "diagnostics")]
    diagnostics: Diagnostics,
    coordinates: Coordinates,
    code: Arc<Prepared>,
    graph: Graph,
    arena: Arena,
    history: History,
    state: StateRoot,
    ids: FreshIds,
    variables: Arc<Vec<u64>>,
    active: Condition,
    queue: VecDeque<Scheduled>,
    parked: BTreeMap<u64, Scheduled>,
    next_task: u64,
    lane: Option<Owner>,
    waiting: VecDeque<Owner>,
    requested: BTreeSet<Owner>,
    pending_root: PendingRoot,
    release_turn: usize,
    obligations: obligations::Obligations,
    ready: Option<Ready>,
    output: Option<Output>,
    observer: Option<Observe>,
    births: BTreeMap<u64, Birth>,
    applications: u64,
    ticks: u64,
    // Keep the large collector stationary across take/resume transitions.
    collector: Option<Box<collection::Collection>>,
    collection_requested: bool,
    collections: u64,
    collection_limit: usize,
    archive: collection::Archive,
    semantic_regions: bool,
    collection_yield: bool,
    record_history: bool,
    snapshots: BTreeMap<u64, inspection::Snapshot>,
    inspections: BTreeMap<u64, inspection::Inspection>,
    restrictions: restriction::Restrictions,
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
        let normalization = code
            .constructors
            .clone()
            .map(|plan| normalization::Configuration {
                plan,
                source: code.clone(),
            });
        let graph = Graph::with_update_plans(
            &code.signatures,
            &code.tuple_indexes,
            code.graph_updates.clone(),
        );
        let history = History::default();
        let state = StateRoot {
            graph: graph.empty(),
            history: history.empty(),
        };
        let obligations = obligations::Obligations::default();
        let pending_root = obligations.empty();
        let mut e = Self {
            normalization,
            normalization_stats: NormalizationStats::default(),
            #[cfg(feature = "diagnostics")]
            diagnostics: Diagnostics {
                rules: vec![diagnostics::RuleDiagnostics::default(); code.rules.len()],
                ..Diagnostics::default()
            },
            coordinates: Coordinates::default(),
            code,
            graph,
            history,
            state,
            pending_root,
            release_turn: 0,
            obligations,
            arena: Arena::default(),
            ids: FreshIds::default(),
            variables: Arc::new(vec![]),
            active: Condition::TRUE,
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
            archive: collection::Archive::default(),
            semantic_regions: false,
            collection_yield: false,
            record_history,
            snapshots: BTreeMap::new(),
            inspections: BTreeMap::new(),
            restrictions: restriction::Restrictions::default(),
            last_inspection: None,
            inspection_round: None,
            latest_inspection: None,
            cancellation: cancel::Cancellation::default(),
            rule_step: None,
        };
        e.spawn(Condition::TRUE, Task::Init(vec![]));
        e
    }
    #[inline]
    fn release_store_tick(&mut self) -> bool {
        if !self.release_pending() {
            return false;
        }
        self.release_store_tick_ready()
    }
    #[inline(never)]
    fn release_store_tick_ready(&mut self) -> bool {
        for _ in 0..3 {
            let turn = self.release_turn;
            self.release_turn = (turn + 1) % 3;
            let pending = match turn {
                0 => self.graph.index.release_pending(),
                1 => self.history.index.release_pending(),
                _ => self.obligations.index.release_pending(),
            };
            if pending {
                match turn {
                    0 => {
                        self.graph.index.release_tick();
                    }
                    1 => {
                        self.history.index.release_tick();
                    }
                    _ => {
                        self.obligations.index.release_tick();
                    }
                }
                return true;
            }
        }
        false
    }
    pub fn release_pending(&self) -> bool {
        self.graph.index.release_pending()
            || self.history.index.release_pending()
            || self.obligations.index.release_pending()
    }
    pub fn mutation_counts(&self) -> [(usize, usize); 3] {
        [
            self.graph.index.mutation_counts(),
            self.history.index.mutation_counts(),
            self.obligations.index.mutation_counts(),
        ]
    }
    pub fn study_status(&self) -> (u64, bool, Option<u8>, bool, usize) {
        (
            self.ticks,
            self.release_pending(),
            self.ready.as_ref().map(|r| r.phase as u8),
            self.semantic_regions,
            self.graph.semantic_debt(),
        )
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
    /// Read current occurrences while borrowing the execution. Use
    /// `capture_snapshot` for a view retained across execution or cancellation.
    ///
    /// ```compile_fail
    /// fn inspect(engine: &mut chr::engine::Engine) {
    ///     let mut facts = engine.facts(0).unwrap();
    ///     engine.cancel();
    ///     let _ = facts.next();
    /// }
    /// ```
    pub fn facts(
        &self,
        relation: usize,
    ) -> Result<impl Iterator<Item = crate::graph::Fact<'_>> + '_, crate::graph::GraphError> {
        let root = self.state.graph.clone();
        let mut rows = self.graph.relation(root.clone(), relation)?;
        Ok(std::iter::from_fn(move || {
            rows.next(&self.graph).map(|(id, _)| {
                self.graph
                    .fact(root.clone(), id)
                    .expect("current occurrence")
            })
        }))
    }
    pub fn query_variables(&self) -> &[u64] {
        &self.variables
    }
    pub fn choices(&self) -> impl DoubleEndedIterator<Item = (&u64, &Birth)> {
        self.births.iter()
    }
    /// Lifetime aggregate work counts; available only with the diagnostics feature.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
    /// Shared restriction work is charged separately from matcher candidate visits.
    #[cfg(feature = "diagnostics")]
    pub fn restriction_diagnostics(&self) -> crate::graph::RestrictionDiagnostics {
        self.graph.restriction_diagnostics()
    }
    pub fn applications(&self) -> u64 {
        self.applications
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
        self.spawn_in_epoch(scope, task, self.coordinates.current());
    }
    // Pending support is current; immutable reader inputs retain their epoch.
    fn spawn_in_epoch(&mut self, scope: Condition, task: Task, epoch: Epoch) {
        if scope == Condition::FALSE {
            return;
        }
        #[cfg(feature = "diagnostics")]
        {
            self.diagnostics.tasks_created += 1;
            if let Task::Search(search) = &task {
                self.diagnostics.rules[search.rule].tasks_created += 1;
            }
        }
        let id = self.next_task;
        self.next_task = id.checked_add(1).expect("task identity exhausted");
        let key = [id, 0, 0, 0];
        let pending = self.pending_task(scope, &task);
        self.pending_root =
            self.obligations
                .index
                .insert(std::mem::take(&mut self.pending_root), key, pending);
        let epoch = (!matches!(&task, Task::Body(_) | Task::Init(_))).then_some(epoch);
        self.queue.push_back(Scheduled {
            id,
            scope,
            task,
            epoch,
        });
    }
    fn body(&mut self, event: u64, instruction: usize, variables: Arc<Vec<u64>>, scope: Condition) {
        self.spawn(
            scope,
            Task::Body(Box::new(Body::new(event, instruction, variables, scope))),
        );
    }
    fn body_range(&mut self, b: &Body, start: usize, end: usize, scope: Condition) {
        let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
            unreachable!()
        };
        assert!(start < end && end <= items.len());
        if end - start == 1 {
            self.body(b.event, items[start], b.variables.clone(), scope);
        } else {
            let mut child = Body::new(b.event, b.instruction, b.variables.clone(), scope);
            child.index = start;
            child.end = Some(end);
            self.spawn(scope, Task::Body(Box::new(child)));
        }
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
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.task_wakes += 1;
                }
                self.queue
                    .push_back(self.parked.remove(&id).expect("waiting task is suspended"));
            }
        }
    }
    /// A budget counts finite continuation steps, not source answers. Zero is a no-op.
    pub fn advance(&mut self, budget: usize) {
        for _ in 0..budget {
            #[cfg(feature = "diagnostics")]
            {
                self.diagnostics.advance_iterations += 1;
            }
            if self.release_store_tick() {
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.store_release += 1;
                }
                continue;
            }
            self.cleanup_coordinates();
            let service_due =
                self.collector.is_none() && std::mem::take(&mut self.collection_yield);
            if !service_due && self.collect_heap() {
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.collection += 1;
                }
                continue;
            }
            if !self.canceled() {
                match self.step_gate() {
                    step::Gate::Run => {}
                    step::Gate::Yield => {
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.dispatch.step_gate += 1;
                        }
                        continue;
                    }
                    step::Gate::Stop => {
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.dispatch.step_gate += 1;
                        }
                        break;
                    }
                }
            }
            if self.cancellation.requested {
                #[cfg(feature = "diagnostics")]
                let tasks_before = self.queue.len() + self.parked.len();
                self.cancel_tick();
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.cancellation += 1;
                    self.diagnostics.tasks_canceled +=
                        (tasks_before - self.queue.len() - self.parked.len()) as u64;
                }
            } else if !self.inspections.is_empty() && self.ticks % 4 == 3 {
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.inspection += 1;
                }
                self.service_inspection();
            } else if self.ticks % 3 == 1 && self.can_complete() {
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.ready += 1;
                }
                self.completion();
            } else if self.ticks % 3 == 2 && self.observer.is_some() && self.output.is_none() {
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.dispatch.observer += 1;
                }
                self.observation();
            } else {
                // Keep each runnable class's reserved share. Fill other slots
                // with source work first, then completion or observation.
                if let Some(mut task) = self.queue.pop_front() {
                    debug_assert!(!self.requested.contains(&Owner::Task(task.id)));
                    #[cfg(feature = "diagnostics")]
                    {
                        let d = &mut self.diagnostics.dispatch;
                        let count = if task.scope == Condition::FALSE
                            && self.lane != Some(Owner::Task(task.id))
                        {
                            &mut d.discard
                        } else {
                            match &task.task {
                                Task::Init(_) => &mut d.init,
                                Task::Body(_) => &mut d.body,
                                Task::Activate { .. } => &mut d.activation,
                                Task::Search(_) => &mut d.search,
                                Task::Wake(_) => &mut d.wake,
                            }
                        };
                        *count += 1;
                    }
                    let done = self.task(&mut task);
                    // A runnable task can only append its own request during this tick.
                    // Handoff removes the request before requeuing a parked task.
                    debug_assert_eq!(
                        self.requested.contains(&Owner::Task(task.id)),
                        self.waiting.back() == Some(&Owner::Task(task.id)),
                    );
                    if done {
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.tasks_completed += 1;
                        }
                        self.pending_root = self
                            .obligations
                            .index
                            .remove(std::mem::take(&mut self.pending_root), &[task.id, 0, 0, 0]);
                        if self.lane == Some(Owner::Task(task.id)) {
                            self.release_lane();
                        }
                    } else if self.waiting.back() == Some(&Owner::Task(task.id)) {
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.task_parks += 1;
                        }
                        self.parked.insert(task.id, task);
                    } else {
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.task_requeues += 1;
                        }
                        self.queue.push_back(task);
                    }
                } else if self.can_complete() {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.dispatch.ready += 1;
                    }
                    self.completion();
                } else if self.observer.is_some() && self.output.is_none() {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.dispatch.observer += 1;
                    }
                    self.observation();
                } else {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.dispatch.idle += 1;
                    }
                }
            }
            self.ticks = self.ticks.wrapping_add(1);
        }
    }
    fn activation(
        &self,
        root: Root,
        occurrence: u64,
        reader_scope: Condition,
        identity_only: bool,
    ) -> Task {
        let fact = self
            .graph
            .fact(root.clone(), occurrence)
            .expect("activation occurrence");
        let relation = fact.relation;
        let end = if identity_only {
            self.code.merge_indexed_end[relation]
        } else {
            self.code.indexed_end[relation]
        };
        Task::Activate {
            identity_only,
            root: (end > 0).then_some(root),
            relation,
            arguments: self.graph.arguments(occurrence),
            occurrence,
            reader_scope,
            next: 0,
        }
    }
    fn task(&mut self, s: &mut Scheduled) -> bool {
        // A false conservative scope admits no remaining descendants. Keep a
        // lane owner's transaction intact; other readers can drain their roots
        // without completing an irrelevant immutable enumeration.
        if s.scope == Condition::FALSE && self.lane != Some(Owner::Task(s.id)) {
            let done = s.task.discard_tick();
            if done && matches!(&s.task, Task::Body(_)) {
                self.graph.retire_scope();
            }
            return done;
        }
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
                let previous = self.obligation_parts(b);
                let done = self.body_tick(s.id, b);
                if done {
                    self.graph.retire_scope();
                }
                if !done && self.obligation_parts(b) != previous {
                    self.sync_obligation(s.id, b);
                }
                done
            }
            Task::Activate {
                identity_only,
                root,
                relation,
                arguments,
                occurrence,
                reader_scope,
                next,
            } => {
                let triggers = if *identity_only {
                    &self.code.merge_triggers[*relation]
                } else {
                    &self.code.triggers[*relation]
                };
                if *next == triggers.len() {
                    true
                } else {
                    let (rule, head) = triggers[*next];
                    *next += 1;
                    let matches = if self.code.rules[rule].direct_anchor() {
                        Discovery::anchor(arguments.clone(), *occurrence, *reader_scope)
                    } else {
                        Discovery::Indexed(Box::new(
                            Matches::new(
                                &self.graph,
                                root.as_ref().expect("indexed trigger root").clone(),
                                self.code.clone(),
                                rule,
                                *reader_scope,
                                Some((head, *occurrence)),
                            )
                            .expect("internal trigger"),
                        ))
                    };
                    let end = if *identity_only {
                        self.code.merge_indexed_end[*relation]
                    } else {
                        self.code.indexed_end[*relation]
                    };
                    if *next >= end {
                        *root = None;
                    }
                    self.spawn_in_epoch(
                        s.scope,
                        Task::Search(Box::new(Search {
                            rule,
                            matches,
                            transport: None,
                            candidate: None,
                            commit: None,
                        })),
                        s.epoch.as_ref().expect("immutable reader epoch").clone(),
                    );
                    false
                }
            }
            Task::Search(search) => {
                if let Some(commit) = &mut search.commit {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.rules[search.rule].commit_dispatches += 1;
                    }
                    match commit.tick(
                        &mut self.graph,
                        &mut self.arena,
                        &mut self.history,
                        &mut self.ids,
                    ) {
                        CommitStatus::Applied(c) => {
                            #[cfg(feature = "diagnostics")]
                            {
                                self.diagnostics.rules[search.rule].applied += 1;
                            }
                            self.applications += 1;
                            let app = c.application;
                            if self
                                .normalization
                                .as_ref()
                                .is_some_and(|c| c.plan.terminal.contains(&search.rule))
                            {
                                let mut body =
                                    Body::new(app.id, app.body, app.variables, app.support);
                                body.phase = BodyPhase::Apply;
                                self.state = c.state;
                                self.step_application(app.id, search.rule, app.support);
                                self.sync_obligation(s.id, &body);
                                self.record(
                                    SnapshotKind::Application {
                                        rule: search.rule,
                                        event: app.id,
                                    },
                                    self.active,
                                );
                                s.task = Task::Body(Box::new(body));
                                return false;
                            }
                            self.state = c.state;
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
                            #[cfg(feature = "diagnostics")]
                            {
                                self.diagnostics.rules[search.rule].rejected += 1;
                            }
                            search.commit = None;
                            self.release_lane();
                        }
                        CommitStatus::Pending => {}
                        CommitStatus::Done => unreachable!(),
                    }
                    false
                } else if search.candidate.is_some() {
                    if self.acquire(Owner::Task(s.id)) {
                        if search.transport.is_none() {
                            search.transport = Some(self.coordinates.transport(
                                search.candidate.as_ref().unwrap().support,
                                s.epoch.as_ref().expect("immutable reader epoch"),
                            ));
                            return false;
                        }
                        let Progress::Complete(support) =
                            self.transport_tick(search.transport.as_mut().unwrap(), true)
                        else {
                            return false;
                        };
                        search.candidate.as_mut().unwrap().support = support;
                        search.transport = None;
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.rules[search.rule].commits_started += 1;
                        }
                        search.commit = Some(
                            Commit::new(
                                &self.graph,
                                &self.history,
                                self.state.clone(),
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
                    #[cfg(feature = "diagnostics")]
                    let before = search.matches.candidate_visits();
                    let status = search.matches.tick(&self.graph, &mut self.arena);
                    #[cfg(feature = "diagnostics")]
                    {
                        let d = &mut self.diagnostics.rules[search.rule];
                        d.matching_dispatches += 1;
                        if let Some(before) = before {
                            d.indexed_candidate_visits +=
                                search.matches.candidate_visits().unwrap() - before;
                        }
                        if matches!(&status, MatchStatus::Found(_)) {
                            d.found += 1;
                            if before.is_none() {
                                d.direct_anchor_matches += 1;
                            }
                        }
                    }
                    match status {
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
                    let relation = self
                        .graph
                        .fact(w.root(), occurrence)
                        .expect("wake occurrence")
                        .relation;
                    if self.code.merge_triggers[relation].is_empty() {
                        return false;
                    }
                    self.spawn_in_epoch(
                        s.scope,
                        self.activation(w.root(), occurrence, support, true),
                        s.epoch.as_ref().expect("immutable reader epoch").clone(),
                    );
                    false
                }
            },
        }
    }
    fn body_tick(&mut self, id: u64, b: &mut Body) -> bool {
        if b.normalizer.is_some() {
            return self.normalization_tick(id, b);
        }
        if b.dispatch.is_some() {
            return self.dispatch_tick(id, b);
        }
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
                    let operation = Operation::And(b.scope, self.active);
                    if let Some(scope) = self.arena.direct(operation) {
                        b.scope = scope;
                        b.phase = BodyPhase::Apply;
                    } else {
                        b.phase = BodyPhase::Filter;
                        b.job = Some(self.arena.start(operation));
                    }
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
                                self.state.graph.clone(),
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
                        self.state.graph.clone(),
                        b.variables[*x],
                        b.variables[*y],
                        b.scope,
                    ));
                    if self.normalization.is_some() {
                        b.merge = b.merge.take().map(Merge::with_links);
                    }
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
                    if !b.dispatch_checked
                        && b.end.is_none()
                        && let Some(config) = &self.normalization
                        && let Some(plan) = config.plan.choices.get(&b.instruction)
                    {
                        b.dispatch = Some(Box::new(dispatch::Dispatch::new(
                            &self.graph,
                            self.state.graph.clone(),
                            b.variables[plan.key],
                            b.scope,
                            plan.clone(),
                        )));
                        self.normalization_stats.conditional_dispatches += 1;
                        return false;
                    }
                    b.phase = BodyPhase::Choice;
                }
                _ => unreachable!(),
            },
            BodyPhase::Post => {
                let update = b.update.as_mut().unwrap();
                if let UpdateStatus::Complete(root) = update.tick(&mut self.graph) {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.body_posts += 1;
                    }
                    let occurrence = update.occurrence();
                    if let Some(config) = &self.normalization
                        && config
                            .plan
                            .relations
                            .contains(&self.graph.fact(root.clone(), occurrence).unwrap().relation)
                    {
                        b.normalizer = Some(Box::new(Normalizer::post(
                            config.clone(),
                            &self.graph,
                            root.clone(),
                            self.active,
                            occurrence,
                            b.scope,
                        )));
                        self.state.graph = root;
                        b.source_complete = true;
                        self.finish_body_record(id);
                        self.record(SnapshotKind::Post { occurrence }, self.active);
                        b.update = None;
                        return false;
                    }
                    self.state.graph = root.clone();
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Post { occurrence }, self.active);
                    self.spawn(b.scope, self.activation(root, occurrence, b.scope, false));
                    return true;
                }
            }
            BodyPhase::Merge => {
                let merge = b.merge.as_mut().unwrap();
                if let Some(root) = merge.tick(&mut self.graph, &mut self.arena) {
                    if let Some(config) = &self.normalization {
                        #[cfg(feature = "diagnostics")]
                        let changed = merge.changed_support() != Condition::FALSE;
                        b.normalizer = Some(Box::new(Normalizer::merged(
                            config.clone(),
                            &self.graph,
                            root.clone(),
                            self.active,
                            merge,
                        )));
                        self.state.graph = root;
                        b.source_complete = true;
                        self.finish_body_record(id);
                        self.record(SnapshotKind::Merge, self.active);
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.body_merges += 1;
                            self.diagnostics.merge_support_changes += u64::from(changed);
                        }
                        b.merge = None;
                        return false;
                    }
                    self.state.graph = root.clone();
                    let scope = merge.changed_support();
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.body_merges += 1;
                        self.diagnostics.merge_support_changes +=
                            u64::from(scope != Condition::FALSE);
                    }
                    let delta = merge.take_delta();
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Merge, self.active);

                    if let Some(delta) = delta {
                        self.spawn(
                            scope,
                            Task::Wake(Box::new(Wake::from_delta(&self.graph, delta))),
                        );
                    }
                    return true;
                }
            }
            BodyPhase::Fail => {
                if let Some(active) = poll(&mut b.job, &mut self.arena) {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.fail_applications += 1;
                        self.diagnostics.fail_support_changes += u64::from(self.active != active);
                    }
                    self.semantic_regions |= self.active != active;
                    self.active = active;
                    self.finish_body_record(id);
                    self.record(SnapshotKind::Failure, b.scope);
                    return true;
                }
            }
            BodyPhase::Choice => {
                let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                let end = b.end.unwrap_or(items.len());
                if b.index + 1 == end {
                    let instruction = items[b.index];
                    self.body(b.event, instruction, b.variables.clone(), b.scope);
                    return true;
                }
                if let Some(plan) = self
                    .normalization
                    .as_ref()
                    .and_then(|c| c.plan.choices.get(&b.instruction))
                    .filter(|p| !p.rejection.is_empty())
                {
                    b.rejection = Some(Box::new(producer::Split::new(
                        self.state.graph.clone(),
                        self.code.clone(),
                        plan.clone(),
                        b,
                        self.active,
                    )));
                    b.phase = BodyPhase::Guard;
                    return false;
                }
                let (choice, decision) = self.arena.fresh_scoped_choice(b.scope);
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.choice_births += 1;
                }
                b.decision = decision;
                self.births.insert(
                    choice,
                    Birth {
                        event: b.event,
                        instruction: b.instruction,
                        start: b.index,
                        split: b.index + (end - b.index) / 2,
                        end,
                        support: b.scope,
                        decision,
                    },
                );
                b.job = Some(self.arena.start(Operation::And(b.scope, decision)));
                b.phase = BodyPhase::Left;
                self.sync_obligation(id, b);
                self.record(SnapshotKind::Choice, self.active);
            }
            BodyPhase::Guard => {
                if let Some(result) = b
                    .rejection
                    .as_mut()
                    .unwrap()
                    .tick(&self.graph, &mut self.arena)
                {
                    b.rejection = None;
                    self.semantic_regions |= self.active != result.active;
                    self.active = result.active;
                    b.scope = result.total;
                    b.split_scopes = [result.left, result.right];
                    b.phase = BodyPhase::GuardLeft;
                    self.sync_obligation(id, b);
                    if let Some((choice, decision, support)) = result.birth {
                        let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                            unreachable!()
                        };
                        let end = b.end.unwrap_or(items.len());
                        self.births.insert(
                            choice,
                            Birth {
                                event: b.event,
                                instruction: b.instruction,
                                start: b.index,
                                split: b.index + (end - b.index) / 2,
                                end,
                                support,
                                decision,
                            },
                        );
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.choice_births += 1;
                        }
                        self.record(SnapshotKind::Choice, self.active);
                    }
                    if result.failed != Condition::FALSE {
                        self.record(SnapshotKind::Failure, result.failed);
                    }
                }
            }
            BodyPhase::GuardLeft => {
                let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                let end = b.end.unwrap_or(items.len());
                self.body_range(b, b.index, b.index + (end - b.index) / 2, b.split_scopes[0]);
                b.phase = BodyPhase::GuardRight;
            }
            BodyPhase::GuardRight => {
                let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                    unreachable!()
                };
                let end = b.end.unwrap_or(items.len());
                self.body_range(b, b.index + (end - b.index) / 2, end, b.split_scopes[1]);
                return true;
            }
            BodyPhase::Left => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                        unreachable!()
                    };
                    let end = b.end.unwrap_or(items.len());
                    let split = b.index + (end - b.index) / 2;
                    self.body_range(b, b.index, split, c);
                    b.job = Some(self.arena.start(Operation::And(b.scope, b.decision.not())));
                    b.phase = BodyPhase::Right;
                }
            }
            BodyPhase::Right => {
                if let Some(c) = poll(&mut b.job, &mut self.arena) {
                    let Instruction::Or(items) = &self.code.instructions[b.instruction] else {
                        unreachable!()
                    };
                    let end = b.end.unwrap_or(items.len());
                    let split = b.index + (end - b.index) / 2;
                    self.body_range(b, split, end, c);
                    return true;
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
                ObserveStatus::Event(event) => {
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.output_events += 1;
                        self.diagnostics.complete_answers +=
                            u64::from(matches!(&event, Output::End));
                    }
                    self.output = Some(event);
                }
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
    fn can_complete(&self) -> bool {
        // Without live choices, every surviving current obligation covers TRUE.
        // It blocks all completion; keep an existing certificate's progress intact.
        self.observer.is_none()
            && self.output.is_none()
            && (self.active != Condition::FALSE || self.ready.is_some())
            && !(self.ready.is_none()
                && self.active == Condition::TRUE
                && self.births.is_empty()
                && self.pending_root != self.obligations.empty())
    }
    fn completion(&mut self) {
        if !self.can_complete() {
            return;
        }
        #[cfg(feature = "diagnostics")]
        if self.ready.is_none() {
            self.diagnostics.certificates_started += 1;
        }
        let mut ready = self.ready.take().unwrap_or_else(|| Ready {
            epoch: self.coordinates.current(),
            transport: None,
            cursor: self
                .obligations
                .index
                .range(self.pending_root.clone(), [0; 4], [u64::MAX; 4]),
            blocked: Condition::FALSE,
            scope: self.active,
            job: None,
            phase: ReadyPhase::Scan,
        });
        match ready.phase {
            ReadyPhase::Scan => {
                if ready.job.is_some() {
                    if let Some(c) = measured_poll!(
                        &mut ready.job,
                        &mut self.arena,
                        self.diagnostics.shared.completion_boolean
                    ) {
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
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.obligation_rows_scanned += 1;
                    }
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
                if let Some(c) = measured_poll!(
                    &mut ready.job,
                    &mut self.arena,
                    self.diagnostics.shared.completion_boolean
                ) {
                    if c == Condition::FALSE {
                        return;
                    }
                    ready.scope = c;
                    ready.phase = ReadyPhase::Acquire;
                }
            }
            ReadyPhase::Acquire => {
                if self.acquire(Owner::Completion) {
                    ready.transport = Some(self.coordinates.transport(ready.scope, &ready.epoch));
                    ready.phase = ReadyPhase::Transport;
                }
            }
            ReadyPhase::Transport => {
                if let Progress::Complete(scope) =
                    self.transport_tick(ready.transport.as_mut().unwrap(), false)
                {
                    ready.scope = scope;
                    ready.epoch = self.coordinates.current();
                    ready.transport = None;
                    ready.job = Some(self.arena.start(Operation::And(scope, self.active)));
                    ready.phase = ReadyPhase::Filter;
                }
            }
            ReadyPhase::Filter => {
                if let Some(c) = measured_poll!(
                    &mut ready.job,
                    &mut self.arena,
                    self.diagnostics.shared.completion_boolean
                ) {
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
                if let Some(c) = measured_poll!(
                    &mut ready.job,
                    &mut self.arena,
                    self.diagnostics.shared.completion_boolean
                ) {
                    self.record(SnapshotKind::NormalForm, ready.scope);
                    self.semantic_regions |= self.active != c;
                    self.active = c;
                    #[cfg(feature = "diagnostics")]
                    {
                        self.diagnostics.certificates_published += 1;
                    }
                    self.observer = Some(Observe::new(
                        Completion {
                            id: self.ids.event(),
                            support: ready.scope,
                            state: self.state.clone(),
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

#[cfg(test)]
mod balanced_phase_tests {
    use super::*;
    #[test]
    fn disjunction_ranges_survive_physical_gc_at_every_phase() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("a(A);b(A);c(A);d(A);e(A);f(A);g(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut phases = BTreeSet::new();
        let mut internal = false;
        let mut answers = 0;
        for _ in 0..100000 {
            e.advance(1);
            for s in e.queue.iter().chain(e.parked.values()) {
                if let Task::Body(b) = &s.task
                    && matches!(e.code.instructions[b.instruction], Instruction::Or(_))
                {
                    phases.insert(b.phase as u8);
                    internal |= b.end.is_some();
                }
            }
            if let Some(Output::End) = e.take_output() {
                answers += 1;
            }
            e.request_collection();
            e.maintain(100000);
            assert!(!e.collecting());
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(answers, 7);
        assert!(internal);
        assert!(!phases.contains(&(BodyPhase::Filter as u8)));
        for phase in [
            BodyPhase::Dispatch,
            BodyPhase::Acquire,
            BodyPhase::Apply,
            BodyPhase::Choice,
            BodyPhase::Left,
            BodyPhase::Right,
        ] {
            assert!(
                phases.contains(&(phase as u8)),
                "missing phase {}",
                phase as u8
            );
        }
    }
}

#[cfg(test)]
mod parking_tail_tests {
    use super::*;
    #[test]
    fn failed_acquires_park_and_fifo_handoffs_survive_collection() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("p(X) <=> done(X).").unwrap(),
            &crate::syntax::parse_query("p(A),p(B),p(C),p(D)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut collected = false;
        let mut handoffs = 0;
        let mut answers = 0;
        let mut variables = Vec::new();
        let mut ports = Vec::new();
        let mut facts = 0;
        for _ in 0..100000 {
            if !collected && e.parked.len() >= 2 {
                e.request_collection();
                collected = true;
            }
            let previous_lane = e.lane;
            let first = e.waiting.front().copied();
            e.advance(1);
            if previous_lane.is_some() && previous_lane != e.lane && first.is_some() {
                assert!(
                    e.lane == first,
                    "waiting owner must receive next reservation"
                );
                handoffs += 1;
            }
            let waiting = e.waiting.iter().copied().collect::<BTreeSet<_>>();
            assert_eq!(waiting.len(), e.waiting.len());
            assert!(waiting == e.requested);
            assert!(
                e.queue
                    .iter()
                    .all(|s| !e.requested.contains(&Owner::Task(s.id)))
            );
            for owner in &waiting {
                if let Owner::Task(id) = owner {
                    assert!(e.parked.contains_key(id));
                }
            }
            for id in e.parked.keys() {
                assert!(waiting.contains(&Owner::Task(*id)));
            }
            if let Some(owner) = e.lane {
                assert!(!waiting.contains(&owner));
            }
            if let Some(event) = e.take_output() {
                match event {
                    Output::Variable { variable, .. } => variables.push(variable),
                    Output::Fact { relation, .. } => {
                        assert_eq!(e.program().signatures[relation].name, "done");
                        facts += 1;
                    }
                    Output::Port { variable } => ports.push(variable),
                    Output::End => answers += 1,
                    _ => {}
                }
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(collected && e.collections() > 0 && handoffs >= 2);
        assert!(e.delivery_done());
        assert_eq!(e.applications(), 4);
        assert_eq!(answers, 1);
        assert_eq!(facts, 4);
        variables.sort();
        ports.sort();
        assert_eq!(variables, ports);
        assert_eq!(variables.iter().collect::<BTreeSet<_>>().len(), 4);
    }
}

#[cfg(test)]
mod body_direct_tests {
    use super::*;

    #[test]
    fn acquired_body_uses_exact_identities_and_resumes_mixed_support() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("p(X) <=> q(X).").unwrap(),
            &crate::syntax::parse_query("p(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let (x_id, x) = e.arena.fresh_choice();
        let (y_id, y) = e.arena.fresh_choice();
        for (scope, active, expected) in [
            (Condition::TRUE, Condition::TRUE, Some(Condition::TRUE)),
            (x, Condition::TRUE, Some(x)),
            (Condition::TRUE, x, Some(x)),
            (x, x, Some(x)),
            (x, x.not(), Some(Condition::FALSE)),
            (x, y, None),
        ] {
            e.active = active;
            let mut b = Body::new(0, 0, Arc::new(Vec::new()), scope);
            b.phase = BodyPhase::Acquire;
            // An existing writer prevents both the shortcut and the general job.
            e.lane = Some(Owner::Completion);
            assert!(!e.body_tick(99, &mut b));
            assert!(matches!(b.phase, BodyPhase::Acquire));
            assert!(b.job.is_none());
            assert_eq!(b.scope, scope);
            assert!(e.waiting.pop_front() == Some(Owner::Task(99)));
            assert!(e.requested.remove(&Owner::Task(99)));
            e.lane = Some(Owner::Task(99));
            assert!(!e.body_tick(99, &mut b));
            if let Some(expected) = expected {
                assert!(matches!(b.phase, BodyPhase::Apply));
                assert!(b.job.is_none());
                assert_eq!(b.scope, expected);
            } else {
                assert!(matches!(b.phase, BodyPhase::Filter));
                assert!(b.job.is_some());
                for _ in 0..100 {
                    if matches!(b.phase, BodyPhase::Apply) {
                        break;
                    }
                    let roots: Vec<_> = b
                        .job
                        .as_ref()
                        .unwrap()
                        .roots()
                        .chain([scope, active, x, y])
                        .collect();
                    let mut gc = e.arena.collect(roots.into_iter());
                    while !gc.tick(&mut e.arena) {}
                    drop(gc);
                    assert!(!e.body_tick(99, &mut b));
                }
                assert!(matches!(b.phase, BodyPhase::Apply));
                for a in [false, true] {
                    for b_value in [false, true] {
                        assert_eq!(
                            e.arena.evaluate(b.scope, |id| {
                                if id == x_id {
                                    a
                                } else {
                                    assert_eq!(id, y_id);
                                    b_value
                                }
                            }),
                            a && b_value
                        );
                    }
                }
            }
            assert!(e.lane == Some(Owner::Task(99)));
        }
    }
}
