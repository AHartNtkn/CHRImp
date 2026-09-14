//! Canonical Boolean conditions on explicit choices.
//!
//! Negation is a complemented edge, not a traversal. General operations and
//! collection expose bounded diagram actions per tick. Map operations and
//! allocation have their ordinary size-dependent costs, not real-time bounds.
//! Dense results can trigger bounded, resumable variable sifting. Adjacent
//! swaps preserve semantic handles by changing only their decompositions.
//! Choice birth IDs remain chronological, independently of diagram order.
//! Conditions denote sets; causal choice births and answer multiplicity belong
//! to the executor, never to Boolean simplification.

use crate::gc::{GcLease, discard_slot};
use crate::trace::{Cursor, Step, Trace};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

pub(crate) fn poll(job: &mut Option<Job>, arena: &mut Arena) -> Option<Condition> {
    match job.as_mut().expect("pending Boolean operation").tick(arena) {
        Progress::Pending => None,
        Progress::Complete(result) => {
            *job = None;
            Some(result)
        }
    }
}

/// Drain one substitution scratch step, leaving shared images with their owner.
pub(crate) fn cleanup_substitution(
    memo: &mut BTreeMap<Condition, Condition>,
    images: &mut Option<Arc<BTreeMap<u64, Condition>>>,
    draining: &mut BTreeMap<u64, Condition>,
) -> bool {
    if memo.pop_first().is_some() {
        return false;
    }
    if let Some(images) = images.take() {
        if let Some(images) = Arc::into_inner(images) {
            *draining = images;
        }
        return false;
    }
    draining.pop_first();
    draining.is_empty()
}

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

#[derive(Default)]
struct ChoiceOrder {
    rank: u64,
    after: Option<u64>,
    nodes: BTreeSet<u64>,
}

struct Node {
    key: NodeKey,
    marked: u64,
    references: usize,
    // Working references are initialized lazily from the protected closure.
    references_epoch: u64,
    archived: u64,
    archive_references: usize,
    max_choice: u64,
}

// Monotonic IDs address an append-only epoch of eight payloads. Only live
// epochs have directory entries; links skip reclaimed epochs. A collector saves
// its successor BEFORE retiring a node, so no tombstone is needed for a cursor.
const NODE_BLOCK: usize = 8;
struct NodeBlock {
    nodes: [Option<Node>; NODE_BLOCK],
    live: usize,
    previous: Option<u64>,
    next: Option<u64>,
}
#[derive(Default)]
struct NodeSlab {
    blocks: HashMap<u64, Box<NodeBlock>>,
    first: Option<u64>,
    last: Option<u64>,
    #[cfg(feature = "diagnostics")]
    accesses: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    moves: usize,
    #[cfg(feature = "diagnostics")]
    reclaimed: usize,
    #[cfg(feature = "diagnostics")]
    directory_lookups: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    chronology_scans: AtomicUsize,
}
impl NodeSlab {
    fn block(&self, epoch: &u64) -> Option<&NodeBlock> {
        #[cfg(feature = "diagnostics")]
        self.directory_lookups.fetch_add(1, Ordering::Relaxed);
        self.blocks.get(epoch).map(Box::as_ref)
    }
    fn block_mut(&mut self, epoch: &u64) -> Option<&mut NodeBlock> {
        #[cfg(feature = "diagnostics")]
        self.directory_lookups.fetch_add(1, Ordering::Relaxed);
        self.blocks.get_mut(epoch).map(Box::as_mut)
    }
    fn accessed(&self) {
        #[cfg(feature = "diagnostics")]
        self.accesses.fetch_add(1, Ordering::Relaxed);
    }
    fn get(&self, id: &u64) -> Option<&Node> {
        self.accessed();
        self.block(&(*id / NODE_BLOCK as u64))?.nodes[*id as usize % NODE_BLOCK].as_ref()
    }
    fn get_mut(&mut self, id: &u64) -> Option<&mut Node> {
        self.accessed();
        self.block_mut(&(*id / NODE_BLOCK as u64))?.nodes[*id as usize % NODE_BLOCK].as_mut()
    }
    fn contains_key(&self, id: &u64) -> bool {
        self.get(id).is_some()
    }
    fn allocate(&mut self, serial: u64, node: Node) -> u64 {
        let epoch = serial / NODE_BLOCK as u64;
        if self.block(&epoch).is_none() {
            if let Some(last) = self.last {
                self.block_mut(&last).unwrap().next = Some(epoch);
            } else {
                self.first = Some(epoch);
            }
            self.blocks.insert(
                epoch,
                Box::new(NodeBlock {
                    nodes: std::array::from_fn(|_| None),
                    live: 0,
                    previous: self.last,
                    next: None,
                }),
            );
            self.last = Some(epoch);
        }
        let block = self.block_mut(&epoch).unwrap();
        let slot = &mut block.nodes[serial as usize % NODE_BLOCK];
        assert!(slot.is_none(), "condition identity reused");
        *slot = Some(node);
        block.live += 1;
        serial
    }
    fn remove(&mut self, id: &u64) -> Option<Node> {
        self.accessed();
        let epoch = *id / NODE_BLOCK as u64;
        let block = self.block_mut(&epoch)?;
        let node = block.nodes[*id as usize % NODE_BLOCK].take()?;
        block.live -= 1;
        if block.live == 0 {
            let (previous, next) = (block.previous, block.next);
            self.blocks.remove(&epoch);
            if let Some(previous) = previous {
                self.block_mut(&previous).unwrap().next = next;
            } else {
                self.first = next;
            }
            if let Some(next) = next {
                self.block_mut(&next).unwrap().previous = previous;
            } else {
                self.last = previous;
            }
            #[cfg(feature = "diagnostics")]
            {
                self.reclaimed += 1;
            }
        }
        Some(node)
    }
    // An exclusive cursor must still name a live node. Sweep keeps the returned
    // successor in Collector before remove; reset/unique never remove payloads.
    // Scan at most two blocks, with at most eight positions in either.
    fn next(&self, cursor: Option<u64>) -> Option<(u64, &Node)> {
        let (epoch, offset) = match cursor {
            Some(id) => (id / NODE_BLOCK as u64, id as usize % NODE_BLOCK + 1),
            None => (self.first?, 0),
        };
        let block = self.block(&epoch).expect("live cursor epoch");
        for i in offset..NODE_BLOCK {
            #[cfg(feature = "diagnostics")]
            self.chronology_scans.fetch_add(1, Ordering::Relaxed);
            if let Some(node) = &block.nodes[i] {
                return Some((epoch * NODE_BLOCK as u64 + i as u64, node));
            }
        }
        let epoch = block.next?;
        let block = self.block(&epoch).expect("live successor epoch");
        let (offset, node) = block
            .nodes
            .iter()
            .enumerate()
            .find_map(|(i, node)| {
                #[cfg(feature = "diagnostics")]
                self.chronology_scans.fetch_add(1, Ordering::Relaxed);
                node.as_ref().map(|node| (i, node))
            })
            .expect("linked epoch is nonempty");
        Some((epoch * NODE_BLOCK as u64 + offset as u64, node))
    }
    fn compact_tick(&mut self) -> bool {
        if self.blocks.capacity() > self.blocks.len().saturating_mul(2) {
            #[cfg(feature = "diagnostics")]
            {
                self.moves += self.blocks.len();
            }
            self.blocks.shrink_to_fit();
        }
        true
    }
}

impl std::ops::Index<&u64> for NodeSlab {
    type Output = Node;
    fn index(&self, id: &u64) -> &Node {
        self.get(id).expect("live condition")
    }
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
    nodes: NodeSlab,
    // Derived sweep candidates only; nodes remains the payload authority.
    // None is the ordinary path, restored incrementally on final release.
    unprotected: Option<BTreeSet<u64>>,
    archive_epoch: u64,
    archive_branching: usize,
    // External archive roots own their Boolean closure. Counts on nodes include
    // these roots and one edge from each protected parent, irrespective of how
    // many snapshots share that parent. Pending edge updates survive an aborted
    // collector; working roots are still traced separately each collection.
    archive_roots: BTreeMap<Condition, usize>,
    archive_pending: Vec<(Condition, usize, bool)>,
    archive_order_epoch: u64,
    archive_rebuild: bool,
    next_node: u64,
    unique: HashMap<NodeKey, Condition>,
    cache: HashMap<Pair, Condition>,
    cache_order: VecDeque<Pair>,
    next_choice: u64,
    order: BTreeMap<u64, ChoiceOrder>,
    ranks: BTreeMap<u64, u64>,
    sift: Option<Sift>,
    order_epoch: u64,
    order_readers: Arc<AtomicUsize>,
    count_edges: bool,
    reorder_after: u64,
    epoch: u64,
    frozen: Arc<AtomicBool>,
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            owner: NEXT_ARENA
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("condition arena identity exhausted"),
            nodes: NodeSlab::default(),
            unprotected: None,
            archive_epoch: 1,
            archive_branching: 0,
            archive_roots: BTreeMap::new(),
            archive_pending: Vec::new(),
            archive_order_epoch: 0,
            archive_rebuild: false,
            next_node: 0,
            unique: HashMap::new(),
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            next_choice: 0,
            order: BTreeMap::new(),
            ranks: BTreeMap::new(),
            sift: None,
            order_epoch: 0,
            order_readers: Arc::new(AtomicUsize::new(0)),
            count_edges: false,
            reorder_after: 256,
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

