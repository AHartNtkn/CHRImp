//! Incremental root gathering and collection at a frozen execution boundary.
use super::*;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use crate::{condition, graph, history};
use std::ops::Bound::{Excluded, Unbounded};

// The same bounded slack serves physical and semantic cleanup accounting.
const CLEANUP_ALLOWANCE: usize = 1024;
type Roots = std::vec::IntoIter<Root>;
fn retain_condition(roots: &mut Vec<Condition>, root: Condition) {
    // Terminals own no arena nodes and need no later collection visit.
    if !root.is_terminal() {
        roots.push(root);
    }
}
#[derive(Clone, Copy)]
enum Phase {
    Compact,
    Prune,
    Tasks,
    Parked,
    Births,
    Coordinates,
    Ready,
    Observe,
    Snapshots,
    Inspections,
    RuleStep,
    TrimBirths,
    SeedVariables,
    PruneGraph,
    Graph,
    History,
    Pending,
    Arena,
}
pub(super) struct Collection {
    phase: Phase,
    compact: Option<super::compact::Compact>,
    owns_lane: bool,
    retain_choice: Option<u64>,
    prune: Option<history::Prune>,
    prune_graph: Option<graph::Prune>,
    variable_groups: BTreeMap<usize, (Arc<Vec<u64>>, Condition)>,
    seed_job: Option<(usize, Job)>,
    seed_slot: usize,
    index: usize,
    task_roots: bool,
    after: Option<u64>,
    slot: usize,
    trace: TraceCursor,
    graph_roots: Vec<Root>,
    history_roots: Vec<Root>,
    pending_roots: Vec<PendingRoot>,
    conditions: Vec<Condition>,
    graph: Option<graph::Collector<Roots>>,
    history: Option<history::Collector<Roots>>,
    pending: Option<obligations::Collector>,
    arena: Option<condition::Collector<std::vec::IntoIter<Condition>>>,
}
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Memory {
    pub graph_nodes: usize,
    pub release_batches: usize,
    pub occurrences: usize,
    pub conditions: usize,
    pub history_nodes: usize,
    pub history_records: usize,
    pub pending_nodes: usize,
    pub obligation_descriptors: usize,
    pub choices: usize,
    pub coordinate_records: usize,
    pub snapshots: usize,
    pub inspections: usize,
}
impl Memory {
    fn total(self) -> usize {
        self.graph_nodes
            .saturating_add(self.release_batches)
            .saturating_add(self.occurrences)
            .saturating_add(self.conditions)
            .saturating_add(self.history_nodes)
            .saturating_add(self.history_records)
            .saturating_add(self.pending_nodes)
            .saturating_add(self.obligation_descriptors)
            .saturating_add(self.choices)
            .saturating_add(self.coordinate_records)
            .saturating_add(self.snapshots)
            .saturating_add(self.inspections)
    }
}
impl Engine {
    pub fn memory(&self) -> Memory {
        Memory {
            graph_nodes: self.graph.index_node_count(),
            release_batches: self.graph.index.release_batches()
                + self.history.index.release_batches()
                + self.obligations.index.release_batches(),
            occurrences: self.graph.occurrence_count(),
            conditions: self.arena.node_count(),
            history_nodes: self.history.node_count(),
            history_records: self.history.record_count(),
            pending_nodes: self.obligations.index.node_count(),
            obligation_descriptors: self.obligations.descriptor_count(),
            choices: self.births.len(),
            coordinate_records: self.coordinates.memory(),
            snapshots: self.snapshots.len(),
            inspections: self.inspections.len(),
        }
    }
    pub fn request_collection(&mut self) {
        self.collection_requested = true;
    }
    pub fn collections(&self) -> u64 {
        self.collections
    }
    pub fn collecting(&self) -> bool {
        self.collector.is_some()
            || self.collection_requested
            || self.lane == Some(Owner::Collection)
            || self.requested.contains(&Owner::Collection)
    }
    pub(super) fn collect_heap(&mut self) -> bool {
        self.collect_heap_mode(true)
    }
    pub(super) fn collect_heap_mode(&mut self, semantic: bool) -> bool {
        if self.collector.is_none() {
            let memory = self.memory().total();
            let explicit = self.collection_requested;
            // Semantic obsolescence is independent of physical node churn. Pure
            // growth earns a larger frontier; invalidations pay for a future pass.
            let semantic_due = semantic
                && !self.canceled()
                && (self.state.graph != self.graph.empty()
                    || self.state.history != self.history.empty()
                    || !self.births.is_empty()
                    || self.lane == Some(Owner::Collection))
                && (self.semantic_regions
                    || (self.graph.semantic_debt() > 0
                        && (self.requested.contains(&Owner::Collection)
                            || self.lane == Some(Owner::Collection)
                            || {
                                let minimum = self
                                    .variables
                                    .len()
                                    .saturating_add(self.pending_tasks())
                                    .saturating_add(CLEANUP_ALLOWANCE);
                                // Occurrence counts cannot lower this frontier.
                                self.graph.semantic_debt() >= minimum
                                    && self.graph.semantic_debt()
                                        >= self
                                            .graph
                                            .occurrence_frontier(&self.state.graph)
                                            .saturating_add(minimum)
                            })));
            if !explicit
                && !semantic_due
                && memory <= self.collection_limit
                && (self.lane != Some(Owner::Collection) || self.canceled() || !semantic)
            {
                return false;
            }
            // Routine pressure reserves the FIFO maintenance lane. Let finite
            // earlier writers finish rather than automatically paying for both
            // a physical pass and a semantic pass over nearly the same heap.
            // Explicit requests bypass the pressure gate; a completed pass still
            // yields one ordinary advance step. If waiting consumes another
            // soft-limit-sized allowance, physical GC interrupts the writer;
            // allocation can exceed this watermark by at most one source tick.
            let owns_lane = semantic
                && !self.canceled()
                && (self.state.graph != self.graph.empty()
                    || self.state.history != self.history.empty()
                    || !self.births.is_empty()
                    || self.lane == Some(Owner::Collection))
                && self.acquire(Owner::Collection);
            if !owns_lane
                && semantic
                && !self.canceled()
                && !explicit
                && self.requested.contains(&Owner::Collection)
                && memory < self.collection_limit.saturating_mul(2)
            {
                return false;
            }
            self.collection_requested = false;
            let mut pending_roots = vec![self.pending_root.clone()];
            if let Some(ready) = &self.ready {
                pending_roots.push(ready.cursor.root());
            }
            if let Some(root) = self.rule_step.as_ref().and_then(|s| s.pending_root()) {
                pending_roots.push(root);
            }
            self.collector = Some(Box::new(Collection {
                phase: if owns_lane {
                    Phase::Compact
                } else {
                    Phase::Tasks
                },
                compact: owns_lane.then(|| super::compact::Compact::new(self)),
                owns_lane,
                retain_choice: None,
                prune: None,
                prune_graph: None,
                variable_groups: BTreeMap::new(),
                seed_job: None,
                seed_slot: 0,
                index: 0,
                task_roots: false,
                after: None,
                slot: 0,
                trace: TraceCursor::default(),
                graph_roots: if owns_lane {
                    vec![]
                } else {
                    vec![self.state.graph.clone()]
                },
                history_roots: if owns_lane {
                    vec![]
                } else {
                    vec![self.state.history.clone()]
                },
                pending_roots,
                conditions: [self.active]
                    .into_iter()
                    .filter(|root| !root.is_terminal())
                    .collect(),
                graph: None,
                history: None,
                pending: None,
                arena: None,
            }));
            return true;
        }
        let mut c = self.collector.take().unwrap();
        match c.phase {
            Phase::Compact => {
                if c.compact.as_mut().unwrap().tick(self) {
                    c.compact = None;
                    c.prune = Some(self.history.prune(
                        &self.graph,
                        self.state.graph.clone(),
                        self.state.history.clone(),
                        self.active,
                    ));
                    c.prune_graph = Some(self.graph.prune(self.state.graph.clone(), self.active));
                    c.variable_groups.insert(
                        Arc::as_ptr(&self.variables) as usize,
                        (self.variables.clone(), self.active),
                    );
                    c.pending_roots[0] = self.pending_root.clone();
                    c.conditions.clear();
                    retain_condition(&mut c.conditions, self.active);
                    c.phase = Phase::Prune;
                }
            }
            Phase::Prune => {
                if let Some(root) = c.prune.as_mut().expect("history pruning").tick(
                    &self.graph,
                    &mut self.history,
                    &mut self.arena,
                ) {
                    self.state.history = root.clone();
                    c.history_roots.push(root);
                    c.prune = None;
                    c.phase = Phase::Tasks;
                }
            }
            Phase::Tasks | Phase::Parked => {
                let task = if matches!(c.phase, Phase::Tasks) {
                    self.queue.get(c.index)
                } else {
                    match c.after {
                        Some(id) => self.parked.range((Excluded(id), Unbounded)).next(),
                        None => self.parked.first_key_value(),
                    }
                    .map(|(_, task)| task)
                };
                if let Some(task) = task {
                    if !c.task_roots {
                        match &task.task {
                            Task::Body(b) => {
                                if c.owns_lane {
                                    let key = Arc::as_ptr(&b.variables) as usize;
                                    match c.variable_groups.entry(key) {
                                        std::collections::btree_map::Entry::Vacant(entry) => {
                                            entry.insert((b.variables.clone(), b.scope));
                                        }
                                        std::collections::btree_map::Entry::Occupied(entry) => {
                                            c.seed_job = Some((
                                                key,
                                                self.arena
                                                    .start(Operation::Or(entry.get().1, b.scope)),
                                            ));
                                        }
                                    }
                                }
                                if let Some(u) = &b.update {
                                    c.graph_roots.extend(u.roots());
                                }
                                if let Some(m) = &b.merge {
                                    c.graph_roots.extend(m.roots());
                                }
                            }
                            Task::Search(s) => {
                                c.graph_roots.extend(s.matches.root());
                                if let Some(commit) = &s.commit {
                                    c.graph_roots.extend(commit.graph_roots());
                                    c.history_roots.extend(commit.history_roots());
                                }
                            }
                            Task::Activate { root, .. } => c.graph_roots.extend(root.clone()),
                            Task::Wake(w) => c.graph_roots.push(w.root()),
                            Task::Init(_) => {}
                        }
                        c.task_roots = true;
                    } else if let Some((key, job)) = &mut c.seed_job {
                        if let Progress::Complete(support) = job.tick(&mut self.arena) {
                            c.variable_groups.get_mut(key).expect("variable group").1 = support;
                            c.seed_job = None;
                        }
                    } else {
                        match task.trace(&mut c.trace) {
                            Step::Root(root) => retain_condition(&mut c.conditions, root),
                            Step::Pending => {}
                            Step::Done => {
                                if matches!(c.phase, Phase::Tasks) {
                                    c.index += 1;
                                } else {
                                    c.after = Some(task.id);
                                }
                                c.task_roots = false;
                                c.trace = TraceCursor::default();
                            }
                        }
                    }
                } else {
                    c.phase = if matches!(c.phase, Phase::Tasks) {
                        Phase::Parked
                    } else {
                        Phase::Births
                    };
                    c.after = None;
                }
            }
            Phase::Births => {
                if self.cancellation.roots_released {
                    c.phase = Phase::Coordinates;
                    c.after = None;
                    c.slot = 0;
                    self.collector = Some(c);
                    return true;
                }
                let next = match c.after {
                    Some(id) => self.births.range((Excluded(id), Unbounded)).next(),
                    None => self.births.first_key_value(),
                };
                if let Some((&id, b)) = next {
                    retain_condition(
                        &mut c.conditions,
                        if c.slot == 0 { b.support } else { b.decision },
                    );
                    c.slot += 1;
                    if c.slot == 2 {
                        c.slot = 0;
                        c.after = Some(id);
                    }
                } else {
                    c.phase = Phase::Coordinates;
                }
            }
            Phase::Coordinates => match self.coordinates.trace(&mut c.trace) {
                Step::Root(root) => retain_condition(&mut c.conditions, root),
                Step::Pending => {}
                Step::Done => {
                    c.trace = TraceCursor::default();
                    c.phase = Phase::Ready;
                }
            },
            Phase::Ready => {
                match c.trace.optional(self.ready.as_ref()) {
                    Step::Root(root) => retain_condition(&mut c.conditions, root),
                    Step::Pending => {}
                    Step::Done => unreachable!(),
                }
                if c.trace.phase == 1 {
                    c.trace = TraceCursor::default();
                    c.phase = Phase::Observe;
                }
            }
            Phase::Observe => {
                if let Some(observer) = &self.observer {
                    if c.slot == 0 {
                        c.graph_roots.push(observer.graph_root());
                        c.slot = 1;
                    } else {
                        match observer.trace(&mut c.trace) {
                            Step::Root(root) => retain_condition(&mut c.conditions, root),
                            Step::Pending => {}
                            Step::Done => {
                                c.phase = Phase::Snapshots;
                                c.after = None;
                                c.slot = 0;
                                c.trace = TraceCursor::default();
                            }
                        }
                    }
                } else {
                    c.phase = Phase::Snapshots;
                    c.after = None;
                    c.slot = 0;
                    c.trace = TraceCursor::default();
                }
            }
            Phase::Snapshots => {
                let next = match c.after {
                    Some(id) => self.snapshots.range((Excluded(id), Unbounded)).next(),
                    None => self.snapshots.first_key_value(),
                };
                if let Some((&id, snapshot)) = next {
                    c.graph_roots.push(snapshot.graph.clone());
                    c.pending_roots.push(snapshot.obligations.clone());
                    c.retain_choice = c.retain_choice.max(snapshot.info.last_choice);
                    retain_condition(&mut c.conditions, snapshot.scope);
                    c.after = Some(id);
                } else {
                    c.phase = Phase::Inspections;
                    c.after = None;
                }
            }
            Phase::Inspections => {
                let next = match c.after {
                    Some(id) => self.inspections.range((Excluded(id), Unbounded)).next(),
                    None => self.inspections.first_key_value(),
                };
                if let Some((&id, inspection)) = next {
                    if c.slot == 0 {
                        if let Some(snapshot) = &inspection.snapshot {
                            c.graph_roots.push(snapshot.graph.clone());
                            c.pending_roots.push(snapshot.obligations.clone());
                            c.retain_choice = c.retain_choice.max(snapshot.info.last_choice);
                        }
                        c.slot = 1;
                    } else {
                        match inspection.trace(&mut c.trace) {
                            Step::Root(root) => retain_condition(&mut c.conditions, root),
                            Step::Pending => {}
                            Step::Done => {
                                c.after = Some(id);
                                c.slot = 0;
                                c.trace = TraceCursor::default();
                            }
                        }
                    }
                } else {
                    c.phase = Phase::RuleStep;
                }
            }
            Phase::RuleStep => {
                match c.trace.optional(self.rule_step.as_ref()) {
                    Step::Root(root) => retain_condition(&mut c.conditions, root),
                    Step::Pending => {}
                    Step::Done => unreachable!(),
                }
                if c.trace.phase == 1 {
                    c.trace = TraceCursor::default();
                    c.phase = if self.cancellation.roots_released {
                        Phase::TrimBirths
                    } else {
                        Phase::SeedVariables
                    };
                    c.after = None;
                    c.slot = 0;
                }
            }
            Phase::TrimBirths => {
                let next = match c.after {
                    Some(id) => self.births.range((Excluded(id), Unbounded)).next(),
                    None => self.births.first_key_value(),
                };
                if let Some((&id, birth)) = next {
                    if c.retain_choice.is_some_and(|last| id <= last) {
                        retain_condition(
                            &mut c.conditions,
                            if c.slot == 0 {
                                birth.support
                            } else {
                                birth.decision
                            },
                        );
                        c.slot += 1;
                        if c.slot == 2 {
                            c.slot = 0;
                            c.after = Some(id);
                        }
                    } else {
                        self.births.remove(&id);
                        c.after = Some(id);
                    }
                } else {
                    c.phase = Phase::SeedVariables;
                }
            }
            Phase::SeedVariables => {
                if let Some((_, (variables, support))) = c.variable_groups.first_key_value() {
                    if let Some(&variable) = variables.get(c.seed_slot) {
                        c.prune_graph
                            .as_mut()
                            .expect("graph pruning")
                            .seed(variable, *support);
                        c.seed_slot += 1;
                    } else {
                        c.variable_groups.pop_first();
                        c.seed_slot = 0;
                    }
                } else {
                    c.phase = Phase::PruneGraph;
                }
            }
            Phase::PruneGraph => {
                if let Some(prune) = &mut c.prune_graph {
                    if let Some(root) = prune.tick(&mut self.graph, &mut self.arena) {
                        self.state.graph = root.clone();
                        c.graph_roots.push(root);
                        c.prune_graph = None;
                        c.phase = Phase::Graph;
                    }
                } else {
                    c.phase = Phase::Graph;
                }
            }
            Phase::Graph => {
                if let Some(gc) = &mut c.graph {
                    if let Some(root) = gc.tick(&mut self.graph) {
                        retain_condition(&mut c.conditions, root);
                    }
                    if gc.done() {
                        c.graph = None;
                        c.phase = Phase::History;
                    }
                } else {
                    c.graph = Some(
                        self.graph
                            .collect(std::mem::take(&mut c.graph_roots).into_iter()),
                    );
                }
            }
            Phase::History => {
                if let Some(gc) = &mut c.history {
                    if let Some(root) = gc.tick(&mut self.history) {
                        retain_condition(&mut c.conditions, root);
                    }
                    if gc.done() {
                        c.history = None;
                        c.phase = Phase::Pending;
                    }
                } else {
                    c.history = Some(
                        self.history
                            .collect(std::mem::take(&mut c.history_roots).into_iter()),
                    );
                }
            }
            Phase::Pending => {
                if let Some(gc) = &mut c.pending {
                    if let Some(roots) = gc.tick(&mut self.obligations) {
                        c.conditions
                            .extend(roots.into_iter().filter(|root| !root.is_terminal()));
                    }
                    if gc.done() {
                        c.pending = None;
                        c.phase = Phase::Arena;
                    }
                } else {
                    c.pending = Some(
                        self.obligations
                            .collect(std::mem::take(&mut c.pending_roots).into_iter()),
                    );
                }
            }
            Phase::Arena => {
                if let Some(gc) = &mut c.arena {
                    if gc.tick(&mut self.arena) {
                        c.arena = None;
                        self.collections += 1;
                        if c.owns_lane {
                            self.release_lane();
                        }
                        if c.owns_lane {
                            self.graph.semantic_collected();
                            self.semantic_regions = false;
                        }
                        // A completed finite pass yields to one ordinary service
                        // step. Repeated requests cannot consume every advance.
                        self.collection_yield = true;
                        self.collection_limit = self
                            .memory()
                            .total()
                            .saturating_mul(2)
                            .saturating_add(CLEANUP_ALLOWANCE);
                        return true;
                    }
                } else {
                    c.arena = Some(
                        self.arena
                            .collect(std::mem::take(&mut c.conditions).into_iter()),
                    );
                }
            }
        }
        self.collector = Some(c);
        true
    }
}
impl Trace for Scheduled {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope]),
            1 => c.optional(Some(&self.task)),
            _ => Step::Done,
        }
    }
}
impl Trace for Task {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        if c.phase != 0 {
            return Step::Done;
        }
        match self {
            Task::Body(b) => c.optional(Some(b.as_ref())),
            Task::Search(s) => c.optional(Some(s.as_ref())),
            Task::Wake(w) => c.optional(Some(w.as_ref())),
            Task::Activate { reader_scope, .. } => c.fields(&[*reader_scope]),
            Task::Init(_) => c.advance(),
        }
    }
}
impl Trace for Body {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope, self.decision]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.merge.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Search {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.optional(Some(&self.matches)),
            1 => c.fields(&[self
                .candidate
                .as_ref()
                .map_or(Condition::FALSE, |m| m.support)]),
            2 => c.optional(self.commit.as_ref()),
            3 => c.optional(self.transport.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Ready {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope, self.blocked]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.transport.as_ref()),
            _ => Step::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_unposted_body_local_keeps_its_identity_through_pruning() {
        let ports = vec!["Y"; 128].join(",");
        let code = crate::program::prepare(
            &crate::syntax::parse_program(&format!("start(X) <=> X=Y,final({ports}).")).unwrap(),
            &crate::syntax::parse_query("start(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut retained = None;
        for _ in 0..10000 {
            e.advance(1);
            if e.lane.is_none() {
                retained = e.queue.iter().find_map(|task| match &task.task {
                    Task::Body(body)
                        if body.variables.len() > 1
                            && matches!(body.phase, BodyPhase::Arguments) =>
                    {
                        let x = body.variables[0];
                        let y = body.variables[1];
                        (e.graph
                            .index
                            .get(&e.state.graph, &[crate::identity::PARENT, y, x, 0])
                            == Some(Condition::TRUE))
                        .then_some((x, y))
                    }
                    _ => None,
                });
                if retained.is_some() {
                    break;
                }
            }
        }
        let (x, y) = retained.expect("aliased body local awaiting its first post");
        assert!(
            e.graph
                .relation(e.state.graph.clone(), 0)
                .unwrap()
                .next(&e.graph)
                .is_none()
        );
        assert!(
            e.graph
                .relation(e.state.graph.clone(), 1)
                .unwrap()
                .next(&e.graph)
                .is_none()
        );
        e.request_collection();
        for _ in 0..100000 {
            e.advance(1);
            if !e.collecting() {
                break;
            }
        }
        assert!(!e.collecting());
        assert_eq!(
            e.graph
                .index
                .get(&e.state.graph, &[crate::identity::PARENT, y, x, 0]),
            Some(Condition::TRUE)
        );
        let mut ports = 0;
        for _ in 0..100000 {
            e.advance(1);
            if let Some(Output::Port { variable }) = e.take_output() {
                assert_eq!(variable, x);
                ports += 1;
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(ports, 128);
    }
    #[test]
    fn collection_status_covers_waiting_for_a_history_writer() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("p(X) ==> seen(X). loop(X) <=> loop(X).").unwrap(),
            &crate::syntax::parse_query("p(X),loop(X)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(std::sync::Arc::new(code));
        for _ in 0..10000 {
            e.advance(1);
            if e.state.history != e.history.empty() && matches!(e.lane, Some(Owner::Task(_))) {
                break;
            }
        }
        assert!(matches!(e.lane, Some(Owner::Task(_))));
        assert_ne!(e.state.history, e.history.empty());
        e.request_collection();
        e.advance(1);
        assert!(e.requested.contains(&Owner::Collection));
        for _ in 0..100000 {
            if e.collector.is_none() {
                break;
            }
            e.advance(1);
        }
        assert!(e.collector.is_none());
        assert!(e.requested.contains(&Owner::Collection));
        assert!(
            e.collecting(),
            "the requested semantic pass still awaits its writer"
        );
        let before = e.collections();
        for _ in 0..100000 {
            if !e.collecting() {
                break;
            }
            e.advance(1);
        }
        assert!(!e.collecting());
        assert!(e.collections() > before);
        assert!(!e.requested.contains(&Owner::Collection));
    }
    #[test]
    fn routine_pressure_preserves_fifo_and_emergency_or_explicit_gc_bounds_writer_growth() {
        for explicit in [false, true] {
            let ports = vec!["A"; 8192].join(",");
            let code = crate::program::prepare(
                &crate::syntax::parse_program("").unwrap(),
                &crate::syntax::parse_query(&format!("seed(A),wide({ports})")).unwrap(),
            )
            .unwrap();
            let mut e = Engine::new(Arc::new(code));
            let mut deferred = false;
            let mut forced = false;
            let mut physical = 0;
            let mut writer = None;
            let mut handed_off = false;
            let mut peak = 0;
            let mut max_growth = 0;
            let mut deferred_ticks = 0;
            for _ in 0..1200000 {
                let before = e.memory().total();
                let emergency = e.collection_limit.saturating_mul(2);
                let collecting = e.collector.is_some();
                let requested = e.collection_requested;
                let ticks = e.ticks;
                let lane = e.lane;
                e.advance(1);
                let memory = e.memory().total();
                peak = peak.max(memory);
                if requested && !collecting {
                    assert!(matches!(lane, Some(Owner::Task(_))));
                    assert!(
                        e.collector.as_ref().is_some_and(|gc| !gc.owns_lane),
                        "an explicit request must start physical GC on this advance"
                    );
                    assert_eq!(e.ticks, ticks);
                    assert!(e.lane == lane);
                }
                if !collecting {
                    max_growth = max_growth.max(memory.saturating_sub(before));
                    if let Some(gc) = &e.collector {
                        if !gc.owns_lane {
                            physical += 1;
                            assert!(
                                requested || before >= emergency,
                                "routine pressure must wait for its FIFO owner: {before}/{emergency}"
                            );
                            assert_eq!(
                                e.ticks, ticks,
                                "a physical pass starts before another writer step"
                            );
                            assert!(e.lane == lane);
                        }
                    } else if e.requested.contains(&Owner::Collection) {
                        assert!(before < emergency);
                        assert!(memory <= emergency.saturating_add(max_growth));
                        assert_eq!(
                            e.waiting
                                .iter()
                                .filter(|owner| **owner == Owner::Collection)
                                .count(),
                            1
                        );
                        if matches!(e.lane, Some(Owner::Task(_))) {
                            writer.get_or_insert(e.lane.unwrap());
                            deferred = true;
                            deferred_ticks += usize::from(e.ticks != ticks);
                            if explicit && !forced {
                                e.request_collection();
                                forced = true;
                            }
                        }
                    }
                }
                if writer.is_some() && e.lane == Some(Owner::Collection) {
                    handed_off = true;
                }
                e.take_output();
                if e.delivery_done() && !e.collecting() {
                    break;
                }
            }
            assert!(deferred && deferred_ticks > 0);
            assert!(physical > 0, "wide writer must exercise the emergency path");
            assert!(
                handed_off && e.delivery_done(),
                "finite writer must reach its queued maintenance owner"
            );
            if explicit {
                assert!(forced);
            }
            eprintln!(
                "pressure explicit={explicit}: peak={peak}, physical={physical}, deferred_ticks={deferred_ticks}, max_source_growth={max_growth}"
            );
        }
    }
}

#[cfg(test)]
mod frontier_tests {
    use super::*;
    fn engine(debt: usize) -> Engine {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("p(X) ==> q(X).").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let x = e.ids.variable();
        e.variables = Arc::new(vec![x]);
        let mut post = e
            .graph
            .post(e.state.graph.clone(), 0, vec![x], Condition::TRUE)
            .unwrap();
        loop {
            if let UpdateStatus::Complete(root) = post.tick(&mut e.graph) {
                e.state.graph = root;
                break;
            }
        }
        for _ in 0..debt {
            e.graph.retire_scope();
        }
        e.collection_limit = usize::MAX;
        e
    }
    #[test]
    fn exact_frontier_boundary_and_explicit_region_requests_are_preserved() {
        let e = engine(0);
        let minimum = e.variables.len() + e.pending_tasks() + CLEANUP_ALLOWANCE;
        let full = minimum + e.graph.occurrence_frontier(&e.state.graph);
        for debt in [minimum - 1, minimum, full - 1, full, full + 1] {
            let mut e = engine(debt);
            assert_eq!(e.collect_heap(), debt >= full, "debt {debt}");
        }
        for region in [false, true] {
            let mut e = engine(1);
            if region {
                e.semantic_regions = true;
            } else {
                e.request_collection();
            }
            assert!(e.collect_heap());
            assert!(e.collector.is_some());
        }
    }
}
