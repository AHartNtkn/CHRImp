//! Canonical Boolean conditions on explicit choices.
//!
//! Negation is a complemented edge, not a traversal. General operations and
//! collection expose one diagram action per tick. Map operations and
//! allocation have their ordinary size-dependent costs, not real-time bounds.
//! Conditions denote sets; causal choice births and answer multiplicity belong
//! to the executor, never to Boolean simplification.

use crate::gc::GcLease;
use crate::trace::{Cursor, Step, Trace};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static NEXT_ARENA: AtomicU32 = AtomicU32::new(1);
const CACHE_LIMIT: usize = 1024;

/// An arena-owned identity, never reused. Terminals are universal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Condition {
    owner: u32,
    id: u64,
    negative: bool,
}

impl Condition {
    pub const FALSE: Self = Self {
        owner: 0,
        id: 0,
        negative: false,
    };
    pub const TRUE: Self = Self {
        negative: true,
        ..Self::FALSE
    };

    pub const fn not(self) -> Self {
        Self {
            negative: !self.negative,
            ..self
        }
    }
    pub const fn is_terminal(self) -> bool {
        self.owner == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeKey {
    choice: u64,
    low: Condition,
    high: Condition,
}

struct Node {
    key: NodeKey,
    marked: u64,
}

/// One node of the represented function, with complemented edges resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Terminal(bool),
    Choice {
        choice: u64,
        low: Condition,
        high: Condition,
    },
}

type Pair = (Condition, Condition);

pub struct Arena {
    owner: u32,
    nodes: BTreeMap<u64, Node>,
    next_node: u64,
    unique: HashMap<NodeKey, Condition>,
    cache: HashMap<Pair, Condition>,
    cache_order: VecDeque<Pair>,
    next_choice: u64,
    epoch: u64,
    frozen: Arc<AtomicBool>,
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            owner: NEXT_ARENA
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("condition arena identity exhausted"),
            nodes: BTreeMap::new(),
            next_node: 0,
            unique: HashMap::new(),
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            next_choice: 0,
            epoch: 0,
            frozen: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Arena {
    pub fn fresh_choice(&mut self) -> (u64, Condition) {
        GcLease::assert_mutable(&self.frozen);
        let choice = self.next_choice;
        self.next_choice = choice.checked_add(1).expect("choice identity exhausted");
        (choice, self.node(choice, Condition::FALSE, Condition::TRUE))
    }

    pub fn contains(&self, condition: Condition) -> bool {
        condition.is_terminal()
            || (condition.owner == self.owner && self.nodes.contains_key(&condition.id))
    }

    pub fn view(&self, condition: Condition) -> View {
        assert!(
            self.contains(condition),
            "stale or foreign condition handle"
        );
        if condition.is_terminal() {
            return View::Terminal(condition.negative);
        }
        let node = self.nodes[&condition.id].key;
        let edge = |c: Condition| if condition.negative { c.not() } else { c };
        View::Choice {
            choice: node.choice,
            low: edge(node.low),
            high: edge(node.high),
        }
    }

    /// Convenience evaluation for tests/owned finite observations. Production
    /// projections use `view` to yield between nodes of large diagrams.
    pub fn evaluate(
        &self,
        mut condition: Condition,
        mut assignment: impl FnMut(u64) -> bool,
    ) -> bool {
        loop {
            match self.view(condition) {
                View::Terminal(value) => return value,
                View::Choice { choice, low, high } => {
                    condition = if assignment(choice) { high } else { low }
                }
            }
        }
    }

    pub fn node_count(&self) -> usize {
        self.unique.len()
    }
    /// Capacity of the weak canonical table, rebuilt during collection.
    pub fn unique_capacity(&self) -> usize {
        self.unique.capacity()
    }
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    fn node(&mut self, choice: u64, mut low: Condition, mut high: Condition) -> Condition {
        if low == high {
            return low;
        }
        let negative = low.negative;
        if negative {
            low = low.not();
            high = high.not();
        }
        let key = NodeKey { choice, low, high };
        let base = if let Some(&found) = self.unique.get(&key) {
            found
        } else {
            let id = self.next_node;
            self.next_node = id.checked_add(1).expect("condition identity exhausted");
            self.nodes.insert(id, Node { key, marked: 0 });
            let handle = Condition {
                owner: self.owner,
                id,
                negative: false,
            };
            self.unique.insert(key, handle);
            handle
        };
        if negative { base.not() } else { base }
    }

    fn cached(&self, pair: Pair) -> Option<Condition> {
        simple(pair).or_else(|| self.cache.get(&pair).copied())
    }

    fn remember(&mut self, pair: Pair, result: Condition) {
        if self.cache.contains_key(&pair) {
            return;
        }
        if self.cache.len() == CACHE_LIMIT {
            self.cache
                .remove(&self.cache_order.pop_front().expect("cache order"));
        }
        self.cache.insert(pair, result);
        self.cache_order.push_back(pair);
    }

    pub fn start(&self, operation: Operation) -> Job {
        let (a, b, negative) = match operation {
            Operation::And(a, b) => (a, b, false),
            Operation::Or(a, b) => (a.not(), b.not(), true),
            Operation::Difference(a, b) => (a, b.not(), false),
        };
        assert!(
            self.contains(a) && self.contains(b),
            "stale or foreign condition operand"
        );
        let pair = ordered(a, b);
        let known = self.cached(pair);
        Job {
            owner: self.owner,
            negative,
            frames: if known.is_some() {
                Vec::new()
            } else {
                vec![Frame::Evaluate(pair)]
            },
            last: known,
            memo: BTreeMap::new(),
            work: 0,
        }
    }

    /// Stop-the-mutator collection, resumable between structural actions. The
    /// caller supplies every semantic root, including `Job::roots()` for all
    /// suspended jobs. Unique tables and operation caches are deliberately weak.
    /// The arena remains read-only until the owned collector is dropped,
    /// including after completion. Dropping it early aborts collection.
    pub fn collect<I: Iterator<Item = Condition>>(&mut self, roots: I) -> Collector<I> {
        let lease = GcLease::acquire(&self.frozen);
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("collection epoch exhausted");
        Collector {
            owner: self.owner,
            epoch: self.epoch,
            _lease: lease,
            roots,
            pending: Vec::new(),
            phase: Phase::Cache,
            sweep: None,
            unique: HashMap::new(),
        }
    }
}

fn ordered(a: Condition, b: Condition) -> Pair {
    if a <= b { (a, b) } else { (b, a) }
}

fn simple((a, b): Pair) -> Option<Condition> {
    if a == Condition::FALSE || b == Condition::FALSE || a == b.not() {
        Some(Condition::FALSE)
    } else if a == Condition::TRUE {
        Some(b)
    } else if b == Condition::TRUE || a == b {
        Some(a)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Operation {
    And(Condition, Condition),
    Or(Condition, Condition),
    Difference(Condition, Condition),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete(Condition),
}

enum Frame {
    Evaluate(Pair),
    AfterLow {
        pair: Pair,
        choice: u64,
        high: Pair,
    },
    AfterHigh {
        pair: Pair,
        choice: u64,
        low: Condition,
    },
}

pub struct Job {
    owner: u32,
    negative: bool,
    frames: Vec<Frame>,
    last: Option<Condition>,
    // Completed subproblems are semantic work dependencies, not an evicting cache.
    memo: BTreeMap<Pair, Condition>,
    work: u64,
}

impl Job {
    pub fn result(&self) -> Option<Condition> {
        if self.frames.is_empty() && self.memo.is_empty() {
            self.last.map(|c| if self.negative { c.not() } else { c })
        } else {
            None
        }
    }
    /// Boolean evaluation actions, excluding incremental scratch cleanup.
    pub fn work(&self) -> u64 {
        self.work
    }
    /// Frame-vector capacity plus live memo entries (not B-tree allocation bytes).
    pub fn scratch_capacity(&self) -> usize {
        self.frames.capacity() + self.memo.len()
    }

    pub fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.memo
            .iter()
            .flat_map(|(&(a, b), &result)| [a, b, result])
            .chain(
                self.last
                    .into_iter()
                    .chain(self.frames.iter().flat_map(|frame| {
                        let mut roots = [None; 4];
                        match *frame {
                            Frame::Evaluate((a, b)) => {
                                roots[0] = Some(a);
                                roots[1] = Some(b);
                            }
                            Frame::AfterLow {
                                pair: (a, b),
                                high: (c, d),
                                ..
                            } => {
                                roots = [Some(a), Some(b), Some(c), Some(d)];
                            }
                            Frame::AfterHigh {
                                pair: (a, b), low, ..
                            } => {
                                roots = [Some(a), Some(b), Some(low), None];
                            }
                        }
                        roots.into_iter().flatten()
                    })),
            )
    }

    pub fn tick(&mut self, arena: &mut Arena) -> Progress {
        assert_eq!(self.owner, arena.owner, "foreign condition job");
        GcLease::assert_mutable(&arena.frozen);
        if let Some(result) = self.result() {
            return Progress::Complete(result);
        }
        if self.frames.is_empty() {
            self.memo.pop_first();
            if let Some(result) = self.result() {
                self.frames = Vec::new();
                return Progress::Complete(result);
            }
            return Progress::Pending;
        }
        self.work += 1;
        match self.frames.pop().expect("unfinished condition operation") {
            Frame::Evaluate(pair) => {
                if let Some(result) = simple(pair) {
                    self.last = Some(result);
                } else if let Some(result) = self
                    .memo
                    .get(&pair)
                    .copied()
                    .or_else(|| arena.cache.get(&pair).copied())
                {
                    self.memo.insert(pair, result);
                    self.last = Some(result);
                } else {
                    let a = arena.view(pair.0);
                    let b = arena.view(pair.1);
                    let choice = match (a, b) {
                        (View::Choice { choice: a, .. }, View::Choice { choice: b, .. }) => {
                            a.min(b)
                        }
                        (View::Choice { choice, .. }, _) | (_, View::Choice { choice, .. }) => {
                            choice
                        }
                        _ => unreachable!("terminal cases simplified"),
                    };
                    let split = |original, view| match view {
                        View::Choice {
                            choice: c,
                            low,
                            high,
                        } if c == choice => (low, high),
                        _ => (original, original),
                    };
                    let (al, ah) = split(pair.0, a);
                    let (bl, bh) = split(pair.1, b);
                    self.frames.push(Frame::AfterLow {
                        pair,
                        choice,
                        high: ordered(ah, bh),
                    });
                    self.frames.push(Frame::Evaluate(ordered(al, bl)));
                    self.last = None;
                }
            }
            Frame::AfterLow { pair, choice, high } => {
                let low = self.last.take().expect("low cofactor completed");
                self.frames.push(Frame::AfterHigh { pair, choice, low });
                self.frames.push(Frame::Evaluate(high));
            }
            Frame::AfterHigh { pair, choice, low } => {
                let high = self.last.take().expect("high cofactor completed");
                let result = arena.node(choice, low, high);
                self.memo.insert(pair, result);
                arena.remember(pair, result);
                self.last = Some(result);
            }
        }
        if let Some(result) = self.result() {
            self.frames = Vec::new();
            Progress::Complete(result)
        } else {
            Progress::Pending
        }
    }
}

enum Phase {
    Cache,
    Mark,
    Sweep,
    Done,
}

pub struct Collector<I> {
    owner: u32,
    epoch: u64,
    _lease: GcLease,
    roots: I,
    pending: Vec<Condition>,
    phase: Phase,
    sweep: Option<u64>,
    unique: HashMap<NodeKey, Condition>,
}

impl<I: Iterator<Item = Condition>> Collector<I> {
    /// Returns true when finished. Each call removes at most one weak cache
    /// entry, marks one node/reads one root, or sweeps one allocated node.
    pub fn tick(&mut self, arena: &mut Arena) -> bool {
        assert_eq!(self.owner, arena.owner, "foreign condition collector");
        assert_eq!(self.epoch, arena.epoch, "stale condition collector");
        match self.phase {
            Phase::Cache => {
                if let Some(pair) = arena.cache_order.pop_front() {
                    arena.cache.remove(&pair);
                } else {
                    self.phase = Phase::Mark;
                }
            }
            Phase::Mark => {
                if let Some(root) = self.pending.pop().or_else(|| self.roots.next()) {
                    assert!(arena.contains(root), "stale or foreign collection root");
                    if !root.is_terminal() {
                        let node = arena.nodes.get_mut(&root.id).expect("live root");
                        if node.marked != self.epoch {
                            node.marked = self.epoch;
                            self.pending.extend([node.key.low, node.key.high]);
                        }
                    }
                } else {
                    self.phase = Phase::Sweep;
                }
            }
            Phase::Sweep => {
                let next = match self.sweep {
                    Some(id) => arena.nodes.range((Excluded(id), Unbounded)).next(),
                    None => arena.nodes.first_key_value(),
                };
                if let Some((&id, node)) = next {
                    let key = node.key;
                    if node.marked != self.epoch {
                        arena.nodes.remove(&id);
                        arena.unique.remove(&key);
                    } else {
                        self.unique.insert(
                            key,
                            Condition {
                                owner: self.owner,
                                id,
                                negative: false,
                            },
                        );
                    }
                    self.sweep = Some(id);
                } else {
                    // Rebuilding incrementally releases peak hash-table storage.
                    // The old table remains valid if a collector is dropped early.
                    arena.unique = std::mem::take(&mut self.unique);
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => return true,
        }
        matches!(self.phase, Phase::Done)
    }
}

impl Trace for Frame {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        if cursor.phase != 0 {
            return Step::Done;
        }
        match *self {
            Self::Evaluate((a, b)) => cursor.fields(&[a, b]),
            Self::AfterLow {
                pair: (a, b),
                high: (c, d),
                ..
            } => cursor.fields(&[a, b, c, d]),
            Self::AfterHigh {
                pair: (a, b), low, ..
            } => cursor.fields(&[a, b, low]),
        }
    }
}
impl Trace for Job {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        match cursor.phase {
            0 => cursor.pairs(&self.memo),
            1 => cursor.fields(self.last.as_slice()),
            2 => cursor.vector(self.frames.len(), |i, child| self.frames[i].trace(child)),
            _ => Step::Done,
        }
    }
}