    /// Preserve causal predecessors when an executor births a scoped decision.
    /// The support maximum is a conservative prefix bound, requiring neither
    /// a support traversal nor ownership of historical scope conditions.
    pub(crate) fn fresh_scoped_choice(&mut self, scope: Condition) -> (u64, Condition) {
        assert!(self.contains(scope), "stale or foreign choice scope");
        let after = self.support_max(scope);
        let (choice, condition) = self.fresh_choice();
        self.order.get_mut(&choice).unwrap().after = after;
        (choice, condition)
    }

    fn support_max(&self, condition: Condition) -> Option<u64> {
        (!condition.is_terminal()).then(|| self.nodes[&condition.id].max_choice)
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

    pub(crate) fn representation_epoch(&self) -> u64 {
        self.order_epoch
    }
    pub fn node_count(&self) -> usize {
        self.unique.len()
    }
    /// Capacity of the weak canonical table, rebuilt during collection.
    pub fn unique_capacity(&self) -> usize {
        self.unique.capacity()
    }
    /// Epoch payload and directory gauges; see docs/performance.md.
    #[cfg(feature = "diagnostics")]
    pub fn slab_diagnostics(&self) -> [usize; 18] {
        let occupied = self.nodes.blocks.len();
        let capacity = self.nodes.blocks.capacity();
        // HashMap requested backing estimate: buckets plus control bytes and
        // trailing SIMD group. Whole-process allocator is the byte authority.
        let directory = if capacity == 0 {
            0
        } else {
            capacity.next_power_of_two() * (std::mem::size_of::<(u64, Box<NodeBlock>)>() + 1) + 16
        };
        let leases = usize::from(self.frozen.load(Ordering::Relaxed));
        [
            self.node_count(),
            occupied * NODE_BLOCK - self.node_count(),
            occupied,
            capacity,
            occupied * std::mem::size_of::<NodeBlock>(),
            self.nodes.accesses.load(Ordering::Relaxed),
            self.nodes.moves,
            self.next_node as usize,
            directory,
            occupied,
            self.nodes.reclaimed,
            self.nodes.accesses.load(Ordering::Relaxed),
            occupied,
            leases,
            leases * std::mem::size_of::<Option<u64>>(),
            occupied * 2 * std::mem::size_of::<Option<u64>>(),
            self.nodes.directory_lookups.load(Ordering::Relaxed),
            self.nodes.chronology_scans.load(Ordering::Relaxed),
        ]
    }
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    // Birth IDs stay immutable; only these representation ranks can move.
    fn precedes(&self, a: u64, b: u64) -> bool {
        self.order[&a].rank < self.order[&b].rank
    }

    fn sift_tick(&mut self) -> bool {
        let Some(mut sift) = self.sift.take() else {
            return false;
        };
        if sift.tick(self) {
            self.reorder_after = self
                .next_node
                .saturating_add(256.max(sift.best_size as u64 * 2));
            if sift.changed {
                self.order_epoch = self
                    .order_epoch
                    .checked_add(1)
                    .expect("order epoch exhausted");
            }
        } else {
            self.sift = Some(sift);
        }
        true
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
            let serial = self.next_node;
            self.next_node = serial.checked_add(1).expect("condition identity exhausted");
            let id = self.nodes.allocate(
                serial,
                Node {
                    key,
                    marked: 0,
                    references: 0,
                    references_epoch: 0,
                    archived: 0,
                    archive_references: 0,
                    max_choice: choice
                        .max(self.support_max(low).unwrap_or(choice))
                        .max(self.support_max(high).unwrap_or(choice)),
                },
            );
            if let Some(ids) = &mut self.unprotected {
                ids.insert(id);
            }
            let order = self.order.entry(choice).or_insert_with(|| {
                self.ranks.insert(choice, choice);
                ChoiceOrder {
                    rank: choice,
                    after: None,
                    nodes: BTreeSet::new(),
                }
            });
            order.nodes.insert(id);
            if self.count_edges {
                Sift::retain(self, low);
                Sift::retain(self, high);
            }
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

    fn operands(&self, operation: Operation) -> (Pair, bool) {
        let (a, b, negative) = match operation {
            Operation::And(a, b) => (a, b, false),
            Operation::Or(a, b) => (a.not(), b.not(), true),
            Operation::Difference(a, b) => (a, b.not(), false),
        };
        assert!(
            self.contains(a) && self.contains(b),
            "stale or foreign condition operand"
        );
        (ordered(a, b), negative)
    }

    /// Exact identities on validated operands; no job or operation cache.
    pub(crate) fn direct(&self, operation: Operation) -> Option<Condition> {
        let (pair, negative) = self.operands(operation);
        GcLease::assert_mutable(&self.frozen);
        if self.sift.is_some() {
            return None;
        }
        simple(pair).map(|c| if negative { c.not() } else { c })
    }

    pub fn start(&self, operation: Operation) -> Job {
        let (pair, negative) = self.operands(operation);
        let known = self.cached(pair);
        Job {
            owner: self.owner,
            negative,
            input: known.is_none().then_some(pair),
            epoch: self.order_epoch,
            restarting: false,
            order_lease: None,
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
    /// including after completion. Dense live graphs may be reordered after
    /// sweeping. Dropping early aborts reclamation; subsequent job/collector
    /// ticks finish any partial swap and restore its best known order.
    pub fn collect<I: Iterator<Item = Condition>>(&mut self, roots: I) -> Collector<I> {
        self.collect_archived(roots, vec![], true)
    }
    // A reset supplies the complete archive root inventory; otherwise the
    // roots are additions emitted by the persistent index collectors. Inventory
    // reconciliation changes closure ownership without retracing unchanged DAGs.
    pub(crate) fn collect_archived<I: Iterator<Item = Condition>>(
        &mut self,
        roots: I,
        archived: Vec<Condition>,
        reset: bool,
    ) -> Collector<I> {
        assert!(
            reset || self.sift.is_none(),
            "cached archive collection requires completed reordering"
        );
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
            archived: archived.into_iter(),
            inventory: BTreeMap::new(),
            archive_cursor: None,
            reset,
            deactivate: false,
            rebuild_unique: false,
            pending: Vec::new(),
            phase: Phase::Reorder,
            branching: false,
            sweep: None,
            sweep_next: None,
            unique: HashMap::new(),
        }
    }

    fn archive_tick(&mut self) -> bool {
        let Some((root, count, retain)) = self.archive_pending.pop() else {
            return true;
        };
        if root.is_terminal() {
            return false;
        }
        let node = self
            .nodes
            .get_mut(&root.id)
            .expect("owned archive condition");
        if retain {
            if node.archived != self.archive_epoch {
                node.archived = self.archive_epoch;
                node.archive_references = 0;
                self.unprotected.as_mut().unwrap().remove(&root.id);
                self.archive_branching +=
                    usize::from(!node.key.low.is_terminal() && !node.key.high.is_terminal());
                self.archive_pending
                    .extend([(node.key.low, 1, true), (node.key.high, 1, true)]);
            }
            node.archive_references = node
                .archive_references
                .checked_add(count)
                .expect("archive reference count exhausted");
        } else {
            assert_eq!(node.archived, self.archive_epoch, "owned archive edge");
            node.archive_references = node
                .archive_references
                .checked_sub(count)
                .expect("owned archive reference");
            if node.archive_references == 0 {
                node.archived = 0;
                self.unprotected.as_mut().unwrap().insert(root.id);
                self.archive_branching -=
                    usize::from(!node.key.low.is_terminal() && !node.key.high.is_terminal());
                self.archive_pending
                    .extend([(node.key.low, 1, false), (node.key.high, 1, false)]);
            }
        }
        false
    }
}

// Sift the live graph while collection owns the mutator barrier. A trial
// measures all supplied roots, so improving one function cannot hide growth
// in another. Causal predecessor bounds constrain eligible swaps.
// ponytail: at most 32 variables and 262144 probing actions per pass; broader
// searches need measured benefit. Restoration and cleanup finish incrementally.
struct Sift {
    phase: SiftPhase,
    candidates: BTreeMap<(std::cmp::Reverse<usize>, u64), ()>,
    zeros: Vec<Condition>,
    best_size: usize,
    best_rank: u64,
    moving: u64,
    remaining: usize,
    steps: usize,
    budget: usize,
    epoch: u64,
    reclaim: bool,
    stop: bool,
    changed: bool,
}
#[derive(Clone, Copy)]
enum Direction {
    Earlier,
    Later,
    Return,
}
#[derive(Clone, Copy)]
enum SiftPhase {
    Candidates(Option<u64>),
    Choose,
    Move(Direction),
    Swap {
        left: u64,
        right: u64,
        cursor: Option<u64>,
        end: u64,
        direction: Direction,
    },
    Drain(Direction),
    Cleanup,
}
impl Sift {
    fn new(arena: &Arena) -> Self {
        Self {
            phase: SiftPhase::Candidates(None),
            candidates: BTreeMap::new(),
            zeros: Vec::new(),
            best_size: arena.node_count(),
            best_rank: 0,
            moving: 0,
            remaining: 32,
            steps: 0,
            budget: arena
                .node_count()
                .saturating_mul(arena.order.len())
                .saturating_mul(16)
                .clamp(1024, 262_144),
            epoch: arena.epoch,
            reclaim: true,
            stop: false,
            changed: false,
        }
    }
    fn retain(arena: &mut Arena, c: Condition) {
        if !c.is_terminal() {
            let node = arena.nodes.get_mut(&c.id).unwrap();
            if node.references_epoch != arena.epoch {
                node.references = if node.archived == arena.archive_epoch {
                    node.archive_references
                } else {
                    0
                };
                node.references_epoch = arena.epoch;
            }
            node.references = node
                .references
                .checked_add(1)
                .expect("condition reference count exhausted");
        }
    }
    fn release(&mut self, arena: &mut Arena, c: Condition) {
        if !c.is_terminal() {
            let node = arena.nodes.get_mut(&c.id).unwrap();
            if node.references_epoch != arena.epoch {
                node.references = if node.archived == arena.archive_epoch {
                    node.archive_references
                } else {
                    0
                };
                node.references_epoch = arena.epoch;
            }
            node.references = node.references.checked_sub(1).expect("counted live edge");
            if node.references == 0 {
                self.zeros.push(c);
            }
        }
    }
    fn tick(&mut self, arena: &mut Arena) -> bool {
        // If a collector is dropped early, finish the swap and restore the best
        // order, but never reclaim nodes against an inventory no longer frozen.
        self.reclaim &= arena.epoch == self.epoch && arena.frozen.load(Ordering::Acquire);
        self.stop |= !self.reclaim;
        self.steps = self.steps.saturating_add(1);
        self.stop |= self.steps >= self.budget;
        match self.phase {
            SiftPhase::Candidates(cursor) => {
                if self.stop {
                    self.phase = SiftPhase::Cleanup;
                } else {
                    let next = match cursor {
                        Some(id) => arena.order.range((Excluded(id), Unbounded)).next(),
                        None => arena.order.first_key_value(),
                    };
                    if let Some((&choice, order)) = next {
                        self.candidates
                            .insert((std::cmp::Reverse(order.nodes.len()), choice), ());
                        self.phase = SiftPhase::Candidates(Some(choice));
                    } else {
                        self.phase = SiftPhase::Choose;
                    }
                }
            }
            SiftPhase::Choose => {
                if self.stop || self.remaining == 0 {
                    self.phase = SiftPhase::Cleanup;
                } else if let Some(((.., choice), ())) = self.candidates.pop_first() {
                    self.remaining -= 1;
                    self.moving = choice;
                    self.best_rank = arena.order[&choice].rank;
                    self.best_size = arena.node_count();
                    self.phase = SiftPhase::Move(Direction::Earlier);
                } else {
                    self.phase = SiftPhase::Cleanup;
                }
            }
            SiftPhase::Move(mut direction) => {
                if self.stop {
                    direction = Direction::Return;
                }
                let rank = arena.order[&self.moving].rank;
                if matches!(direction, Direction::Return) && rank == self.best_rank {
                    self.phase = SiftPhase::Choose;
                    return false;
                }
                let earlier = match direction {
                    Direction::Earlier => true,
                    Direction::Later => false,
                    Direction::Return => rank > self.best_rank,
                };
                let neighbor = if earlier {
                    arena.ranks.range(..rank).next_back()
                } else {
                    arena.ranks.range((Excluded(rank), Unbounded)).next()
                };
                if let Some((_, &other)) = neighbor {
                    let (left, right) = if earlier {
                        (other, self.moving)
                    } else {
                        (self.moving, other)
                    };
                    if arena.order[&right].after.is_some_and(|bound| left <= bound) {
                        self.phase = SiftPhase::Move(match direction {
                            Direction::Earlier => Direction::Later,
                            Direction::Later => Direction::Return,
                            Direction::Return => {
                                unreachable!("best position respects causal predecessors")
                            }
                        });
                        return false;
                    }
                    self.phase = SiftPhase::Swap {
                        left,
                        right,
                        cursor: None,
                        end: arena.next_node,
                        direction,
                    };
                } else {
                    self.phase = SiftPhase::Move(match direction {
                        Direction::Earlier => Direction::Later,
                        Direction::Later => Direction::Return,
                        Direction::Return => unreachable!("best rank remains live while sifting"),
                    });
                }
            }
            SiftPhase::Swap {
                left,
                right,
                cursor,
                end,
                direction,
            } => {
                let level = &arena.order[&left].nodes;
                let next = match cursor {
                    Some(id) => level.range((Excluded(id), Excluded(end))).next(),
                    None => level.range(..end).next(),
                }
                .copied();
                if let Some(id) = next {
                    let old = arena.nodes[&id].key;
                    let split = |c| match arena.view(c) {
                        View::Choice { choice, low, high } if choice == right => (low, high),
                        _ => (c, c),
                    };
                    let (ll, lh) = split(old.low);
                    let (hl, hh) = split(old.high);
                    if ll != lh || hl != hh {
                        arena.count_edges = self.reclaim;
                        let low = arena.node(left, ll, hl);
                        let high = arena.node(left, lh, hh);
                        arena.count_edges = false;
                        // Regular handles are false on the all-false assignment,
                        // independently of order. A swap cannot flip their sign.
                        assert!(!low.negative && low != high);
                        let key = NodeKey {
                            choice: right,
                            low,
                            high,
                        };
                        let handle = Condition {
                            owner: arena.owner,
                            id,
                            negative: false,
                        };
                        debug_assert_eq!(
                            arena.nodes[&id].max_choice,
                            right
                                .max(arena.support_max(low).unwrap_or(right))
                                .max(arena.support_max(high).unwrap_or(right))
                        );
                        assert_eq!(arena.unique.remove(&old), Some(handle));
                        assert!(
                            arena.unique.insert(key, handle).is_none(),
                            "canonical collision during swap"
                        );
                        arena.nodes.get_mut(&id).unwrap().key = key;
                        arena.order.get_mut(&left).unwrap().nodes.remove(&id);
                        arena.order.get_mut(&right).unwrap().nodes.insert(id);
                        if self.reclaim {
                            Self::retain(arena, low);
                            Self::retain(arena, high);
                            self.release(arena, old.low);
                            self.release(arena, old.high);
                        }
                    }
                    self.phase = SiftPhase::Swap {
                        left,
                        right,
                        cursor: Some(id),
                        end,
                        direction,
                    };
                } else {
                    let lrank = arena.order[&left].rank;
                    let rrank = arena.order[&right].rank;
                    arena.order.get_mut(&left).unwrap().rank = rrank;
                    arena.order.get_mut(&right).unwrap().rank = lrank;
                    arena.ranks.insert(lrank, right);
                    arena.ranks.insert(rrank, left);
                    self.changed = true;
                    self.phase = SiftPhase::Drain(direction);
                }
            }
            SiftPhase::Drain(direction) => {
                if let Some(c) = self.zeros.pop() {
                    if self.reclaim && arena.nodes.get(&c.id).is_some_and(|n| n.references == 0) {
                        let key = arena.nodes.remove(&c.id).unwrap().key;
                        if let Some(ids) = &mut arena.unprotected {
                            ids.remove(&c.id);
                        }
                        arena.unique.remove(&key);
                        arena
                            .order
                            .get_mut(&key.choice)
                            .unwrap()
                            .nodes
                            .remove(&c.id);
                        self.release(arena, key.low);
                        self.release(arena, key.high);
                    }
                } else {
                    let size = arena.node_count();
                    if self.reclaim && size < self.best_size {
                        self.best_size = size;
                        self.best_rank = arena.order[&self.moving].rank;
                    }
                    let direction = if self.stop {
                        Direction::Return
                    } else if size > self.best_size.saturating_mul(2) {
                        match direction {
                            Direction::Earlier => Direction::Later,
                            _ => Direction::Return,
                        }
                    } else {
                        direction
                    };
                    self.phase = SiftPhase::Move(direction);
                }
            }
            SiftPhase::Cleanup => {
                self.zeros = Vec::new();
                if self.candidates.pop_first().is_none() {
                    return true;
                }
            }
        }
        false
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

// An interrupted finite apply gets a stable order until it completes. Holding
// or dropping a suspended job cannot cause repeated reordering to starve it.
struct OrderLease(Arc<AtomicUsize>);
impl Drop for OrderLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct Job {
    owner: u32,
    negative: bool,
    input: Option<Pair>,
    epoch: u64,
    restarting: bool,
    order_lease: Option<OrderLease>,
    frames: Vec<Frame>,
    last: Option<Condition>,
    // Completed subproblems are semantic work dependencies, not an evicting cache.
    memo: BTreeMap<Pair, Condition>,
    work: u64,
    discarding: bool,
}

impl Job {
    pub fn result(&self) -> Option<Condition> {
        if !self.discarding && !self.restarting && self.frames.is_empty() && self.memo.is_empty() {
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
        self.input
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .chain(
                self.memo
                    .iter()
                    .flat_map(|(&(a, b), &result)| [a, b, result]),
            )
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
        self.order_lease = None;
        self.input = None;
        self.restarting = false;
        self.frames = Vec::new();
        self.last = None;
        self.memo.pop_first();
        self.memo.is_empty()
    }

    pub fn tick(&mut self, arena: &mut Arena) -> Progress {
        assert!(!self.discarding, "condition job has been discarded");
        assert_eq!(self.owner, arena.owner, "foreign condition job");
        GcLease::assert_mutable(&arena.frozen);
        if arena.sift_tick() {
            return Progress::Pending;
        }
        if self.epoch != arena.order_epoch {
            self.epoch = arena.order_epoch;
            if !self.frames.is_empty() {
                self.frames = Vec::new();
                self.last = None;
                self.restarting = true;
                if self.order_lease.is_none() {
                    arena.order_readers.fetch_add(1, Ordering::Relaxed);
                    self.order_lease = Some(OrderLease(arena.order_readers.clone()));
                }
            }
        }
        if self.restarting {
            if self.memo.pop_first().is_none() {
                self.frames
                    .push(Frame::Evaluate(self.input.expect("restarted operation")));
                self.restarting = false;
            }
            return Progress::Pending;
        }
        if let Some(result) = self.result() {
            return Progress::Complete(result);
        }
        if self.frames.is_empty() {
            if self.input.take().is_some() {
                return Progress::Pending;
            }
            self.memo.pop_first();
            if let Some(result) = self.result() {
                self.frames = Vec::new();
                self.input = None;
                self.order_lease = None;
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
                            if arena.precedes(a, b) { a } else { b }
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
            self.input = None;
            self.order_lease = None;
            Progress::Complete(result)
        } else {
            Progress::Pending
        }
    }
}

enum Phase {
    ArchiveClear,
    ArchiveRoots,
    ArchiveAdd,
    ArchiveRelease,
    ArchiveFinish,
    Reset,
    Reorder,
    Sift,
    Cache,
    Mark,
    Sweep,
    Unique,
    Finish,
    Compact,
    Done,
}

pub struct Collector<I> {
    owner: u32,
    epoch: u64,
    _lease: GcLease,
    roots: I,
    archived: std::vec::IntoIter<Condition>,
    inventory: BTreeMap<Condition, usize>,
    archive_cursor: Option<Condition>,
    reset: bool,
    deactivate: bool,
    rebuild_unique: bool,
    pending: Vec<Condition>,
    phase: Phase,
    branching: bool,
    sweep: Option<u64>,
    // Saved live successor, independent of the payload about to be retired.
    sweep_next: Option<u64>,
    unique: HashMap<NodeKey, Condition>,
}

impl<I: Iterator<Item = Condition>> Collector<I> {
    /// Returns true when finished. Each call clears one cache entry, marks or
    /// sweeps one node, or performs one bounded reordering action (at most two
    /// new nodes). An interrupted swap is completed before marking begins.
    pub fn tick(&mut self, arena: &mut Arena) -> bool {
        assert_eq!(self.owner, arena.owner, "foreign condition collector");
        assert_eq!(self.epoch, arena.epoch, "stale condition collector");
        match self.phase {
            Phase::Reorder => {
                if let Some(sift) = arena.sift.as_mut() {
                    sift.stop = true;
                }
                if !arena.sift_tick() {
                    if arena.archive_order_epoch != arena.order_epoch {
                        // Stable function identities now have different edges.
                        // The caller supplies the complete inventory after a
                        // representation change, including an interrupted swap.
                        assert!(self.reset, "reordered archive requires all roots");
                        assert!(arena.archive_pending.is_empty());
                        arena.archive_order_epoch = arena.order_epoch;
                        arena.archive_epoch = arena
                            .archive_epoch
                            .checked_add(1)
                            .expect("condition archive epoch exhausted");
                        arena.archive_branching = 0;
                        arena.archive_rebuild = true;
                        self.phase = Phase::ArchiveClear;
                    } else if arena.archive_rebuild {
                        assert!(self.reset, "interrupted archive rebuild requires all roots");
                        self.phase = Phase::ArchiveClear;
                    } else if !arena.archive_tick() {
                        // Complete an interrupted ownership change before
                        // reconciling this collector's inventory.
                    } else if !self.archived.as_slice().is_empty() && arena.unprotected.is_none() {
                        arena.unprotected = Some(BTreeSet::new());
                        arena.archive_rebuild = true;
                        self.phase = Phase::Reset;
                    } else {
                        self.phase = Phase::ArchiveRoots;
                    }
                }
            }
            Phase::ArchiveClear => {
                if arena.archive_roots.pop_first().is_none() {
                    if arena.unprotected.is_some() || !self.archived.as_slice().is_empty() {
                        arena.unprotected.get_or_insert_with(BTreeSet::new);
                        self.phase = Phase::Reset;
                    } else {
                        self.phase = Phase::ArchiveRoots;
                    }
                }
            }
            Phase::ArchiveRoots => {
                if let Some(root) = self.archived.next() {
                    assert!(arena.contains(root), "stale or foreign archive root");
                    if !root.is_terminal() {
                        let root = Condition {
                            negative: false,
                            ..root
                        };
                        let count = self.inventory.entry(root).or_default();
                        *count = count.checked_add(1).expect("archive root count exhausted");
                    }
                } else {
                    self.phase = Phase::ArchiveAdd;
                }
            }
            Phase::ArchiveAdd => {
                if !arena.archive_tick() {
                    return false;
                }
                let next = match self.archive_cursor {
                    Some(root) => self.inventory.range((Excluded(root), Unbounded)).next(),
                    None => self.inventory.first_key_value(),
                }
                .map(|(&root, &count)| (root, count));
                if let Some((root, incoming)) = next {
                    let previous = arena.archive_roots.get(&root).copied().unwrap_or(0);
                    let desired = if self.reset {
                        incoming
                    } else {
                        previous
                            .checked_add(incoming)
                            .expect("archive root count exhausted")
                    };
                    if desired > previous {
                        arena.archive_roots.insert(root, desired);
                        arena.archive_pending.push((root, desired - previous, true));
                    }
                    self.archive_cursor = Some(root);
                } else {
                    self.archive_cursor = None;
                    self.phase = Phase::ArchiveRelease;
                }
            }
            Phase::ArchiveRelease => {
                if !arena.archive_tick() {
                    return false;
                }
                let next = if self.reset {
                    match self.archive_cursor {
                        Some(root) => arena
                            .archive_roots
                            .range((Excluded(root), Unbounded))
                            .next(),
                        None => arena.archive_roots.first_key_value(),
                    }
                    .map(|(&root, &count)| (root, count))
                } else {
                    None
                };
                if let Some((root, previous)) = next {
                    let desired = self.inventory.get(&root).copied().unwrap_or(0);
                    if desired < previous {
                        if desired == 0 {
                            arena.archive_roots.remove(&root);
                        } else {
                            arena.archive_roots.insert(root, desired);
                        }
                        arena
                            .archive_pending
                            .push((root, previous - desired, false));
                    }
                    self.archive_cursor = Some(root);
                } else {
                    self.phase = Phase::ArchiveFinish;
                }
            }
            Phase::ArchiveFinish => {
                // Release temporary inventory storage incrementally as well.
                if self.inventory.pop_first().is_none() {
                    arena.archive_pending = Vec::new();
                    self.deactivate = arena.archive_roots.is_empty() && arena.unprotected.is_some();
                    self.rebuild_unique = arena.archive_roots.is_empty();
                    arena.archive_rebuild = self.deactivate;
                    self.branching = arena.archive_branching != 0;
                    self.phase = if self.deactivate {
                        Phase::Reset
                    } else {
                        Phase::Cache
                    };
                }
            }
            Phase::Sift => {
                if !arena.sift_tick() {
                    self.phase = Phase::Compact;
                }
            }
            Phase::Reset => {
                if self.deactivate {
                    if arena.unprotected.as_mut().unwrap().pop_first().is_none() {
                        arena.unprotected = None;
                        arena.archive_rebuild = false;
                        self.phase = Phase::Cache;
                    }
                    return false;
                }
                let next = arena.nodes.next(self.sweep).map(|(id, _)| id);
                if let Some(id) = next {
                    arena.unprotected.as_mut().unwrap().insert(id);
                    self.sweep = Some(id);
                } else {
                    self.sweep = None;
                    arena.archive_rebuild = false;
                    self.phase = Phase::ArchiveRoots;
                }
            }
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
                            node.references_epoch = self.epoch;
                            let protected = node.archived == arena.archive_epoch;
                            node.references = if protected {
                                node.archive_references
                            } else {
                                0
                            };
                            if !protected {
                                self.branching |=
                                    !node.key.low.is_terminal() && !node.key.high.is_terminal();
                                self.pending.extend([node.key.low, node.key.high]);
                            }
                        }
                        node.references = node
                            .references
                            .checked_add(1)
                            .expect("condition reference count exhausted");
                    }
                } else {
                    self.sweep_next = if arena.unprotected.is_none() {
                        arena.nodes.next(None).map(|(id, _)| id)
                    } else {
                        None
                    };
                    self.phase = Phase::Sweep;
                }
            }
            Phase::Sweep => {
                let next = if let Some(ids) = &arena.unprotected {
                    match self.sweep {
                        Some(id) => ids.range((Excluded(id), Unbounded)).next(),
                        None => ids.first(),
                    }
                    .copied()
                } else {
                    self.sweep_next
                };
                if let Some(id) = next {
                    if arena.unprotected.is_none() {
                        self.sweep_next = arena.nodes.next(Some(id)).map(|(id, _)| id);
                    }
                    let node = &arena.nodes[&id];
                    let key = node.key;
                    if node.marked != self.epoch {
                        arena.nodes.remove(&id);
                        if let Some(ids) = &mut arena.unprotected {
                            ids.remove(&id);
                        }
                        arena.unique.remove(&key);
                        let order = arena.order.get_mut(&key.choice).expect("allocated choice");
                        order.nodes.remove(&id);
                        if order.nodes.is_empty() {
                            arena.ranks.remove(&order.rank);
                            arena.order.remove(&key.choice);
                        }
                    } else if self.rebuild_unique {
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
                    if self.rebuild_unique {
                        arena.unique = std::mem::take(&mut self.unique);
                        self.phase = Phase::Finish;
                    } else if arena.unique.capacity() > arena.node_count().saturating_mul(2) {
                        // Retain the canonical table across small owner changes.
                        // A large contraction pays one bounded pass to return
                        // excess table storage, amortized against reclaimed nodes.
                        self.sweep = None;
                        self.phase = Phase::Unique;
                    } else {
                        self.phase = Phase::Finish;
                    }
                }
            }
            Phase::Unique => {
                let next = arena.nodes.next(self.sweep);
                if let Some((id, node)) = next {
                    self.unique.insert(
                        node.key,
                        Condition {
                            owner: arena.owner,
                            id,
                            negative: false,
                        },
                    );
                    self.sweep = Some(id);
                } else {
                    arena.unique = std::mem::take(&mut self.unique);
                    self.phase = Phase::Finish;
                }
            }
            Phase::Finish => {
                // Many retained path prefixes can be dense without a
                // branching representation problem. Avoid paying for a
                // whole sift merely because observation accumulated paths.
                // This is a conservative trigger, not a claim that chains
                // can never share better under another order.
                if self.branching
                    && arena.next_node >= arena.reorder_after
                    && arena.order_readers.load(Ordering::Relaxed) == 0
                    && arena.node_count() >= 128
                    && arena.node_count() > 4 * arena.order.len()
                {
                    arena.sift = Some(Sift::new(arena));
                    self.phase = Phase::Sift;
                } else {
                    self.phase = Phase::Compact;
                }
            }
            Phase::Compact => {
                if arena.nodes.compact_tick() {
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
            3 => match self.input {
                Some((a, b)) => cursor.fields(&[a, b]),
                None => Step::Done,
            },
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
    QuantifyLow {
        input: Condition,
        high: Condition,
    },
    QuantifyHigh {
        input: Condition,
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
        cleanup_substitution(&mut self.memo, &mut self.images, &mut self.draining)
    }
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.frames = Vec::new();
        self.last = None;
        if discard_slot(&mut self.job, |child| child.discard_tick()) {
            return false;
        }
        self.cleanup_tick()
    }
    pub fn tick(&mut self, arena: &mut Arena) -> Progress {
        assert!(!self.discarding, "condition transform has been discarded");
        assert_eq!(self.owner, arena.owner, "foreign condition transform");
        GcLease::assert_mutable(&arena.frozen);
        if arena.sift_tick() {
            return Progress::Pending;
        }
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
                                    self.frames
                                        .push(TransformFrame::QuantifyLow { input, high });
                                    self.frames.push(TransformFrame::Evaluate(low));
                                    self.last = None;
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
                TransformFrame::QuantifyLow { input, high } => {
                    let low = self.last.take().expect("quantified low cofactor");
                    self.frames
                        .push(TransformFrame::QuantifyHigh { input, low });
                    self.frames.push(TransformFrame::Evaluate(high));
                }
                TransformFrame::QuantifyHigh { input, low } => {
                    let high = self.last.take().expect("quantified high cofactor");
                    self.frames.push(TransformFrame::Finish(input));
                    self.job = Some(arena.start(Operation::Or(low, high)));
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
                                    View::Choice { choice: child, .. } => {
                                        arena.precedes(choice, child)
                                    }
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
            Self::QuantifyLow { input, high } => [Some(input), Some(high), None],
            Self::QuantifyHigh { input, low } => [Some(input), Some(low), None],
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

#[cfg(test)]
mod direct_tests {
    use super::*;
    #[test]
    fn order_metadata_is_collected_with_its_last_node() {
        let mut arena = Arena::default();
        for _ in 0..4 {
            let vars: Vec<_> = (0..64).map(|_| arena.fresh_choice().1).collect();
            let mut job = arena.start(Operation::And(vars[0], vars[63]));
            for tick in 0..100 {
                if let Progress::Complete(_) = job.tick(&mut arena) {
                    break;
                }
                assert!(tick < 99);
            }
            assert_eq!(arena.order.len(), 64);
            let mut gc = arena.collect([vars[0]].into_iter());
            for tick in 0..1000 {
                if gc.tick(&mut arena) {
                    break;
                }
                assert!(tick < 999);
            }
            drop(gc);
            assert_eq!(arena.order.len(), 1);
            assert_eq!(arena.order.values().next().unwrap().nodes.len(), 1);
            let mut gc = arena.collect(std::iter::empty());
            for tick in 0..100 {
                if gc.tick(&mut arena) {
                    break;
                }
                assert!(tick < 99);
            }
            drop(gc);
            assert!(arena.order.is_empty());
        }
    }

    #[test]
    fn direct_identities_match_truth_tables_and_leave_mixed_operands_resumable() {
        let mut a = Arena::default();
        let (x, c) = a.fresh_choice();
        let (_, d) = a.fresh_choice();
        let values = [Condition::FALSE, Condition::TRUE, c, c.not(), d, d.not()];
        for left in values {
            for right in values {
                for op in [
                    Operation::And(left, right),
                    Operation::Or(left, right),
                    Operation::Difference(left, right),
                ] {
                    let nodes = a.node_count();
                    let cache = a.cache_len();
                    if let Some(result) = a.direct(op) {
                        for bits in 0..4 {
                            let eval =
                                |v| a.evaluate(v, |id| bits & (if id == x { 1 } else { 2 }) != 0);
                            let expected = match op {
                                Operation::And(_, _) => eval(left) && eval(right),
                                Operation::Or(_, _) => eval(left) || eval(right),
                                Operation::Difference(_, _) => eval(left) && !eval(right),
                            };
                            assert_eq!(eval(result), expected);
                        }
                    }
                    assert_eq!(a.node_count(), nodes);
                    assert_eq!(a.cache_len(), cache);
                }
            }
        }
        assert!(a.direct(Operation::And(c, d)).is_none());
        assert!(a.direct(Operation::Or(c, d)).is_none());
        assert!(a.direct(Operation::Difference(c, d)).is_none());
    }
    #[test]
    fn direct_checks_authority_before_false_or_identical_shortcuts() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        for stale in [false, true] {
            let mut owner = Arena::default();
            let c = owner.fresh_choice().1;
            let foreign = Arena::default();
            if stale {
                let mut gc = owner.collect(std::iter::empty());
                while !gc.tick(&mut owner) {}
            }
            let arena = if stale { &owner } else { &foreign };
            assert!(!arena.contains(c));
            for op in [
                Operation::And(c, Condition::FALSE),
                Operation::And(Condition::FALSE, c),
                Operation::And(c, c),
                Operation::Or(c, c),
                Operation::Difference(c, c),
            ] {
                assert!(catch_unwind(AssertUnwindSafe(|| arena.direct(op))).is_err());
            }
        }
    }

    #[test]
    fn direct_rejects_frozen_arena_for_simple_and_mixed_operands() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut a = Arena::default();
        let c = a.fresh_choice().1;
        let d = a.fresh_choice().1;
        let gc = a.collect([c, d].into_iter());
        for op in [
            Operation::And(c, c),
            Operation::And(c, Condition::FALSE),
            Operation::And(c, d),
        ] {
            assert!(catch_unwind(AssertUnwindSafe(|| a.direct(op))).is_err());
        }
        drop(gc);
        assert_eq!(a.direct(Operation::And(c, c)), Some(c));
    }

    #[test]
    fn mixed_direct_fallback_resumes_and_discards_with_collection_each_tick() {
        let mut resumed = 0;
        let mut discarded = 0;
        for discard_at in 0..16 {
            let mut a = Arena::default();
            let (x, c) = a.fresh_choice();
            let (_, d) = a.fresh_choice();
            let op = Operation::And(c, d);
            assert_eq!(a.direct(op), None);
            let mut job = a.start(op);
            let mut finished = false;
            for tick in 0..100 {
                let roots: Vec<_> = job.roots().chain([c, d]).collect();
                let mut gc = a.collect(roots.into_iter());
                while !gc.tick(&mut a) {}
                drop(gc);
                if tick >= discard_at {
                    if job.discard_tick() {
                        discarded += 1;
                        finished = true;
                        break;
                    }
                } else if let Progress::Complete(result) = job.tick(&mut a) {
                    resumed += 1;
                    for bits in 0..4 {
                        assert_eq!(
                            a.evaluate(result, |id| bits & if id == x { 1 } else { 2 } != 0),
                            bits == 3
                        );
                    }
                    finished = true;
                    break;
                }
            }
            assert!(finished);
        }
        assert!(resumed > 0 && discarded > 0);
    }
}

#[cfg(test)]
mod reorder_tests {
    use super::*;
    fn finish(a: &mut Arena, operation: Operation) -> Condition {
        let mut j = a.start(operation);
        (0..100_000)
            .find_map(|_| match j.tick(a) {
                Progress::Complete(c) => Some(c),
                Progress::Pending => None,
            })
            .expect("finite test operation")
    }
    struct Fixture {
        a: Arena,
        vars: Vec<Condition>,
        root: Condition,
        job: Job,
        transform: Transform,
        projection: Transform,
        cached: Job,
    }
    impl Fixture {
        fn new() -> Self {
            let mut a = Arena::default();
            let vars: Vec<_> = (0..12).map(|_| a.fresh_choice().1).collect();
            let mut root = Condition::TRUE;
            for i in 0..6 {
                let both = finish(&mut a, Operation::And(vars[i], vars[i + 6]));
                let neither = finish(&mut a, Operation::And(vars[i].not(), vars[i + 6].not()));
                let eq = finish(&mut a, Operation::Or(both, neither));
                root = finish(&mut a, Operation::And(root, eq));
            }
            let selected = finish(&mut a, Operation::And(root, vars[0]));
            let cached = a.start(Operation::And(root, vars[0]));
            assert_eq!(cached.result(), Some(selected));
            let mut job = a.start(Operation::Difference(root, vars[3]));
            for step in 0..1000 {
                job.tick(&mut a);
                if job
                    .frames
                    .iter()
                    .any(|f| matches!(f, Frame::AfterHigh { .. }))
                {
                    break;
                }
                assert!(step < 999);
            }
            assert!(!job.frames.is_empty());
            let mut transform = a.substitute(root, Arc::new(BTreeMap::from([(1, vars[2])])));
            let mut projection = a.project_before(selected, 6);
            transform.tick(&mut a);
            projection.tick(&mut a);
            Self {
                a,
                vars,
                root,
                job,
                transform,
                projection,
                cached,
            }
        }
        fn roots(&self) -> Vec<Condition> {
            self.vars
                .iter()
                .copied()
                .chain([self.root])
                .chain(self.job.roots())
                .chain(self.transform.roots())
                .chain(self.projection.roots())
                .chain(self.cached.roots())
                .collect()
        }
        fn assert_root(&self) {
            for bits in (0..64u64).chain((0..64).map(|b| b | (b << 6))) {
                let value = |i| bits & (1u64 << i) != 0;
                let expected = (0..6).all(|i| value(i) == value(i + 6));
                assert_eq!(self.a.evaluate(self.root, value), expected);
                assert_eq!(self.a.evaluate(self.root.not(), value), !expected);
                assert_eq!(
                    self.a.evaluate(self.cached.result().unwrap(), value),
                    expected && value(0)
                );
            }
        }
    }
    #[test]
    fn archived_closures_keep_exact_sift_references_and_rebuild_after_reordering() {
        let mut f = Fixture::new();
        let roots = f.roots();
        f.a.reorder_after = u64::MAX;
        let mut gc =
            f.a.collect_archived(std::iter::empty(), roots.clone(), true);
        while !gc.tick(&mut f.a) {}
        drop(gc);
        f.assert_root();
        let working = finish(&mut f.a, Operation::And(f.vars[0], f.vars[1].not()));
        f.a.reorder_after = 0;
        let before = f.a.order_epoch;
        let mut gc = f.a.collect_archived([working].into_iter(), vec![], false);
        let mut checked = false;
        for _ in 0..200_000 {
            let done = gc.tick(&mut f.a);
            if !checked && f.a.sift.is_some() {
                let mut expected = BTreeMap::<u64, usize>::new();
                for root in roots.iter().copied().chain([working]).chain(
                    std::iter::successors(f.a.nodes.next(None), |(id, _)| {
                        f.a.nodes.next(Some(*id))
                    })
                    .flat_map(|(_, node)| [node.key.low, node.key.high]),
                ) {
                    if !root.is_terminal() {
                        *expected.entry(root.id).or_default() += 1;
                    }
                }
                for (id, node) in
                    std::iter::successors(f.a.nodes.next(None), |(id, _)| f.a.nodes.next(Some(*id)))
                {
                    let count = if node.references_epoch == f.a.epoch {
                        node.references
                    } else if node.archived == f.a.archive_epoch {
                        node.archive_references
                    } else {
                        0
                    };
                    assert_eq!(count, expected[&id], "exact incoming references for {id}");
                }
                checked = true;
            }
            if done {
                break;
            }
        }
        assert!(checked && matches!(gc.phase, Phase::Done));
        drop(gc);
        assert!(f.a.order_epoch > before);
        f.assert_root();
        let mut gc = f.a.collect_archived([working].into_iter(), roots, true);
        while !gc.tick(&mut f.a) {}
        drop(gc);
        f.assert_root();
        for bits in 0..4 {
            assert_eq!(f.a.evaluate(working, |id| bits & (1 << id) != 0), bits == 1);
        }
        let mut gc = f.a.collect(std::iter::empty());
        while !gc.tick(&mut f.a) {}
        drop(gc);
        assert_eq!(f.a.node_count(), 0);
        assert!(f.a.unprotected.is_none());

        // Public collection may replace an interrupted archival collector. Its
        // reset must finish the swap before rebuilding reachability and counts.
        let mut f = Fixture::new();
        let mut gc = f.a.collect_archived(std::iter::empty(), f.roots(), true);
        for tick in 0..200_000 {
            gc.tick(&mut f.a);
            if f.a.sift.as_ref().is_some_and(|s| {
                matches!(
                    s.phase,
                    SiftPhase::Swap {
                        cursor: Some(_),
                        ..
                    }
                )
            }) {
                break;
            }
            assert!(tick < 199_999);
        }
        drop(gc);
        let mut gc = f.a.collect(f.roots().into_iter());
        for tick in 0..200_000 {
            if gc.tick(&mut f.a) {
                break;
            }
            assert!(tick < 199_999);
        }
        drop(gc);
        f.assert_root();
        let mut gc = f.a.collect(std::iter::empty());
        while !gc.tick(&mut f.a) {}
        drop(gc);
        assert_eq!(f.a.node_count(), 0);
    }
    #[test]
    fn collection_reordering_can_be_interrupted_in_every_structural_phase() {
        let mut discover = Fixture::new();
        let mut checkpoints = BTreeMap::new();
        let mut gc = discover.a.collect(discover.roots().into_iter());
        let mut completion = 0;
        for tick in 0..200_000 {
            let phase = match discover.a.sift.as_ref().map(|s| s.phase) {
                None => 0,
                Some(SiftPhase::Candidates(_)) => 1,
                Some(SiftPhase::Choose) => 2,
                Some(SiftPhase::Move(Direction::Return)) => 3,
                Some(SiftPhase::Move(_)) => 4,
                Some(SiftPhase::Swap { cursor: None, .. }) => 5,
                Some(SiftPhase::Swap { .. }) => 6,
                Some(SiftPhase::Drain(_)) => 7,
                Some(SiftPhase::Cleanup) => 8,
            };
            checkpoints.entry(phase).or_insert(tick);
            if discover
                .a
                .sift
                .as_ref()
                .is_some_and(|s| matches!(s.phase, SiftPhase::Swap { .. }) && !s.zeros.is_empty())
            {
                checkpoints.entry(9).or_insert(tick);
            }
            let before = discover.a.node_count();
            if gc.tick(&mut discover.a) {
                completion = tick + 1;
                break;
            }
            assert!(
                discover.a.node_count() <= before + 2,
                "unbounded swap allocation"
            );
            assert!(tick < 199_999);
        }
        assert_eq!(
            checkpoints.len(),
            10,
            "exercise each maintenance phase, including a partially rewritten level"
        );
        checkpoints.insert(10, completion);
        drop(gc);
        assert!(discover.a.order_epoch > 0);
        for (_, checkpoint) in checkpoints {
            let mut f = Fixture::new();
            let mut gc = f.a.collect(f.roots().into_iter());
            for _ in 0..checkpoint {
                gc.tick(&mut f.a);
            }
            f.assert_root();
            drop(gc);
            let born = f.a.fresh_choice();
            assert_eq!(born.0, 12, "birth identity is independent of order");
            for step in 0..200_000 {
                if f.a.sift.is_none() {
                    break;
                }
                let before = f.a.node_count();
                f.job.tick(&mut f.a);
                assert!(
                    f.a.node_count() >= before,
                    "aborted collector cannot reclaim against an unfrozen inventory"
                );
                assert!(step < 199_999);
            }
            f.assert_root();
            for tick in 0..20_000 {
                if tick % 7 == 0 {
                    let mut gc = f.a.collect(f.roots().into_iter());
                    for step in 0..200_000 {
                        if gc.tick(&mut f.a) {
                            break;
                        }
                        assert!(step < 199_999);
                    }
                    drop(gc);
                }
                f.job.tick(&mut f.a);
                f.transform.tick(&mut f.a);
                f.projection.tick(&mut f.a);
                if f.job.result().is_some()
                    && f.transform.result().is_some()
                    && f.projection.result().is_some()
                {
                    break;
                }
                assert!(tick < 19_999);
            }
            let expected = finish(&mut f.a, Operation::Difference(f.root, f.vars[3]));
            assert_eq!(
                f.job.result(),
                Some(expected),
                "restart stale AfterHigh frames before construction"
            );
            assert_eq!(f.projection.result(), Some(f.vars[0]));
            for bits in 0..4096u64 {
                let value = |i| bits & (1u64 << i) != 0;
                assert_eq!(
                    f.a.evaluate(f.transform.result().unwrap(), value),
                    (0..6).all(|i| value(if i == 1 { 2 } else { i }) == value(i + 6))
                );
            }
            assert_eq!(f.a.order_readers.load(Ordering::Relaxed), 0);
            let mut gc = f.a.collect(std::iter::empty());
            for step in 0..200_000 {
                if gc.tick(&mut f.a) {
                    break;
                }
                assert!(step < 199_999);
            }
            drop(gc);
            assert_eq!(f.a.node_count(), 0);
            assert!(f.a.order.is_empty() && f.a.ranks.is_empty());
        }
    }
    #[test]
    fn path_prefix_collection_stays_linear_without_branching_pressure() {
        let mut a = Arena::default();
        let vars: Vec<_> = (0..30).map(|_| a.fresh_choice().1).collect();
        let mut roots = vars.clone();
        let mut prefix = Condition::TRUE;
        for &literal in &vars {
            roots.push(finish(&mut a, Operation::And(prefix, literal.not())));
            prefix = finish(&mut a, Operation::And(prefix, literal));
            roots.push(prefix);
        }
        let allocated = a.node_count();
        assert!(allocated >= 128 && allocated > 4 * vars.len());
        let budget = 8 * allocated + a.cache.len() + roots.len() + 32;
        let mut gc = a.collect(roots.iter().copied());
        let mut finished = false;
        for _ in 0..budget {
            if gc.tick(&mut a) {
                finished = true;
                break;
            }
        }
        assert!(
            finished,
            "path-prefix roots must collect within linear structural work"
        );
        drop(gc);
        assert_eq!(a.node_count(), allocated);
        assert!(a.evaluate(prefix, |_| true));
        for false_choice in 0..30 {
            assert!(!a.evaluate(prefix, |choice| choice != false_choice));
        }
    }

    #[test]
    fn scoped_sifting_respects_causal_bounds_without_retaining_scope_graphs() {
        let mut a = Arena::default();
        let parent = a.fresh_choice().1;
        let vars: Vec<_> = (0..16).map(|_| a.fresh_scoped_choice(parent).1).collect();
        let scope = finish(&mut a, Operation::And(parent, vars[3]));
        let nested = a.fresh_scoped_choice(scope).1;
        assert_eq!(a.support_max(scope), Some(4));
        let mut root = parent;
        for i in 0..8 {
            let p = finish(&mut a, Operation::And(vars[i], vars[i + 8]));
            let q = finish(&mut a, Operation::And(vars[i].not(), vars[i + 8].not()));
            let eq = finish(&mut a, Operation::Or(p, q));
            root = finish(&mut a, Operation::And(root, eq));
        }
        let roots = vars.iter().copied().chain([parent, scope, nested, root]);
        let mut gc = a.collect(roots);
        for tick in 0..1_000_000 {
            if gc.tick(&mut a) {
                break;
            }
            assert!(tick < 999_999);
        }
        drop(gc);
        assert!(a.order_epoch > 0 && a.node_count() < 160);
        for (&choice, order) in &a.order {
            if let Some(bound) = order.after {
                for (_, predecessor) in a.order.range(..=bound) {
                    assert!(
                        predecessor.rank < order.rank,
                        "causal prefix of {choice} crossed"
                    );
                }
            }
        }
        for bits in 0..256u64 {
            let assignment = 1 | (bits << 1) | (bits << 9);
            assert!(a.evaluate(root, |i| assignment & (1 << i) != 0));
            assert!(!a.evaluate(root, |i| i != 0 && assignment & (1 << i) != 0));
        }
        assert_eq!(a.support_max(scope), Some(4));
        let mut gc = a.collect([nested].into_iter());
        for tick in 0..100_000 {
            if gc.tick(&mut a) {
                break;
            }
            assert!(tick < 99_999);
        }
        drop(gc);
        assert_eq!(
            a.node_count(),
            1,
            "predecessor bounds must not own historical scope conditions"
        );
        assert!(!a.contains(parent) && !a.contains(scope));
    }

    #[test]
    fn restarted_job_has_stable_order_until_completion_or_cancellation() {
        for cancel in [false, true] {
            let mut f = Fixture::new();
            let mut gc = f.a.collect(f.roots().into_iter());
            for step in 0..200_000 {
                if gc.tick(&mut f.a) {
                    break;
                }
                assert!(step < 199_999);
            }
            drop(gc);
            f.job.tick(&mut f.a);
            assert_eq!(f.a.order_readers.load(Ordering::Relaxed), 1);
            let epoch = f.a.order_epoch;
            // Independent new dense work must not repeatedly restart this job.
            let vars: Vec<_> = (0..16).map(|_| f.a.fresh_choice().1).collect();
            let mut root = Condition::TRUE;
            for i in 0..8 {
                let p = finish(&mut f.a, Operation::And(vars[i], vars[i + 8]));
                let q = finish(&mut f.a, Operation::And(vars[i].not(), vars[i + 8].not()));
                let eq = finish(&mut f.a, Operation::Or(p, q));
                root = finish(&mut f.a, Operation::And(root, eq));
            }
            for _ in 0..3 {
                let roots = f.roots().into_iter().chain([root]);
                let mut gc = f.a.collect(roots);
                for step in 0..200_000 {
                    if gc.tick(&mut f.a) {
                        break;
                    }
                    assert!(step < 199_999);
                }
                drop(gc);
                assert!(f.a.node_count() > 128 && f.a.node_count() > 4 * f.a.order.len());
                assert_eq!(
                    f.a.order_epoch, epoch,
                    "growth must not starve an interrupted finite job"
                );
                f.job.tick(&mut f.a);
            }
            for step in 0..100_000 {
                let done = if cancel {
                    f.job.discard_tick()
                } else {
                    matches!(f.job.tick(&mut f.a), Progress::Complete(_))
                };
                if done {
                    break;
                }
                assert!(step < 99_999);
            }
            assert_eq!(f.a.order_readers.load(Ordering::Relaxed), 0);
            let mut gc = f.a.collect(f.roots().into_iter().chain([root]));
            for step in 0..200_000 {
                if gc.tick(&mut f.a) {
                    break;
                }
                assert!(step < 199_999);
            }
            drop(gc);
            assert!(
                f.a.order_epoch > epoch,
                "released lease permits growth-triggered adaptation"
            );
        }
    }
}

#[cfg(test)]
mod archive_ownership_tests {
    use super::*;

    fn finish(a: &mut Arena, operation: Operation) -> Condition {
        let mut job = a.start(operation);
        loop {
            if let Progress::Complete(result) = job.tick(a) {
                return result;
            }
        }
    }

    fn collect(a: &mut Arena, working: Vec<Condition>, archived: Vec<Condition>) -> usize {
        let mut collector = a.collect_archived(working.into_iter(), archived, true);
        for ticks in 1..1_000_000 {
            if collector.tick(a) {
                return ticks;
            }
        }
        panic!("finite ownership update must complete");
    }

    #[test]
    fn epoch_handles_keep_full_serial_width_and_reject_retired_payloads() {
        for start in [u32::MAX as u64, u64::MAX - 2] {
            let mut a = Arena::default();
            a.next_node = start;
            let old = a.fresh_choice().1;
            collect(&mut a, vec![], vec![]);
            let new = a.fresh_choice().1;
            assert_eq!(new.id, start + 1);
            assert!(!a.contains(old) && !a.contains(old.not()));
            assert!(a.evaluate(new, |_| true));
            if start == u64::MAX - 2 {
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        a.fresh_choice();
                    }))
                    .is_err()
                );
                assert!(a.contains(new));
            }
            collect(&mut a, vec![], vec![]);
            assert_eq!(a.nodes.blocks.capacity(), 0);
        }
    }

    #[test]
    fn frozen_successor_survives_retirement_of_entire_current_epoch() {
        let mut a = Arena::default();
        let vars: Vec<_> = (0..64).map(|_| a.fresh_choice().1).collect();
        let high = vars[63];
        let mut gc = a.collect([high].into_iter());
        while !matches!(gc.phase, Phase::Sweep) {
            assert!(!gc.tick(&mut a));
        }
        for _ in 0..8 {
            assert!(!gc.tick(&mut a));
        }
        assert!(!a.nodes.blocks.contains_key(&0));
        assert_eq!(gc.sweep_next, Some(8));
        #[cfg(feature = "diagnostics")]
        assert_eq!(a.slab_diagnostics()[13..15], [1, 16]);
        while !gc.tick(&mut a) {}
        drop(gc);
        assert_eq!(a.node_count(), 1);
        assert_eq!(a.nodes.blocks.len(), 1);
        assert!(a.contains(high));
        assert!(!a.contains(vars[0]));
        collect(&mut a, vec![], vec![]);
        assert_eq!(a.nodes.blocks.capacity(), 0);
    }

    #[test]
    fn sparse_blocks_release_payload_below_high_archive_and_reuse_safely() {
        for size in [64, 512, 4096] {
            let mut a = Arena::default();
            let variables: Vec<_> = (0..size).map(|_| a.fresh_choice().1).collect();
            let high = variables[size - 1];
            #[cfg(feature = "diagnostics")]
            println!(
                "sparse size={size} phase=dense gauges={:?}",
                a.slab_diagnostics()
            );
            collect(&mut a, vec![], vec![high]);
            #[cfg(feature = "diagnostics")]
            println!(
                "sparse size={size} phase=archived gauges={:?}",
                a.slab_diagnostics()
            );
            assert_eq!(a.node_count(), 1);
            assert_eq!(a.nodes.blocks.len(), 1);
            assert!(a.nodes.blocks.capacity() <= 3);
            assert_eq!(a.nodes.blocks.values().next().unwrap().live, 1);
            assert!(a.evaluate(high, |_| true));
            for old in &variables[..size - 1] {
                assert!(!a.contains(*old));
            }
            let fresh = a.fresh_choice().1;
            assert_eq!(fresh.id, size as u64);
            assert!(!a.contains(variables[0]));
            let ids: Vec<_> =
                std::iter::successors(a.nodes.next(None), |(id, _)| a.nodes.next(Some(*id)))
                    .map(|(id, _)| id)
                    .collect();
            assert_eq!(ids, [high.id, fresh.id]);
            collect(&mut a, vec![fresh], vec![high]);
            collect(&mut a, vec![], vec![]);
            #[cfg(feature = "diagnostics")]
            println!(
                "sparse size={size} phase=released gauges={:?}",
                a.slab_diagnostics()
            );
            assert_eq!(a.nodes.blocks.capacity(), 0);
        }
    }

    #[test]
    fn epoch_recreation_rejects_stale_handles_and_preserves_birth_order() {
        let mut a = Arena::default();
        let old = a.fresh_choice().1;
        let held = a.fresh_choice().1;
        collect(&mut a, vec![held], vec![]);
        let new = a.fresh_choice().1;
        assert!(new.id > old.id);
        assert!(new.id > held.id);
        assert!(!a.contains(old) && !a.contains(old.not()));
        assert!(a.contains(held) && a.contains(new));
        assert!(std::panic::catch_unwind(|| a.view(old)).is_err());
        collect(&mut a, vec![], vec![]);
        assert_eq!(a.nodes.blocks.capacity(), 0);
        assert!(a.nodes.first.is_none() && a.nodes.last.is_none());
        let born = a.fresh_choice().1;
        assert!(born.id > new.id && !a.contains(new));
    }

    #[test]
    fn interrupted_epoch_sweep_keeps_archives_and_cancelled_job_roots() {
        for (cutoff, archived) in (0..80).flat_map(|n| [(n, false), (n, true)]) {
            let mut a = Arena::default();
            let x = a.fresh_choice().1;
            let y = a.fresh_choice().1;
            let mut job = a.start(Operation::And(x, y));
            job.tick(&mut a);
            for _ in 0..64 {
                a.fresh_choice();
            }
            let mut gc = a.collect_archived(
                job.roots().collect::<Vec<_>>().into_iter(),
                if archived { vec![x] } else { vec![] },
                true,
            );
            while !matches!(gc.phase, Phase::Sweep) {
                assert!(!gc.tick(&mut a));
            }
            for _ in 0..cutoff {
                if gc.tick(&mut a) {
                    break;
                }
            }
            drop(gc);
            let fresh = a.fresh_choice().1;
            collect(&mut a, job.roots().chain([fresh]).collect(), vec![x]);
            assert!(a.evaluate(x, |_| true));
            while !job.discard_tick() {
                collect(&mut a, job.roots().collect(), vec![x]);
            }
            collect(&mut a, vec![], vec![x]);
            assert_eq!(a.node_count(), 1);
            assert!(!a.contains(y) && !a.contains(fresh));
            collect(&mut a, vec![], vec![]);
            assert_eq!(a.nodes.blocks.capacity(), 0);
        }
    }

    #[test]
    fn rotating_owner_work_is_independent_of_unchanged_boolean_closure() {
        let rotation = |size| {
            let mut a = Arena::default();
            let variables: Vec<_> = (0..size).map(|_| a.fresh_choice().1).collect();
            let mut shared = Condition::TRUE;
            for &variable in variables.iter().rev() {
                shared = finish(&mut a, Operation::And(variable, shared));
            }
            let old = a.fresh_choice().1;
            let new = a.fresh_choice().1;
            collect(&mut a, vec![new], vec![shared, old]);
            let work = collect(&mut a, vec![], vec![shared, new]);
            assert!(a.evaluate(shared, |_| true));
            assert!(!a.evaluate(shared, |choice| choice != 0));
            assert!(a.contains(new));
            assert!(!a.contains(old));
            collect(&mut a, vec![], vec![]);
            assert_eq!(a.node_count(), 0);
            work
        };
        let small = rotation(64);
        let large = rotation(512);
        assert!(
            large <= small + 64,
            "changing one owner must not revisit its unchanged closure: {small} vs {large}"
        );
    }

    #[test]
    fn releasing_a_large_archive_reclaims_its_canonical_table_storage() {
        let mut a = Arena::default();
        let variables: Vec<_> = (0..512).map(|_| a.fresh_choice().1).collect();
        let mut large = Condition::TRUE;
        for &variable in variables.iter().rev() {
            large = finish(&mut a, Operation::And(variable, large));
        }
        let small = a.fresh_choice().1;
        collect(&mut a, vec![], vec![large, small]);
        collect(&mut a, vec![], vec![small]);
        assert_eq!(a.node_count(), 1);
        assert!(
            a.unique_capacity() < 16,
            "released archive must not retain its table allocation"
        );
        assert!(a.evaluate(small, |_| true));
        assert!(!a.evaluate(small, |_| false));
    }

    #[test]
    fn interrupted_owner_changes_resume_without_retaining_unowned_nodes() {
        for already_registered in [false, true] {
            for cutoff in 0..160 {
                let mut a = Arena::default();
                let variables: Vec<_> = (0..16).map(|_| a.fresh_choice().1).collect();
                let mut shared = Condition::TRUE;
                for &variable in variables.iter().rev() {
                    shared = finish(&mut a, Operation::And(variable, shared));
                }
                let old = a.fresh_choice().1;
                let new = a.fresh_choice().1;
                if already_registered {
                    collect(&mut a, vec![new], vec![shared, old]);
                }
                let mut collector = a.collect_archived([new].into_iter(), vec![shared, old], true);
                for _ in 0..cutoff {
                    if collector.tick(&mut a) {
                        break;
                    }
                }
                drop(collector);
                collect(&mut a, vec![], vec![shared, new]);
                assert!(a.evaluate(shared, |_| true));
                assert!(!a.evaluate(shared, |id| id != 8));
                assert_eq!(
                    a.node_count(),
                    17,
                    "only the 16-node conjunction and new atom survive: registered={already_registered}, cutoff={cutoff}"
                );
                // Interrupt final-owner release, then reinstall another owner.
                let mut collector = a.collect_archived([shared, new].into_iter(), vec![], true);
                for _ in 0..cutoff {
                    if collector.tick(&mut a) {
                        break;
                    }
                }
                drop(collector);
                collect(&mut a, vec![], vec![shared, new]);
                assert_eq!(a.node_count(), 17);
                assert!(a.evaluate(shared, |_| true));
                collect(&mut a, vec![], vec![]);
                assert_eq!(a.node_count(), 0);
            }
        }
    }

    #[test]
    fn overlapping_complemented_owners_release_in_any_order_with_a_suspended_reader() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut a = Arena::default();
            let x = a.fresh_choice().1;
            let y = a.fresh_choice().1;
            let z = a.fresh_choice().1;
            let xy = finish(&mut a, Operation::And(x, y));
            let yz = finish(&mut a, Operation::And(y, z));
            let mut owners = [Some(xy), Some(xy.not()), Some(yz)];
            let mut job = a.start(Operation::Or(xy, yz));
            job.tick(&mut a);
            assert!(job.result().is_none(), "the working reader is suspended");
            for released in order {
                collect(
                    &mut a,
                    job.roots().collect(),
                    owners.iter().flatten().copied().collect(),
                );
                owners[released] = None;
                collect(
                    &mut a,
                    job.roots().collect(),
                    owners.iter().flatten().copied().collect(),
                );
                for bits in 0..8 {
                    let assignment = |id| bits & (1 << id) != 0;
                    for (index, root) in owners.iter().enumerate() {
                        if let Some(root) = root {
                            let expected = match index {
                                0 => bits & 3 == 3,
                                1 => bits & 3 != 3,
                                _ => bits & 6 == 6,
                            };
                            assert_eq!(a.evaluate(*root, assignment), expected);
                        }
                    }
                }
            }
            let result = loop {
                if let Progress::Complete(result) = job.tick(&mut a) {
                    break result;
                }
            };
            for bits in 0..8 {
                assert_eq!(
                    a.evaluate(result, |id| bits & (1 << id) != 0),
                    bits & 3 == 3 || bits & 6 == 6
                );
            }
            drop(job);
            collect(&mut a, vec![], vec![]);
            assert_eq!(a.node_count(), 0);
        }
    }
}
