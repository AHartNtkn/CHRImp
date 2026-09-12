//! Incremental root gathering and collection at a frozen execution boundary.
use super::*;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use crate::{condition, graph, history, store};
use std::ops::Bound::{Excluded, Unbounded};

type Roots = std::vec::IntoIter<Root>;
#[derive(Clone, Copy)]
enum Phase {
    Tasks,
    Parked,
    Births,
    Ready,
    Observe,
    Graph,
    History,
    Pending,
    Arena,
}
pub(super) struct Collection {
    phase: Phase,
    index: usize,
    task_roots: bool,
    after: Option<u64>,
    slot: usize,
    trace: TraceCursor,
    graph_roots: Vec<Root>,
    history_roots: Vec<Root>,
    pending_roots: Vec<Root>,
    conditions: Vec<Condition>,
    graph: Option<graph::Collector<Roots>>,
    history: Option<history::Collector<Roots>>,
    pending: Option<store::Collector<Roots>>,
    arena: Option<condition::Collector<std::vec::IntoIter<Condition>>>,
}
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Memory {
    pub graph_nodes: usize,
    pub occurrences: usize,
    pub conditions: usize,
    pub history_nodes: usize,
    pub history_records: usize,
    pub pending_nodes: usize,
    pub choices: usize,
}
impl Memory {
    fn total(self) -> usize {
        self.graph_nodes
            .saturating_add(self.occurrences)
            .saturating_add(self.conditions)
            .saturating_add(self.history_nodes)
            .saturating_add(self.history_records)
            .saturating_add(self.pending_nodes)
            .saturating_add(self.choices)
    }
}
impl Engine {
    pub fn memory(&self) -> Memory {
        Memory {
            graph_nodes: self.graph.index_node_count(),
            occurrences: self.graph.occurrence_count(),
            conditions: self.arena.node_count(),
            history_nodes: self.history.node_count(),
            history_records: self.history.record_count(),
            pending_nodes: self.pending.node_count(),
            choices: self.births.len(),
        }
    }
    pub fn request_collection(&mut self) {
        self.collection_requested = true;
    }
    pub fn collections(&self) -> u64 {
        self.collections
    }
    pub fn collecting(&self) -> bool {
        self.collector.is_some() || self.collection_requested
    }
    pub(super) fn collect_heap(&mut self) -> bool {
        if self.collector.is_none() {
            if !self.collection_requested && self.memory().total() <= self.collection_limit {
                return false;
            }
            self.collection_requested = false;
            let mut pending_roots = vec![self.pending_root];
            if let Some(ready) = &self.ready {
                pending_roots.push(ready.cursor.root());
            }
            self.collector = Some(Collection {
                phase: Phase::Tasks,
                index: 0,
                task_roots: false,
                after: None,
                slot: 0,
                trace: TraceCursor::default(),
                graph_roots: vec![self.state.graph],
                history_roots: vec![self.state.history],
                pending_roots,
                conditions: vec![self.active, self.failed],
                graph: None,
                history: None,
                pending: None,
                arena: None,
            });
            return true;
        }
        let mut c = self.collector.take().unwrap();
        match c.phase {
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
                                if let Some(u) = &b.update {
                                    c.graph_roots.extend(u.roots());
                                }
                                if let Some(m) = &b.merge {
                                    c.graph_roots.extend(m.roots());
                                }
                            }
                            Task::Search(s) => {
                                c.graph_roots.push(s.matches.root());
                                if let Some(commit) = &s.commit {
                                    c.graph_roots.extend(commit.graph_roots());
                                    c.history_roots.extend(commit.history_roots());
                                }
                            }
                            Task::Activate { root, .. } => c.graph_roots.push(*root),
                            Task::Wake(w) => c.graph_roots.push(w.root()),
                            Task::Init(_) => {}
                        }
                        c.task_roots = true;
                    } else {
                        match task.trace(&mut c.trace) {
                            Step::Root(root) => c.conditions.push(root),
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
                let next = match c.after {
                    Some(id) => self.births.range((Excluded(id), Unbounded)).next(),
                    None => self.births.first_key_value(),
                };
                if let Some((&id, b)) = next {
                    c.conditions
                        .push(if c.slot == 0 { b.support } else { b.decision });
                    c.slot += 1;
                    if c.slot == 2 {
                        c.slot = 0;
                        c.after = Some(id);
                    }
                } else {
                    c.phase = Phase::Ready;
                }
            }
            Phase::Ready => {
                match c.trace.optional(self.ready.as_ref()) {
                    Step::Root(root) => c.conditions.push(root),
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
                            Step::Root(root) => c.conditions.push(root),
                            Step::Pending => {}
                            Step::Done => c.phase = Phase::Graph,
                        }
                    }
                } else {
                    c.phase = Phase::Graph;
                }
            }
            Phase::Graph => {
                if let Some(gc) = &mut c.graph {
                    if let Some(root) = gc.tick(&mut self.graph) {
                        c.conditions.push(root);
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
                        c.conditions.push(root);
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
                    if let Some((_, root)) = gc.tick(&mut self.pending) {
                        c.conditions.push(root);
                    }
                    if gc.done() {
                        c.pending = None;
                        c.phase = Phase::Arena;
                    }
                } else {
                    c.pending = Some(
                        self.pending
                            .collect(std::mem::take(&mut c.pending_roots).into_iter()),
                    );
                }
            }
            Phase::Arena => {
                if let Some(gc) = &mut c.arena {
                    if gc.tick(&mut self.arena) {
                        c.arena = None;
                        self.collections += 1;
                        self.collection_limit =
                            self.memory().total().saturating_mul(2).saturating_add(1024);
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
            Task::Init(_) | Task::Activate { .. } => c.advance(),
        }
    }
}
impl Trace for Body {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope, self.decision, self.pending_active]),
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
            _ => Step::Done,
        }
    }
}
impl Trace for Ready {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope, self.blocked]),
            1 => c.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}
