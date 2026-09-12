//! Read-only supported class enumeration against a frozen graph root.
//!
//! Resolve representatives first, then follow their reverse parent prefixes.
//! Children may coexist on overlapping support. Only previously visited support
//! for the same variable is subtracted; emitted fragments never overlap.

use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::graph::Graph;
use crate::identity::{CHILD, Resolve, ResolveStatus};
use crate::store::{Cursor, Root};
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy)]
enum Phase {
    Resolve,
    Enqueue,
    Next,
    Novel,
    Remember,
    Scan,
    Hit,
    Cleanup,
    Done,
}

/// Retain `root()` for graph collection and `condition_roots()` for condition
/// collection while suspended. Consumers own the supports returned by `Found`.
/// Disjoint fragments for one member may be returned separately.
pub struct Members {
    root: Root,
    resolver: Option<Resolve>,
    pending: BTreeMap<u64, Condition>,
    queue: VecDeque<u64>,
    visited: BTreeMap<u64, Condition>,
    variable: u64,
    target: u64,
    fresh: Condition,
    boolean: Option<Job>,
    cursor: Option<Cursor>,
    phase: Phase,
    visits: u64,
    discard: u8,
}

impl Members {
    pub fn new(g: &Graph, root: Root, variable: u64, scope: Condition) -> Self {
        Self {
            discard: 0,
            root,
            resolver: Some(Resolve::new(g, root, variable, scope)),
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            visited: BTreeMap::new(),
            variable,
            target: variable,
            fresh: Condition::FALSE,
            boolean: None,
            cursor: None,
            phase: Phase::Resolve,
            visits: 0,
        }
    }
    /// Descend a supported subtree without following its new parent.
    pub(crate) fn subtree(g: &Graph, root: Root, variable: u64, scope: Condition) -> Self {
        let mut result = Self::new(g, root, variable, scope);
        result.resolver = None;
        result.pending.insert(variable, scope);
        result.queue.push_back(variable);
        result.phase = Phase::Next;
        result
    }
    pub fn root(&self) -> Root {
        self.root
    }
    /// Resolver variable visits, novel member visits, and reverse-index trie
    /// nodes inspected. Prefix seeking has bounded key-path overhead.
    pub fn visits(&self) -> u64 {
        self.visits + self.resolver.as_ref().map_or(0, Resolve::visits)
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        std::iter::once(self.fresh)
            .chain(self.pending.values().copied())
            .chain(self.visited.values().copied())
            .chain(self.boolean.iter().flat_map(Job::roots))
            .chain(self.resolver.iter().flat_map(Resolve::condition_roots))
    }
    fn enqueue(&mut self, a: &Arena, variable: u64, support: Condition) {
        self.target = variable;
        let old = self
            .pending
            .get(&variable)
            .copied()
            .unwrap_or(Condition::FALSE);
        self.boolean = Some(a.start(Operation::Or(old, support)));
        self.phase = Phase::Enqueue;
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
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.fresh = Condition::FALSE;
            self.queue = VecDeque::new();
            self.cursor = None;
        }
        match self.discard {
            1 => {
                if let Some(j) = &mut self.boolean {
                    if j.discard_tick() {
                        self.boolean = None;
                    }
                } else {
                    self.discard = 2;
                }
            }
            2 => {
                if let Some(r) = &mut self.resolver {
                    if r.discard_tick() {
                        self.resolver = None;
                    }
                } else {
                    self.discard = 3;
                }
            }
            3 => {
                if self.pending.pop_first().is_none() {
                    self.discard = 4;
                }
            }
            4 => {
                if self.visited.pop_first().is_none() {
                    self.discard = 5;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> ResolveStatus {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        match self.phase {
            Phase::Resolve => {
                let resolver = self.resolver.as_mut().expect("representative resolver");
                match resolver.tick(g, a) {
                    ResolveStatus::Found { variable, support } => {
                        self.enqueue(a, variable, support)
                    }
                    ResolveStatus::Done => {
                        self.visits += resolver.visits();
                        self.resolver = None;
                        self.phase = Phase::Next;
                    }
                    ResolveStatus::Pending => {}
                }
            }
            Phase::Enqueue => {
                if let Some(c) = self.poll(a) {
                    if self.pending.insert(self.target, c).is_none() {
                        self.queue.push_back(self.target);
                    }
                    self.phase = if self.resolver.is_some() {
                        Phase::Resolve
                    } else {
                        Phase::Scan
                    };
                }
            }
            Phase::Next => {
                if let Some(v) = self.queue.pop_front() {
                    self.variable = v;
                    let carry = self.pending.remove(&v).expect("queued member");
                    let old = self.visited.get(&v).copied().unwrap_or(Condition::FALSE);
                    self.boolean = Some(a.start(Operation::Difference(carry, old)));
                    self.phase = Phase::Novel;
                } else {
                    self.phase = Phase::Cleanup;
                }
            }
            Phase::Novel => {
                if let Some(c) = self.poll(a) {
                    self.fresh = c;
                    if c == Condition::FALSE {
                        self.phase = Phase::Next;
                    } else {
                        let old = self
                            .visited
                            .get(&self.variable)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(a.start(Operation::Or(old, c)));
                        self.phase = Phase::Remember;
                    }
                }
            }
            Phase::Remember => {
                if let Some(c) = self.poll(a) {
                    self.visited.insert(self.variable, c);
                    self.visits += 1;
                    self.cursor = Some(g.index.range(
                        self.root,
                        [CHILD, self.variable, 0, 0],
                        [CHILD, self.variable, u64::MAX, 0],
                    ));
                    self.phase = Phase::Scan;
                    return ResolveStatus::Found {
                        variable: self.variable,
                        support: self.fresh,
                    };
                }
            }
            Phase::Scan => {
                let cursor = self.cursor.as_mut().expect("child prefix cursor");
                let before = cursor.visits();
                let next = cursor.next(&g.index);
                self.visits += cursor.visits() - before;
                if let Some((key, support)) = next {
                    self.target = key[2];
                    // Each child intersects the full incoming support, independently.
                    self.boolean = Some(a.start(Operation::And(self.fresh, support)));
                    self.phase = Phase::Hit;
                } else {
                    self.cursor = None;
                    self.phase = Phase::Next;
                }
            }
            Phase::Hit => {
                if let Some(c) = self.poll(a) {
                    if c == Condition::FALSE {
                        self.phase = Phase::Scan;
                    } else {
                        self.enqueue(a, self.target, c);
                    }
                }
            }
            Phase::Cleanup => {
                if self.pending.pop_first().is_none() && self.visited.pop_first().is_none() {
                    self.queue = VecDeque::new();
                    self.fresh = Condition::FALSE;
                    self.phase = Phase::Done;
                    return ResolveStatus::Done;
                }
            }
            Phase::Done => return ResolveStatus::Done,
        }
        ResolveStatus::Pending
    }
}

impl Trace for Members {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.fresh]),
            1 => cursor.values(&self.pending),
            2 => cursor.values(&self.visited),
            3 => cursor.optional(self.boolean.as_ref()),
            4 => cursor.optional(self.resolver.as_ref()),
            _ => Step::Done,
        }
    }
}
