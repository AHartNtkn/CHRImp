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
            discarding: false,
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

#[derive(Clone, Copy)]
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
    discarding: bool,
}

impl Job {
    pub fn result(&self) -> Option<Condition> {
        if !self.discarding && self.frames.is_empty() && self.memo.is_empty() {
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

    /// Cancel without evaluating another subproblem. Frames contain only Copy
    /// scalars, so their backing can be released directly; drain at most one
    /// B-tree memo entry per call. Roots remain traceable throughout discard.
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.frames = Vec::new();
        self.last = None;
        self.memo.pop_first();
        self.memo.is_empty()
    }

    pub fn tick(&mut self, arena: &mut Arena) -> Progress {
        assert!(!self.discarding, "condition job has been discarded");
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

/// A budgeted simultaneous substitution or existential suffix projection.
/// Causal ownership, rather than this Boolean operation, determines which
/// choices the executor may forget. Images are not themselves transformed.
pub struct Transform {
    owner: u32,
    images: Option<Arc<BTreeMap<u64, Condition>>>,
    cutoff: Option<u64>,
    draining: BTreeMap<u64, Condition>,
    frames: Vec<TransformFrame>,
    memo: BTreeMap<Condition, Condition>,
    last: Option<Condition>,
    job: Option<Job>,
    work: u64,
    discarding: bool,
}
#[derive(Clone, Copy)]
enum TransformFrame {
    Evaluate(Condition),
    Finish(Condition),
    Low {
        input: Condition,
        image: Condition,
        high: Condition,
    },
    High {
        input: Condition,
        image: Condition,
        low: Condition,
    },
    Product {
        input: Condition,
        image: Condition,
        high: Condition,
    },
    Union {
        input: Condition,
        left: Condition,
    },
}
impl Arena {
    /// Substitute referenced choice images simultaneously. Validate the key
    /// bound now and each referenced image on use, without scanning the map.
    pub fn substitute(&self, input: Condition, images: Arc<BTreeMap<u64, Condition>>) -> Transform {
        assert!(self.contains(input), "stale or foreign condition operand");
        assert!(
            images
                .last_key_value()
                .is_none_or(|(&choice, _)| choice < self.next_choice),
            "unknown substitution choice"
        );
        let empty = images.is_empty();
        Transform::new(self.owner, input, (!empty).then_some(images), None, empty)
    }
    /// Existentially quantify all choice IDs greater than or equal to cutoff.
    pub fn project_before(&self, input: Condition, cutoff: u64) -> Transform {
        assert!(self.contains(input), "stale or foreign condition operand");
        Transform::new(
            self.owner,
            input,
            None,
            Some(cutoff),
            cutoff >= self.next_choice,
        )
    }
}
impl Transform {
    fn new(
        owner: u32,
        input: Condition,
        images: Option<Arc<BTreeMap<u64, Condition>>>,
        cutoff: Option<u64>,
        unchanged: bool,
    ) -> Self {
        Self {
            owner,
            images,
            cutoff,
            draining: BTreeMap::new(),
            frames: if unchanged || input.is_terminal() {
                vec![]
            } else {
                vec![TransformFrame::Evaluate(input)]
            },
            memo: BTreeMap::new(),
            last: (unchanged || input.is_terminal()).then_some(input),
            job: None,
            work: 0,
            discarding: false,
        }
    }
    pub fn result(&self) -> Option<Condition> {
        if !self.discarding
            && self.frames.is_empty()
            && self.memo.is_empty()
            && self.images.is_none()
            && self.draining.is_empty()
            && self.job.is_none()
        {
            self.last
        } else {
            None
        }
    }
    /// Evaluation actions, including nested Boolean work, excluding cleanup.
    pub fn work(&self) -> u64 {
        self.work
    }
    /// Frame capacity and live memo/image entries, not allocation bytes.
    pub fn scratch_capacity(&self) -> usize {
        self.frames.capacity()
            + self.memo.len()
            + self.draining.len()
            + self.images.as_ref().map_or(0, |b| b.len())
            + self.job.as_ref().map_or(0, Job::scratch_capacity)
    }
    pub fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.memo
            .iter()
            .flat_map(|(&input, &result)| [input, result])
            .chain(self.last)
            .chain(
                self.frames
                    .iter()
                    .flat_map(|frame| frame.roots().into_iter().flatten()),
            )
            .chain(
                self.images
                    .iter()
                    .flat_map(|images| images.values().copied()),
            )
            .chain(self.draining.values().copied())
            .chain(self.job.iter().flat_map(Job::roots))
    }
    fn cleanup_tick(&mut self) -> bool {
        if self.memo.pop_first().is_some() {
            return false;
        }
        if let Some(images) = self.images.take() {
            // Only the last map owner drains; a shared owner retains its roots.
            if let Some(images) = Arc::into_inner(images) {
                self.draining = images;
            }
            return false;
        }
        self.draining.pop_first();
        self.draining.is_empty()
    }
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.frames = Vec::new();
        self.last = None;
        if let Some(job) = self.job.as_mut() {
            if job.discard_tick() {
                self.job = None;
            }
            return false;
        }
        self.cleanup_tick()
    }
    pub fn tick(&mut self, arena: &mut Arena) -> Progress {
        assert!(!self.discarding, "condition transform has been discarded");
        assert_eq!(self.owner, arena.owner, "foreign condition transform");
        GcLease::assert_mutable(&arena.frozen);
        if let Some(result) = self.result() {
            return Progress::Complete(result);
        }
        if let Some(job) = self.job.as_mut() {
            let before = job.work();
            let progress = job.tick(arena);
            self.work += job.work() - before;
            if let Progress::Complete(result) = progress {
                self.last = Some(result);
                self.job = None;
            }
        } else if self.frames.is_empty() {
            self.cleanup_tick();
        } else {
            self.work += 1;
            match self.frames.pop().unwrap() {
                TransformFrame::Evaluate(input) => {
                    if let Some(&result) = self.memo.get(&input) {
                        self.last = Some(result);
                    } else {
                        match arena.view(input) {
                            View::Terminal(_) => self.last = Some(input),
                            View::Choice { choice, low, high } => {
                                if self.cutoff.is_some_and(|cutoff| choice >= cutoff) {
                                    // Ordered descendants are all quantified. Every
                                    // canonical nonterminal has a satisfying path.
                                    self.last = Some(Condition::TRUE);
                                } else if self.images.as_ref().is_some_and(|images| {
                                    choice > *images.last_key_value().unwrap().0
                                }) {
                                    self.last = Some(input);
                                } else {
                                    let image = self
                                        .images
                                        .as_ref()
                                        .and_then(|images| images.get(&choice))
                                        .copied()
                                        .unwrap_or_else(|| {
                                            arena.node(choice, Condition::FALSE, Condition::TRUE)
                                        });
                                    assert!(
                                        arena.contains(image),
                                        "stale or foreign substitution image"
                                    );
                                    if image.is_terminal() {
                                        self.frames.push(TransformFrame::Finish(input));
                                        self.frames.push(TransformFrame::Evaluate(
                                            if image == Condition::TRUE { high } else { low },
                                        ));
                                    } else {
                                        self.frames.push(TransformFrame::Low {
                                            input,
                                            image,
                                            high,
                                        });
                                        self.frames.push(TransformFrame::Evaluate(low));
                                    }
                                    self.last = None;
                                }
                            }
                        }
                    }
                }
                TransformFrame::Finish(input) => {
                    self.memo
                        .insert(input, self.last.expect("completed cofactor"));
                }
                TransformFrame::Low { input, image, high } => {
                    let low = self.last.take().expect("low cofactor");
                    self.frames.push(TransformFrame::High { input, image, low });
                    self.frames.push(TransformFrame::Evaluate(high));
                }
                TransformFrame::High { input, image, low } => {
                    let high = self.last.take().expect("high cofactor");
                    if low == high {
                        self.last = Some(low);
                        self.memo.insert(input, low);
                    } else if low == Condition::FALSE && high == Condition::TRUE {
                        self.last = Some(image);
                        self.memo.insert(input, image);
                    } else if low == Condition::TRUE && high == Condition::FALSE {
                        self.last = Some(image.not());
                        self.memo.insert(input, image.not());
                    } else {
                        // A literal above both children can be rebuilt directly.
                        // Otherwise ITE must reorder even an unchanged parent:
                        // a descendant image may have introduced older choices.
                        let direct = match arena.view(image) {
                            View::Choice {
                                choice,
                                low: il,
                                high: ih,
                            } if il.is_terminal() && ih.is_terminal() => {
                                let later = |c| match arena.view(c) {
                                    View::Terminal(_) => true,
                                    View::Choice { choice: child, .. } => child > choice,
                                };
                                (later(low) && later(high))
                                    .then_some((choice, il == Condition::FALSE))
                            }
                            _ => None,
                        };
                        if let Some((choice, positive)) = direct {
                            let result = if positive {
                                arena.node(choice, low, high)
                            } else {
                                arena.node(choice, high, low)
                            };
                            self.last = Some(result);
                            self.memo.insert(input, result);
                        } else {
                            self.frames
                                .push(TransformFrame::Product { input, image, high });
                            self.job = Some(arena.start(Operation::And(image.not(), low)));
                        }
                    }
                }
                TransformFrame::Product { input, image, high } => {
                    let left = self.last.take().expect("low product");
                    self.frames.push(TransformFrame::Union { input, left });
                    self.job = Some(arena.start(Operation::And(image, high)));
                }
                TransformFrame::Union { input, left } => {
                    let right = self.last.take().expect("high product");
                    self.frames.push(TransformFrame::Finish(input));
                    self.job = Some(arena.start(Operation::Or(left, right)));
                }
            }
            if self.frames.is_empty() {
                self.frames = Vec::new();
            }
        }
        self.result().map_or(Progress::Pending, Progress::Complete)
    }
}
impl TransformFrame {
    fn roots(self) -> [Option<Condition>; 3] {
        match self {
            Self::Evaluate(input) | Self::Finish(input) => [Some(input), None, None],
            Self::Low { input, image, high } | Self::Product { input, image, high } => {
                [Some(input), Some(image), Some(high)]
            }
            Self::High { input, image, low } => [Some(input), Some(image), Some(low)],
            Self::Union { input, left } => [Some(input), Some(left), None],
        }
    }
}
impl Trace for TransformFrame {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        if cursor.phase != 0 {
            return Step::Done;
        }
        let roots = self.roots();
        let fields = roots.map(|root| root.unwrap_or(Condition::FALSE));
        cursor.fields(&fields[..roots.iter().flatten().count()])
    }
}
impl Trace for Transform {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        match cursor.phase {
            0 => cursor.substitutions(&self.memo),
            1 => cursor.fields(self.last.as_slice()),
            2 => cursor.vector(self.frames.len(), |i, child| self.frames[i].trace(child)),
            3 => match &self.images {
                Some(images) => cursor.values(images),
                None => cursor.advance(),
            },
            4 => cursor.values(&self.draining),
            5 => cursor.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}
