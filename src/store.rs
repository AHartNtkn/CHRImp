//! Arc-owned persistent indexes. Explicit leaf tracing protects scalar payloads;
//! completed collection epochs invalidate untraced roots. Child release is deferred.
use crate::{condition::Condition, gc::GcLease};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed},
};
pub type Key = [u64; 4];
pub trait Value: Copy + Eq + Send + Sync + 'static {}
impl<T: Copy + Eq + Send + Sync + 'static> Value for T {}
static NEXT_STORE: AtomicU32 = AtomicU32::new(1);
mod substitute;
pub use substitute::Substitution;
#[derive(Default)]
struct Stats {
    #[cfg(feature = "diagnostics")]
    batch: Mutex<BatchDiagnostics>,
    allocated: AtomicUsize,
    live: AtomicUsize,
    unique: AtomicUsize,
    copied: AtomicUsize,
    pages: AtomicUsize,
    page_copies: AtomicUsize,
    page_writes: AtomicUsize,
    page_splits: AtomicUsize,
    page_merges: AtomicUsize,
    page_cursor_entries: AtomicUsize,
    page_inline_slots: AtomicUsize,
    page_shifts: AtomicUsize,
    page_runs: AtomicUsize,
    page_fallback_visits: AtomicUsize,
    prefix_calls: AtomicUsize,
    prefix_steps: AtomicUsize,
    prefix_hits: AtomicUsize,
    prefix_misses: AtomicUsize,
    prefix_evictions: AtomicUsize,
}
/// Cumulative batch preparation work; scratch peaks are per call, not retained
/// storage. Comparisons count complete exact keys, not individual words.
/// Hash requests count calls to the key's Hash implementation, including
/// promotion and growth rehashing, but not internal bucket probes.
#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct BatchDiagnostics {
    pub calls: usize,
    pub input_writes: usize,
    pub comparisons: usize,
    pub hash_requests: usize,
    pub membership_checks: usize,
    pub retained_writes: usize,
    pub scratch_tables: usize,
    pub max_input_writes: usize,
    pub scratch_peak_capacity: usize,
    pub scratch_peak_bytes: usize,
}
// The diagnostic wrapper has exactly Key's layout. Thread-local counters avoid
// adding a pointer to every hash bucket or changing scratch allocation sizes.
#[derive(Eq)]
struct BatchKey(Key);
#[cfg(feature = "diagnostics")]
thread_local! {
    static BATCH_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BATCH_HASH_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl PartialEq for BatchKey {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(feature = "diagnostics")]
        BATCH_COMPARISONS.with(|c| c.set(c.get() + 1));
        self.0 == other.0
    }
}
impl std::hash::Hash for BatchKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        #[cfg(feature = "diagnostics")]
        BATCH_HASH_REQUESTS.with(|c| c.set(c.get() + 1));
        self.0.hash(state);
    }
}
// Pages never cross a whole-word prefix or an aligned eight-key interval.
// Thus classification, copying, shifts, destruction and each update are bounded;
// sparse boundaries and all three-word dependency witnesses remain crit-bit.
const PAGE_BIT: usize = 253;
const PAGE_CAPACITY: usize = 8;
// All slots are initialized from the first scalar pair; only the occupied
// prefix is observable. This keeps Value's Copy contract without unsafe code
// or a Default/sentinel requirement on payloads.
#[derive(Clone)]
struct PageEntries<V: Value> {
    slots: [(u64, V); PAGE_CAPACITY],
    len: usize,
}
impl<V: Value> PageEntries<V> {
    fn new(first: (u64, V)) -> Self {
        Self {
            slots: [first; PAGE_CAPACITY],
            len: 1,
        }
    }
    fn as_slice(&self) -> &[(u64, V)] {
        &self.slots[..self.len]
    }
    fn insert(&mut self, position: usize, pair: (u64, V)) {
        assert!(self.len < PAGE_CAPACITY && position <= self.len);
        for i in (position..self.len).rev() {
            self.slots[i + 1] = self.slots[i];
        }
        self.slots[position] = pair;
        self.len += 1;
    }
    fn remove(&mut self, position: usize) {
        assert!(position < self.len);
        for i in position + 1..self.len {
            self.slots[i - 1] = self.slots[i];
        }
        self.len -= 1;
    }
    fn extend_from_slice(&mut self, entries: &[(u64, V)]) {
        assert!(self.len + entries.len() <= PAGE_CAPACITY);
        self.slots[self.len..self.len + entries.len()].copy_from_slice(entries);
        self.len += entries.len();
    }
}
impl<V: Value> std::ops::Deref for PageEntries<V> {
    type Target = [(u64, V)];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
impl<V: Value> std::ops::DerefMut for PageEntries<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots[..self.len]
    }
}
const RELEASE_CAPACITY: usize = 16;
type Pair<V> = (Option<Arc<Record<V>>>, Option<Arc<Record<V>>>);
struct Block<V: Value> {
    pairs: [Option<Pair<V>>; RELEASE_CAPACITY],
    len: usize,
    next: Option<Box<Block<V>>>,
}
impl<V: Value> Block<V> {
    fn new() -> Self {
        Self {
            pairs: std::array::from_fn(|_| None),
            len: 0,
            next: None,
        }
    }
    fn push(&mut self, pair: Pair<V>) {
        self.pairs[self.len] = Some(pair);
        self.len += 1;
    }
    fn pop(&mut self) -> Pair<V> {
        self.len -= 1;
        self.pairs[self.len].take().unwrap()
    }
}
struct QueueState<V: Value> {
    inline: Block<V>,
    overflow: Option<Box<Block<V>>>,
}
impl<V: Value> QueueState<V> {
    fn push(&mut self, pair: Pair<V>) {
        if let Some(block) = self.overflow.as_mut() {
            if block.len < RELEASE_CAPACITY {
                block.push(pair);
                return;
            }
        } else if self.inline.len < RELEASE_CAPACITY {
            self.inline.push(pair);
            return;
        }
        let mut block = Box::new(Block::new());
        block.push(pair);
        block.next = self.overflow.take();
        self.overflow = Some(block);
    }
    fn pop(&mut self) -> Option<Pair<V>> {
        if let Some(block) = self.overflow.as_mut() {
            let pair = block.pop();
            if block.len == 0 {
                let mut empty = self.overflow.take().unwrap();
                self.overflow = empty.next.take();
                // The empty block owns neither child references nor a chain.
            }
            Some(pair)
        } else if self.inline.len != 0 {
            Some(self.inline.pop())
        } else {
            None
        }
    }
}
struct Queue<V: Value> {
    state: Mutex<QueueState<V>>,
    count: AtomicUsize,
}
impl<V: Value> Queue<V> {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                inline: Block::new(),
                overflow: None,
            }),
            count: AtomicUsize::new(0),
        }
    }
    fn push(&self, left: Option<Arc<Record<V>>>, right: Option<Arc<Record<V>>>) {
        let mut state = self.state.lock().unwrap();
        state.push((left, right));
        self.count.fetch_add(1, Relaxed);
    }
    fn pop(&self) -> Option<Pair<V>> {
        let mut state = self.state.lock().unwrap();
        let pair = state.pop()?;
        self.count.fetch_sub(1, Relaxed);
        Some(pair)
    }
}
impl<V: Value> Drop for Queue<V> {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap();
        while let Some(pair) = state.pop() {
            drop(pair);
        }
    }
}
#[derive(Clone)]
enum Node<V: Value> {
    Leaf {
        key: Key,
        value: V,
    },
    Page {
        prefix: Key,
        // Keep the bounded payload inline within its own allocation so sparse
        // Leaf/Branch records do not inherit eight slots in their enum size.
        entries: Box<PageEntries<V>>,
    },
    Branch {
        prefix: Key,
        bit: u8,
        left: Root<V>,
        right: Root<V>,
    },
}
struct Record<V: Value> {
    leaves: usize,
    node: Node<V>,
    marked: AtomicU64,
    archived: AtomicU64,
    queue: Weak<Queue<V>>,
    stats: Arc<Stats>,
}
impl<V: Value> Record<V> {
    fn children(&mut self) -> (Option<Arc<Self>>, Option<Arc<Self>>) {
        match &mut self.node {
            Node::Leaf { .. } | Node::Page { .. } => (None, None),
            Node::Branch { left, right, .. } => (left.node.take(), right.node.take()),
        }
    }
}
impl<V: Value> Drop for Record<V> {
    fn drop(&mut self) {
        self.stats.live.fetch_sub(1, Relaxed);
        let (left, right) = self.children();
        if left.is_none() && right.is_none() {
            return;
        }
        if let Some(queue) = self.queue.upgrade() {
            queue.push(left, right);
        } else {
            let mut pending = Vec::new();
            pending.extend(left);
            pending.extend(right);
            while let Some(node) = pending.pop() {
                if let Some(mut node) = Arc::into_inner(node) {
                    let (a, b) = node.children();
                    pending.extend(a);
                    pending.extend(b);
                }
            }
        }
    }
}
#[derive(Clone)]
pub struct Root<V: Value = Condition> {
    owner: u32,
    node: Option<Arc<Record<V>>>,
}
impl<V: Value> Root<V> {
    fn empty() -> Self {
        Self {
            owner: 0,
            node: None,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.node.is_none()
    }
}
/// Non-owning identity witness. Weak ownership prevents in-place mutation of
/// the witnessed node, without retaining its payload or descendants.
pub(crate) struct WeakRoot<V: Value = Condition> {
    owner: u32,
    node: Option<Weak<Record<V>>>,
}
impl<V: Value> WeakRoot<V> {
    pub(crate) fn valid(&self, store: &Store<V>) -> bool {
        match &self.node {
            None => true,
            Some(node) => node.upgrade().is_some_and(|node| {
                store.contains(&Root {
                    owner: self.owner,
                    node: Some(node),
                })
            }),
        }
    }
    pub(crate) fn matches(&self, root: &Root<V>) -> bool {
        self.owner == root.owner
            && match (&self.node, &root.node) {
                (None, None) => true,
                (Some(a), Some(b)) => a.as_ptr() == Arc::as_ptr(b),
                _ => false,
            }
    }
}
impl<V: Value> Root<V> {
    pub(crate) fn downgrade(&self) -> WeakRoot<V> {
        WeakRoot {
            owner: self.owner,
            node: self.node.as_ref().map(Arc::downgrade),
        }
    }
}
impl<V: Value> Default for Root<V> {
    fn default() -> Self {
        Self::empty()
    }
}
impl<V: Value> PartialEq for Root<V> {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner
            && match (&self.node, &other.node) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}
impl<V: Value> Eq for Root<V> {}
impl<V: Value> std::fmt::Debug for Root<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Root({}, {:?})",
            self.owner,
            self.node.as_ref().map(Arc::as_ptr)
        )
    }
}
pub struct Store<V: Value> {
    owner: u32,
    epoch: u64,
    completed: u64,
    archive_epoch: u64,
    archive_completed: u64,
    frozen: Arc<AtomicBool>,
    queue: Arc<Queue<V>>,
    stats: Arc<Stats>,
    prefix_memo: Mutex<HashMap<PrefixKey, PrefixEntry<V>>>,
}
const PREFIX_MEMO_LIMIT: usize = 32;
#[derive(Hash, PartialEq, Eq)]
struct PrefixKey {
    allocation: usize,
    prefix: Key,
    words: usize,
}
struct PrefixEntry<V: Value> {
    input: WeakRoot<V>,
    result: WeakRoot<V>,
}
impl<V: Value> Default for Store<V> {
    fn default() -> Self {
        Self {
            owner: NEXT_STORE
                .fetch_update(Relaxed, Relaxed, |x| x.checked_add(1))
                .expect("index identity exhausted"),
            epoch: 0,
            completed: 0,
            archive_epoch: 1,
            archive_completed: 1,
            frozen: Arc::new(AtomicBool::new(false)),
            queue: Arc::new(Queue::new()),
            stats: Arc::new(Stats::default()),
            prefix_memo: Mutex::new(HashMap::new()),
        }
    }
}
fn right(key: &Key, bit: u8) -> bool {
    key[bit as usize / 64] & (1 << (63 - bit % 64)) != 0
}
fn page_key(mut prefix: Key, tail: u64) -> Key {
    prefix[3] = tail;
    prefix
}
fn difference(a: &Key, b: &Key) -> Option<u8> {
    a.iter().zip(b).enumerate().find_map(|(i, (&a, &b))| {
        (a != b).then(|| (64 * i + (a ^ b).leading_zeros() as usize) as u8)
    })
}
fn bounds(mut low: Key, bit: u8) -> (Key, Key) {
    let word = bit as usize / 64;
    let mask = if bit.is_multiple_of(64) {
        0
    } else {
        u64::MAX << (64 - bit % 64)
    };
    low[word] &= mask;
    let mut high = low;
    high[word] |= !mask;
    for i in word + 1..4 {
        low[i] = 0;
        high[i] = u64::MAX;
    }
    (low, high)
}
impl<V: Value> Store<V> {
    fn append_page_entries(&self, root: &Root<V>, result: &mut PageEntries<V>) -> Key {
        match &self.record(root).node {
            Node::Leaf { key, value } => {
                result.insert(result.len(), (key[3], *value));
                self.stats.page_copies.fetch_add(1, Relaxed);
                *key
            }
            Node::Page { prefix, entries } => {
                result.extend_from_slice(entries);
                self.stats.page_copies.fetch_add(entries.len(), Relaxed);
                *prefix
            }
            Node::Branch { .. } => unreachable!("page crosses sparse boundary"),
        }
    }
    fn page_batch(&mut self, mut root: Root<V>, writes: &[(Key, Option<V>)]) -> Root<V> {
        // Called only after membership/no-op checks and aligned interval checks.
        // Reuse unique page storage; copying is bounded by eight scalar pairs.
        if root.is_empty() {
            let mut pairs = writes
                .iter()
                .filter_map(|&(key, value)| value.map(|v| (key[3], v)));
            let Some(first) = pairs.next() else {
                return self.empty();
            };
            let mut entries = PageEntries::new(first);
            self.stats
                .page_inline_slots
                .fetch_add(PAGE_CAPACITY, Relaxed);
            for pair in pairs {
                entries.insert(entries.len(), pair);
            }
            return match entries.as_slice() {
                [] => self.empty(),
                &[(tail, value)] => self.allocate(Node::Leaf {
                    key: page_key(writes[0].0, tail),
                    value,
                }),
                _ => self.allocate(Node::Page {
                    prefix: writes[0].0,
                    entries: Box::new(entries),
                }),
            };
        }
        self.unique(&mut root);
        let record = Arc::get_mut(root.node.as_mut().unwrap()).unwrap();
        if let Node::Leaf { key, value } = record.node {
            let entries = PageEntries::new((key[3], value));
            self.stats
                .page_inline_slots
                .fetch_add(PAGE_CAPACITY, Relaxed);
            record.node = Node::Page {
                prefix: key,
                entries: Box::new(entries),
            };
            self.stats.page_merges.fetch_add(1, Relaxed);
        }
        let Node::Page { prefix, entries } = &mut record.node else {
            unreachable!()
        };
        for &(key, value) in writes {
            self.stats.page_writes.fetch_add(1, Relaxed);
            match (
                entries.binary_search_by_key(&key[3], |&(tail, _)| tail),
                value,
            ) {
                (Ok(i), Some(v)) => entries[i].1 = v,
                (Ok(i), None) => {
                    self.stats
                        .page_shifts
                        .fetch_add(entries.len() - i - 1, Relaxed);
                    entries.remove(i);
                }
                (Err(i), Some(v)) => {
                    self.stats.page_shifts.fetch_add(entries.len() - i, Relaxed);
                    entries.insert(i, (key[3], v));
                }
                (Err(_), None) => {}
            }
        }
        assert!(entries.len() <= PAGE_CAPACITY);
        record.leaves = entries.len();
        match entries.as_slice() {
            [] => return self.empty(),
            &[(tail, value)] => {
                record.node = Node::Leaf {
                    key: page_key(*prefix, tail),
                    value,
                }
            }
            _ => {}
        }
        root
    }
    pub(crate) fn assert_mutable(&self) {
        GcLease::assert_mutable(&self.frozen);
    }
    pub fn empty(&self) -> Root<V> {
        Root::empty()
    }
    pub fn contains(&self, root: &Root<V>) -> bool {
        root.is_empty()
            || (root.owner == self.owner
                && (root.node.as_ref().unwrap().marked.load(Relaxed) >= self.completed
                    || root.node.as_ref().unwrap().archived.load(Relaxed)
                        >= self.archive_completed))
    }
    pub fn allocations(&self) -> usize {
        self.stats.allocated.load(Relaxed)
    }
    pub fn node_count(&self) -> usize {
        self.stats.live.load(Relaxed)
    }
    pub fn release_batches(&self) -> usize {
        self.queue.count.load(Relaxed)
    }
    pub fn release_pending(&self) -> bool {
        self.queue.count.load(Relaxed) != 0
    }
    pub fn release_tick(&mut self) -> bool {
        if let Some((left, right)) = self.queue.pop() {
            drop(left);
            drop(right);
            false
        } else {
            true
        }
    }
    pub(crate) fn owner(&self) -> u32 {
        self.owner
    }
    pub fn mutation_counts(&self) -> (usize, usize) {
        (
            self.stats.unique.load(Relaxed),
            self.stats.copied.load(Relaxed),
        )
    }
    #[cfg(feature = "diagnostics")]
    pub fn batch_diagnostics(&self) -> BatchDiagnostics {
        *self.stats.batch.lock().unwrap()
    }
    /// Page allocations, copied entries, writes, sparse-boundary splits,
    /// packed merges, cursor entries, initialized/copied inline slots, shifted
    /// entries, page-run transfers and scalar fallback visits (cumulative).
    pub fn page_counts(&self) -> [usize; 10] {
        [
            &self.stats.pages,
            &self.stats.page_copies,
            &self.stats.page_writes,
            &self.stats.page_splits,
            &self.stats.page_merges,
            &self.stats.page_cursor_entries,
            &self.stats.page_inline_slots,
            &self.stats.page_shifts,
            &self.stats.page_runs,
            &self.stats.page_fallback_visits,
        ]
        .map(|counter| counter.load(Relaxed))
    }
    /// Prefix requests and actual path-node inspections (cumulative).
    pub fn prefix_counts(&self) -> [usize; 2] {
        [
            self.stats.prefix_calls.load(Relaxed),
            self.stats.prefix_steps.load(Relaxed),
        ]
    }
    /// Hits, misses, evicted entries, current entries and table capacity.
    pub fn prefix_memo_counts(&self) -> [usize; 5] {
        let memo = self.prefix_memo.lock().unwrap();
        [
            self.stats.prefix_hits.load(Relaxed),
            self.stats.prefix_misses.load(Relaxed),
            self.stats.prefix_evictions.load(Relaxed),
            memo.len(),
            memo.capacity(),
        ]
    }
    fn record<'a>(&self, root: &'a Root<V>) -> &'a Record<V> {
        assert!(
            self.contains(root) && !root.is_empty(),
            "stale or foreign index root"
        );
        root.node.as_deref().unwrap()
    }
    fn node(&self, root: &Root<V>) -> Node<V> {
        if matches!(self.record(root).node, Node::Page { .. }) {
            self.stats
                .page_inline_slots
                .fetch_add(PAGE_CAPACITY, Relaxed);
        }
        self.record(root).node.clone()
    }
    fn allocate(&mut self, mut node: Node<V>) -> Root<V> {
        if let Node::Branch {
            bit, left, right, ..
        } = &node
            && *bit as usize >= PAGE_BIT
        {
            let first = match &self.record(left).node {
                Node::Leaf { key, value } => (key[3], *value),
                Node::Page { entries, .. } => entries[0],
                Node::Branch { .. } => unreachable!(),
            };
            let mut entries = PageEntries::new(first);
            self.stats
                .page_inline_slots
                .fetch_add(PAGE_CAPACITY, Relaxed);
            entries.len = 0;
            let prefix = self.append_page_entries(left, &mut entries);
            self.append_page_entries(right, &mut entries);
            self.stats.page_merges.fetch_add(1, Relaxed);
            node = Node::Page {
                prefix,
                entries: Box::new(entries),
            };
        }
        if matches!(node, Node::Page { .. }) {
            self.stats.pages.fetch_add(1, Relaxed);
        }
        let leaves = Self::leaf_count(&node);
        self.stats.allocated.fetch_add(1, Relaxed);
        self.stats.live.fetch_add(1, Relaxed);
        Root {
            owner: self.owner,
            node: Some(Arc::new(Record {
                leaves,
                node,
                marked: AtomicU64::new(self.epoch),
                archived: AtomicU64::new(0),
                queue: Arc::downgrade(&self.queue),
                stats: self.stats.clone(),
            })),
        }
    }
    fn unique(&mut self, root: &mut Root<V>) {
        assert!(self.contains(root));
        if Arc::get_mut(root.node.as_mut().unwrap()).is_some() {
            let record = root.node.as_ref().unwrap();
            record.archived.store(0, Relaxed);
            record.marked.store(self.epoch, Relaxed);
            self.stats.unique.fetch_add(1, Relaxed);
        } else {
            self.stats.copied.fetch_add(1, Relaxed);
            if let Node::Page { entries, .. } = &self.record(root).node {
                self.stats.page_copies.fetch_add(entries.len(), Relaxed);
            }
            *root = self.allocate(self.node(root));
        }
    }
    fn leaf_count(node: &Node<V>) -> usize {
        match node {
            Node::Leaf { .. } => 1,
            Node::Page { entries, .. } => entries.len(),
            Node::Branch { left, right, .. } => {
                left.node.as_ref().map_or(0, |n| n.leaves)
                    + right.node.as_ref().map_or(0, |n| n.leaves)
            }
        }
    }
    fn refresh(root: &mut Root<V>) {
        let record = Arc::get_mut(root.node.as_mut().unwrap()).unwrap();
        record.leaves = Self::leaf_count(&record.node);
    }
    /// Count a key interval via cached subtrees; only the two boundary paths descend.
    pub fn count(&self, root: &Root<V>, low: Key, high: Key) -> usize {
        assert!(self.contains(root));
        if root.is_empty() || low > high {
            return 0;
        }
        let record = self.record(root);
        match &record.node {
            Node::Leaf { key, .. } => usize::from(*key >= low && *key <= high),
            Node::Page { prefix, entries } => {
                let begin = entries.partition_point(|&(tail, _)| page_key(*prefix, tail) < low);
                let end = entries.partition_point(|&(tail, _)| page_key(*prefix, tail) <= high);
                end - begin
            }
            Node::Branch {
                prefix,
                bit,
                left,
                right,
            } => {
                let (a, b) = bounds(*prefix, *bit);
                if a > high || b < low {
                    0
                } else if low <= a && b <= high {
                    record.leaves
                } else {
                    self.count(left, low, high) + self.count(right, low, high)
                }
            }
        }
    }
    pub fn get(&self, root: &Root<V>, key: &Key) -> Option<V> {
        let mut r = root;
        while !r.is_empty() {
            match &self.record(r).node {
                Node::Leaf { key: found, value } => return (found == key).then_some(*value),
                Node::Page { prefix, entries } => {
                    if prefix[..3] != key[..3] {
                        return None;
                    }
                    return entries
                        .binary_search_by_key(&key[3], |&(tail, _)| tail)
                        .ok()
                        .map(|i| entries[i].1);
                }
                Node::Branch {
                    bit,
                    left,
                    right: rgt,
                    ..
                } => r = if right(key, *bit) { rgt } else { left },
            }
        }
        None
    }
    pub fn insert(&mut self, root: Root<V>, key: Key, value: V) -> Root<V> {
        self.assert_mutable();
        assert!(self.contains(&root), "stale or foreign index root");
        if self.get(&root, &key) == Some(value) {
            return root;
        }
        self.insert_node(root, key, value)
    }
    /// Apply ordered writes privately and publish one root. None removes a key;
    /// the last write wins. The caller supplies scratch storage, which is
    /// reordered/compacted. Reverse exact-key coalescing takes expected linear
    /// work and temporary storage bounded by the distinct keys. Finalization
    /// remains synchronous. No roots or
    /// scalar values escape to a suspended overlay, so the ordinary collector
    /// lease and deferred child-release protocol suffice.
    pub fn batch(&mut self, root: Root<V>, writes: &mut [(Key, Option<V>)]) -> Root<V> {
        self.assert_mutable();
        assert!(self.contains(&root), "stale or foreign index root");
        let mut end = writes.len();
        #[cfg(feature = "diagnostics")]
        let mut d = self.stats.batch.lock().unwrap();
        #[cfg(feature = "diagnostics")]
        {
            d.calls += 1;
            d.input_writes += writes.len();
            d.max_input_writes = d.max_input_writes.max(writes.len());
        }
        // Eight exact keys stay on the stack: Graph's bounded update batches
        // need no table, and long runs over a few keys need no hashing either.
        let mut inline = [[0; 4]; 8];
        let mut inline_len = 0;
        let mut seen: Option<HashSet<BatchKey>> = None;
        #[cfg(feature = "diagnostics")]
        BATCH_COMPARISONS.with(|c| c.set(0));
        #[cfg(feature = "diagnostics")]
        BATCH_HASH_REQUESTS.with(|c| c.set(0));
        for i in (0..writes.len()).rev() {
            let (key, value) = writes[i];
            let fresh = if let Some(seen) = seen.as_mut() {
                seen.insert(BatchKey(key))
            } else {
                if inline[..inline_len].iter().any(|k| {
                    #[cfg(feature = "diagnostics")]
                    {
                        d.comparisons += 1;
                    }
                    *k == key
                }) {
                    false
                } else if inline_len < inline.len() {
                    inline[inline_len] = key;
                    inline_len += 1;
                    true
                } else {
                    // Construct the randomized hasher only on promotion.
                    let mut promoted = HashSet::with_capacity(inline.len() + 1);
                    for k in inline {
                        promoted.insert(BatchKey(k));
                    }
                    promoted.insert(BatchKey(key));
                    seen = Some(promoted);
                    true
                }
            };
            // Include final no-ops: they still supersede earlier writes.
            if !fresh {
                continue;
            }
            #[cfg(feature = "diagnostics")]
            {
                d.membership_checks += 1;
            }
            if self.get(&root, &key) == value {
                continue;
            }
            // Compact into the already-read suffix, never the unread prefix.
            end -= 1;
            writes[end] = (key, value);
        }
        let len = writes.len() - end;
        #[cfg(feature = "diagnostics")]
        {
            d.comparisons += BATCH_COMPARISONS.with(std::cell::Cell::get);
            d.hash_requests += BATCH_HASH_REQUESTS.with(std::cell::Cell::get);
            let capacity = seen.as_ref().map_or(0, HashSet::capacity);
            d.scratch_tables += usize::from(capacity != 0);
            d.scratch_peak_capacity = d.scratch_peak_capacity.max(capacity);
            // Current std HashSet bucket/control layout estimate. The measured
            // allocator includes actual growth traffic and is authoritative.
            let buckets = if capacity == 0 {
                0
            } else {
                (capacity + 1).next_power_of_two()
            };
            let bytes = if buckets == 0 {
                0
            } else {
                buckets * (size_of::<Key>() + 1) + 16
            };
            d.scratch_peak_bytes = d.scratch_peak_bytes.max(bytes);
            d.retained_writes += len;
            drop(d);
        }
        drop(seen);
        writes.copy_within(end.., 0);
        let writes = &mut writes[..len];
        writes.sort_unstable_by_key(|&(key, _)| key);
        self.batch_node(root, writes)
    }

    fn batch_node(&mut self, mut root: Root<V>, writes: &[(Key, Option<V>)]) -> Root<V> {
        if writes.is_empty() {
            return root;
        }
        let (prefix, bit) = if root.is_empty() {
            (writes[0].0, 256)
        } else {
            let record = self.record(&root);
            // Every deletion was verified present before descending. A whole
            // removed subtree needs neither copying nor visits to descendants.
            if record.leaves == writes.len() && writes.iter().all(|&(_, v)| v.is_none()) {
                return self.empty();
            }
            match &record.node {
                Node::Leaf { key, .. } => (*key, 256),
                Node::Page { prefix, .. } => (*prefix, PAGE_BIT),
                Node::Branch { prefix, bit, .. } => (*prefix, *bit as usize),
            }
        };
        let split = bit
            .min(difference(&prefix, &writes[0].0).map_or(256, usize::from))
            .min(difference(&writes[0].0, &writes[writes.len() - 1].0).map_or(256, usize::from));
        if split >= PAGE_BIT && split != 256 {
            return self.page_batch(root, writes);
        }
        if bit == PAGE_BIT && split < PAGE_BIT {
            self.stats.page_splits.fetch_add(1, Relaxed);
        }
        if split == 256 {
            let (key, value) = writes[0];
            let Some(value) = value else {
                return self.empty();
            };
            if root.is_empty() {
                return self.allocate(Node::Leaf { key, value });
            }
            self.unique(&mut root);
            Arc::get_mut(root.node.as_mut().unwrap()).unwrap().node = Node::Leaf { key, value };
            return root;
        }
        let split = split as u8;
        let middle = writes.partition_point(|&(key, _)| !right(&key, split));
        let (low, high) = writes.split_at(middle);
        if split as usize == bit && !root.is_empty() {
            self.unique(&mut root);
            let Node::Branch { left, right, .. } =
                &mut Arc::get_mut(root.node.as_mut().unwrap()).unwrap().node
            else {
                unreachable!()
            };
            let a = std::mem::take(left);
            let b = std::mem::take(right);
            let a = self.batch_node(a, low);
            let b = self.batch_node(b, high);
            if a.is_empty() {
                return b;
            }
            if b.is_empty() {
                return a;
            }
            let Node::Branch { left, right, .. } =
                &mut Arc::get_mut(root.node.as_mut().unwrap()).unwrap().node
            else {
                unreachable!()
            };
            *left = a;
            *right = b;
            Self::refresh(&mut root);
            root
        } else {
            let (a, b) = if right(&prefix, split) {
                (self.empty(), root)
            } else {
                (root, self.empty())
            };
            let left = self.batch_node(a, low);
            let right = self.batch_node(b, high);
            if left.is_empty() {
                return right;
            }
            if right.is_empty() {
                return left;
            }
            self.allocate(Node::Branch {
                prefix,
                bit: split,
                left,
                right,
            })
        }
    }
    fn insert_node(&mut self, mut root: Root<V>, key: Key, value: V) -> Root<V> {
        if root.is_empty() {
            return self.allocate(Node::Leaf { key, value });
        }
        let (prefix, bit) = match &self.record(&root).node {
            Node::Leaf { key, .. } => (*key, 256),
            Node::Page { prefix, .. } => (*prefix, PAGE_BIT),
            Node::Branch { prefix, bit, .. } => (*prefix, *bit as usize),
        };
        if bit == PAGE_BIT || (bit == 256 && prefix != key) {
            if difference(&prefix, &key).is_none_or(|d| d as usize >= PAGE_BIT) {
                return self.page_batch(root, &[(key, Some(value))]);
            }
            if bit == PAGE_BIT {
                self.stats.page_splits.fetch_add(1, Relaxed);
            }
        }
        if let Some(split) = difference(&prefix, &key).filter(|&x| (x as usize) < bit) {
            let new = self.allocate(Node::Leaf { key, value });
            let (left, rgt) = if right(&key, split) {
                (root, new)
            } else {
                (new, root)
            };
            return self.allocate(Node::Branch {
                prefix: key,
                bit: split,
                left,
                right: rgt,
            });
        }
        self.unique(&mut root);
        let record = Arc::get_mut(root.node.as_mut().unwrap()).unwrap();
        match &mut record.node {
            Node::Leaf { value: v, .. } => *v = value,
            Node::Page { .. } => unreachable!(),
            Node::Branch {
                bit,
                left,
                right: rgt,
                ..
            } => {
                let child = if right(&key, *bit) { rgt } else { left };
                let input = std::mem::take(child);
                *child = self.insert_node(input, key, value);
            }
        }
        Self::refresh(&mut root);
        root
    }
    pub fn remove(&mut self, root: Root<V>, key: &Key) -> Root<V> {
        self.assert_mutable();
        assert!(self.contains(&root), "stale or foreign index root");
        if self.get(&root, key).is_none() {
            return root;
        }
        self.remove_node(root, key)
    }
    fn remove_node(&mut self, mut root: Root<V>, key: &Key) -> Root<V> {
        if matches!(self.record(&root).node, Node::Page { .. }) {
            return self.page_batch(root, &[(*key, None)]);
        }
        if matches!(self.record(&root).node, Node::Leaf { .. }) {
            return Root::empty();
        }
        self.unique(&mut root);
        let Node::Branch {
            bit,
            left,
            right: rgt,
            ..
        } = &mut Arc::get_mut(root.node.as_mut().unwrap()).unwrap().node
        else {
            unreachable!()
        };
        let (child, sibling) = if right(key, *bit) {
            (rgt, left)
        } else {
            (left, rgt)
        };
        let input = std::mem::take(child);
        *child = self.remove_node(input, key);
        if child.is_empty() {
            return std::mem::take(sibling);
        }
        Self::refresh(&mut root);
        root
    }
    /// Exact subtree for a prefix of at most three words (at most 192 path steps).
    /// Unrelated prefix updates preserve its identity even when the root changes.
    pub(crate) fn prefix_root(&self, root: &Root<V>, prefix: Key, words: usize) -> Root<V> {
        assert!(words <= 3 && self.contains(root));
        self.stats.prefix_calls.fetch_add(1, Relaxed);
        if root.is_empty() {
            return self.empty();
        }
        let mut normalized = prefix;
        normalized[words..].fill(0);
        let key = PrefixKey {
            allocation: Arc::as_ptr(root.node.as_ref().unwrap()) as usize,
            prefix: normalized,
            words,
        };
        let mut memo = self.prefix_memo.lock().unwrap();
        if let Some(entry) = memo.get(&key)
            && entry.input.matches(root)
        {
            let result = match &entry.result.node {
                None => Some(self.empty()),
                Some(node) => node.upgrade().map(|node| Root {
                    owner: entry.result.owner,
                    node: Some(node),
                }),
            };
            if let Some(result) = result.filter(|result| self.contains(result)) {
                self.stats.prefix_hits.fetch_add(1, Relaxed);
                return result;
            }
        }
        self.stats.prefix_misses.fetch_add(1, Relaxed);
        let result = self.prefix_root_uncached(root, prefix, words);
        if memo.len() == PREFIX_MEMO_LIMIT {
            self.stats.prefix_evictions.fetch_add(memo.len(), Relaxed);
            memo.clear();
        }
        // The weak input prevents in-place mutation and allocation address reuse.
        // Neither witness keeps payloads alive. Empty results are exact too.
        memo.insert(
            key,
            PrefixEntry {
                input: root.downgrade(),
                result: result.downgrade(),
            },
        );
        result
    }
    fn prefix_root_uncached(&self, root: &Root<V>, prefix: Key, words: usize) -> Root<V> {
        let mut node = root;
        while !node.is_empty() {
            self.stats.prefix_steps.fetch_add(1, Relaxed);
            let (key, bit) = match &self.record(node).node {
                Node::Leaf { key, .. } => (*key, 256),
                Node::Page { prefix, .. } => (*prefix, PAGE_BIT),
                Node::Branch { prefix, bit, .. } => (*prefix, *bit as usize),
            };
            if difference(&key, &prefix).is_some_and(|d| (d as usize) < bit.min(words * 64)) {
                return self.empty();
            }
            if bit >= words * 64 {
                return node.clone();
            }
            let Node::Branch {
                bit,
                left,
                right: rgt,
                ..
            } = &self.record(node).node
            else {
                unreachable!()
            };
            node = if right(&prefix, *bit) { rgt } else { left };
        }
        self.empty()
    }
    /// Split a large immutable prefix in key order. A weak witness of either
    /// child is an allocation-generation certificate: it prevents in-place
    /// mutation and address reuse without keeping descendant payloads alive.
    pub(crate) fn split_prefix(&self, root: &Root<V>, limit: usize) -> Option<[Root<V>; 2]> {
        assert!(self.contains(root), "stale or foreign prefix root");
        if root.is_empty() || self.record(root).leaves <= limit {
            return None;
        }
        match &self.record(root).node {
            Node::Branch { left, right, .. } => Some([left.clone(), right.clone()]),
            _ => None,
        }
    }
    pub fn range(&self, root: Root<V>, low: Key, high: Key) -> Cursor<V> {
        assert!(self.contains(&root), "stale or foreign index root");
        Cursor {
            current: PageRun {
                root: root.clone(),
                position: 0,
                end: 0,
            },
            root,
            pending: Vec::new(),
            low,
            high,
            visits: 0,
        }
    }
    pub fn filter(&self, root: Root<V>) -> Filter<V> {
        assert!(self.contains(&root), "stale or foreign index root");
        Filter {
            owner: self.owner,
            base: root.clone(),
            frame: Some(FilterFrame::Visit(root)),
            frames: Vec::new(),
            last: Root::empty(),
            leaf: None,
        }
    }
    pub fn collect<I: Iterator<Item = Root<V>>>(&mut self, roots: I) -> Collector<I, V> {
        self.collect_archived(roots, vec![], true)
    }
    // Internal callers finish registration, or reset and supply all remaining
    // archive roots after an abort. Ordinary public collection always resets.
    pub(crate) fn collect_archived<I: Iterator<Item = Root<V>>>(
        &mut self,
        roots: I,
        archived: Vec<Root<V>>,
        reset: bool,
    ) -> Collector<I, V> {
        let lease = GcLease::acquire(&self.frozen);
        let memo = self.prefix_memo.get_mut().unwrap();
        self.stats.prefix_evictions.fetch_add(memo.len(), Relaxed);
        // Release weak allocation headers and table backing at the lifecycle barrier.
        *memo = HashMap::new();
        if reset {
            self.archive_epoch = self
                .archive_epoch
                .checked_add(1)
                .expect("archive epoch exhausted");
        }
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("index collection epoch exhausted");
        Collector {
            owner: self.owner,
            epoch: self.epoch,
            _lease: lease,
            roots,
            archiving: !archived.is_empty(),
            archived: archived.into_iter(),
            pending: Vec::new(),
            marking: true,
            done: false,
            page: None,
        }
    }
}
pub struct Cursor<V: Value = Condition> {
    root: Root<V>,
    current: PageRun<V>,
    pending: Vec<Root<V>>,
    low: Key,
    high: Key,
    visits: u64,
}
// Move a contiguous interval's authority into a continuation, not a temporary
// scalar buffer. Its owning root remains visible to tracing through the cursor
// root/filter base. Scalar yields preserve every existing suspension point.
#[derive(Clone)]
struct PageRun<V: Value> {
    root: Root<V>,
    position: u8,
    end: u8,
}
impl<V: Value> PageRun<V> {
    fn next(&mut self, store: &Store<V>) -> Option<(Key, V)> {
        let Node::Page { prefix, entries } = &store.record(&self.root).node else {
            unreachable!()
        };
        if self.position == self.end {
            return None;
        }
        let (tail, value) = entries[self.position as usize];
        self.position += 1;
        Some((page_key(*prefix, tail), value))
    }
}
impl<V: Value> Cursor<V> {
    pub fn root(&self) -> Root<V> {
        self.root.clone()
    }
    pub fn visits(&self) -> u64 {
        self.visits
    }
    pub fn next(&mut self, store: &Store<V>) -> Option<(Key, V)> {
        assert!(store.contains(&self.root), "stale or foreign cursor root");
        loop {
            if self.current.end != 0 {
                if let Some(pair) = self.current.next(store) {
                    store.stats.page_cursor_entries.fetch_add(1, Relaxed);
                    return Some(pair);
                }
                self.current.root = Root::empty();
                self.current.end = 0;
            }
            let root = if self.current.root.is_empty() {
                self.pending.pop()?
            } else {
                std::mem::take(&mut self.current.root)
            };
            self.visits += 1;
            match &store.record(&root).node {
                Node::Leaf { key, value } => {
                    store.stats.page_fallback_visits.fetch_add(1, Relaxed);
                    if *key >= self.low && *key <= self.high {
                        return Some((*key, *value));
                    }
                }
                Node::Page { prefix, entries } => {
                    let position =
                        entries.partition_point(|&(tail, _)| page_key(*prefix, tail) < self.low);
                    let end =
                        entries.partition_point(|&(tail, _)| page_key(*prefix, tail) <= self.high);
                    if position < end {
                        self.current = PageRun {
                            root,
                            position: position as u8,
                            end: end as u8,
                        };
                        store.stats.page_runs.fetch_add(1, Relaxed);
                    }
                }
                Node::Branch {
                    prefix,
                    bit,
                    left,
                    right,
                } => {
                    store.stats.page_fallback_visits.fetch_add(1, Relaxed);
                    let (low, high) = bounds(*prefix, *bit);
                    if low <= self.high && high >= self.low {
                        self.pending.push(right.clone());
                        self.current.root = left.clone();
                    }
                }
            }
        }
    }
}
#[derive(Clone)]
enum FilterFrame<V: Value> {
    Visit(Root<V>),
    AfterLeft { root: Root<V> },
    AfterRight { root: Root<V>, left: Root<V> },
    Page { run: PageRun<V>, result: Root<V> },
}
struct FilterLeaf<V: Value> {
    root: Root<V>,
    key: Key,
    value: V,
    replacement: Option<Option<V>>,
}
pub struct Filter<V: Value> {
    owner: u32,
    base: Root<V>,
    frame: Option<FilterFrame<V>>,
    frames: Vec<FilterFrame<V>>,
    last: Root<V>,
    leaf: Option<FilterLeaf<V>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterStatus<V: Value> {
    Pending,
    Leaf { key: Key, value: V },
    Complete(Root<V>),
}
impl<V: Value> Filter<V> {
    pub fn replace(&mut self, value: Option<V>) {
        let leaf = self.leaf.as_mut().expect("filter replace requires a Leaf");
        assert!(leaf.replacement.is_none(), "filter Leaf already replaced");
        leaf.replacement = Some(value);
    }
    pub fn roots(&self) -> impl Iterator<Item = Root<V>> + '_ {
        [self.base.clone(), self.last.clone()].into_iter().chain(
            self.frames
                .iter()
                .chain(self.frame.iter())
                .filter_map(|f| match f {
                    FilterFrame::AfterRight { left, .. } => Some(left.clone()),
                    FilterFrame::Page { result, .. } => Some(result.clone()),
                    _ => None,
                }),
        )
    }
    pub fn values(&self) -> impl Iterator<Item = V> + '_ {
        self.leaf.iter().filter_map(|l| l.replacement.flatten())
    }
    fn returned(&mut self, root: Root<V>) -> FilterStatus<V> {
        self.last = root;
        self.frame = self.frames.pop();
        if self.frame.is_none() {
            self.base = self.last.clone();
            self.frames = Vec::new();
            FilterStatus::Complete(self.last.clone())
        } else {
            FilterStatus::Pending
        }
    }
    pub fn tick(&mut self, store: &mut Store<V>) -> FilterStatus<V> {
        self.tick_frontier(store, None)
    }
    /// Caller certifies that all keys outside this sorted frontier are already
    /// correct. Unaffected immutable subtrees are returned without leaf visits.
    pub(crate) fn tick_keys(&mut self, store: &mut Store<V>, keys: &[Key]) -> FilterStatus<V> {
        self.tick_frontier(store, Some(keys))
    }
    fn tick_frontier(&mut self, store: &mut Store<V>, keys: Option<&[Key]>) -> FilterStatus<V> {
        assert_eq!(self.owner, store.owner, "foreign index filter");
        store.assert_mutable();
        assert!(store.contains(&self.base), "stale index filter root");
        if let Some(leaf) = &self.leaf {
            leaf.replacement
                .expect("filter Leaf requires replace before tick");
        }
        if let Some(leaf) = self.leaf.take() {
            let replacement = leaf.replacement.unwrap();
            if let Some(FilterFrame::Page { result, .. }) = &mut self.frame {
                if replacement != Some(leaf.value) {
                    let input = std::mem::take(result);
                    *result = match replacement {
                        Some(value) => store.insert(input, leaf.key, value),
                        None => store.remove(input, &leaf.key),
                    };
                }
                return FilterStatus::Pending;
            }
            let root = match replacement {
                None => Root::empty(),
                Some(v) if v == leaf.value => leaf.root,
                Some(value) => store.allocate(Node::Leaf {
                    key: leaf.key,
                    value,
                }),
            };
            return self.returned(root);
        }
        if let Some(FilterFrame::Page { run, .. }) = &mut self.frame {
            if let Some((key, value)) = run.next(store) {
                self.leaf = Some(FilterLeaf {
                    root: Root::empty(),
                    key,
                    value,
                    replacement: None,
                });
                return FilterStatus::Leaf { key, value };
            }
            let Some(FilterFrame::Page { result, .. }) = self.frame.take() else {
                unreachable!()
            };
            return self.returned(result);
        }
        if let (Some(keys), Some(FilterFrame::Visit(root))) = (keys, &self.frame)
            && !root.is_empty()
        {
            let (low, high) = match &store.record(root).node {
                Node::Leaf { key, .. } => (*key, *key),
                Node::Page { prefix, .. } => bounds(*prefix, PAGE_BIT as u8),
                Node::Branch { prefix, bit, .. } => bounds(*prefix, *bit),
            };
            let position = keys.partition_point(|key| *key < low);
            if keys.get(position).is_none_or(|key| *key > high) {
                store.stats.page_fallback_visits.fetch_add(1, Relaxed);
                let Some(FilterFrame::Visit(root)) = self.frame.take() else {
                    unreachable!()
                };
                return self.returned(root);
            }
        }
        match self.frame.take() {
            None => FilterStatus::Complete(self.last.clone()),
            Some(FilterFrame::Visit(root)) if root.is_empty() => self.returned(root),
            Some(FilterFrame::Visit(root)) => match &store.record(&root).node {
                Node::Leaf { key, value } => {
                    store.stats.page_fallback_visits.fetch_add(1, Relaxed);
                    let (key, value) = (*key, *value);
                    self.leaf = Some(FilterLeaf {
                        root,
                        key,
                        value,
                        replacement: None,
                    });
                    FilterStatus::Leaf { key, value }
                }
                Node::Page { entries, .. } => {
                    store.stats.page_runs.fetch_add(1, Relaxed);
                    self.frame = Some(FilterFrame::Page {
                        run: PageRun {
                            root: root.clone(),
                            position: 0,
                            end: entries.len() as u8,
                        },
                        result: root,
                    });
                    FilterStatus::Pending
                }
                Node::Branch { left, .. } => {
                    store.stats.page_fallback_visits.fetch_add(1, Relaxed);
                    let left = left.clone();
                    self.frames.push(FilterFrame::AfterLeft { root });
                    self.frame = Some(FilterFrame::Visit(left));
                    FilterStatus::Pending
                }
            },
            Some(FilterFrame::AfterLeft { root }) => {
                let Node::Branch { right, .. } = store.node(&root) else {
                    unreachable!()
                };
                self.frames.push(FilterFrame::AfterRight {
                    root,
                    left: self.last.clone(),
                });
                self.frame = Some(FilterFrame::Visit(right));
                FilterStatus::Pending
            }
            Some(FilterFrame::AfterRight { root, left }) => {
                let Node::Branch {
                    prefix,
                    bit,
                    left: old_left,
                    right: old_right,
                } = store.node(&root)
                else {
                    unreachable!()
                };
                let right = self.last.clone();
                let result = if left == old_left && right == old_right {
                    root
                } else if left.is_empty() {
                    right
                } else if right.is_empty() {
                    left
                } else {
                    store.allocate(Node::Branch {
                        prefix,
                        bit,
                        left,
                        right,
                    })
                };
                self.returned(result)
            }
            Some(FilterFrame::Page { .. }) => unreachable!(),
        }
    }
}
pub struct Collector<I, V: Value = Condition> {
    owner: u32,
    epoch: u64,
    _lease: GcLease,
    roots: I,
    archived: std::vec::IntoIter<Root<V>>,
    archiving: bool,
    pending: Vec<Root<V>>,
    marking: bool,
    done: bool,
    page: Option<(Root<V>, usize)>,
}
impl<I: Iterator<Item = Root<V>>, V: Value> Collector<I, V> {
    pub(crate) fn validate(&self, store: &Store<V>) {
        assert_eq!(self.owner, store.owner, "foreign index collector");
        assert_eq!(self.epoch, store.epoch, "stale index collector");
    }
    pub fn done(&self) -> bool {
        self.done
    }
    pub(crate) fn archiving(&self) -> bool {
        self.archiving
    }
    pub fn tick(&mut self, store: &mut Store<V>) -> Option<(Key, V)> {
        self.validate(store);
        if self.done {
            return None;
        }
        if let Some((root, position)) = &mut self.page {
            let Node::Page { prefix, entries } = &store.record(root).node else {
                unreachable!()
            };
            if let Some(&(tail, value)) = entries.get(*position) {
                *position += 1;
                return Some((page_key(*prefix, tail), value));
            }
            self.page = None;
            return None;
        }
        if self.marking {
            if let Some(root) = self.pending.pop().or_else(|| {
                if self.archiving {
                    self.archived.next()
                } else {
                    self.roots.next()
                }
            }) {
                assert!(store.contains(&root), "stale or foreign collection root");
                if !root.is_empty() {
                    let record = store.record(&root);
                    let fresh = if self.archiving {
                        record.archived.swap(store.archive_epoch, Relaxed) != store.archive_epoch
                    } else {
                        record.archived.load(Relaxed) != store.archive_epoch
                            && record.marked.swap(self.epoch, Relaxed) != self.epoch
                    };
                    if fresh {
                        match &record.node {
                            Node::Leaf { key, value } => return Some((*key, *value)),
                            Node::Page { prefix, entries } => {
                                let (tail, value) = entries[0];
                                let key = page_key(*prefix, tail);
                                self.page = Some((root, 1));
                                return Some((key, value));
                            }
                            Node::Branch { left, right, .. } => {
                                self.pending.extend([right.clone(), left.clone()])
                            }
                        }
                    }
                }
            } else if self.archiving {
                self.archiving = false;
            } else {
                store.completed = self.epoch;
                store.archive_completed = store.archive_epoch;
                self.marking = false;
            }
        } else if store.release_tick() {
            self.pending = Vec::new();
            self.done = true;
        }
        None
    }
}
#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn maximal_filter_stack_is_bounded_and_completion_releases_capacity() {
        let mut store = Store::default();
        let mut root = store.insert(store.empty(), [0; 4], 0_u64);
        for bit in 0..256 {
            let mut key = [0; 4];
            key[bit / 64] = 1_u64 << (63 - bit % 64);
            root = store.insert(root, key, bit as u64 + 1);
        }
        let mut filter = store.filter(root.clone());
        let mut peak = 0;
        let before = store.node_count();
        let mut leaves = 0;
        loop {
            let status = filter.tick(&mut store);
            peak = peak.max(filter.frames.len());
            assert!(filter.frames.len() <= 256);
            match status {
                FilterStatus::Pending => {}
                FilterStatus::Leaf { value, .. } => {
                    leaves += 1;
                    filter.replace(Some(value));
                }
                FilterStatus::Complete(result) => {
                    assert_eq!(result, root);
                    break;
                }
            }
        }
        assert_eq!(leaves, 257);
        assert_eq!(peak, PAGE_BIT);
        assert_eq!(store.node_count(), before);
        assert_eq!(filter.frames.capacity(), 0);
        assert!(filter.frame.is_none());
        assert!(filter.leaf.is_none());
    }
}

#[cfg(test)]
mod release_block_tests {
    use super::*;
    #[test]
    fn queue_preserves_lifo_and_frees_every_empty_overflow_block() {
        let mut store = Store::<u64>::default();
        let mut state = QueueState {
            inline: Block::new(),
            overflow: None,
        };
        for i in 0..4097 {
            let mut root = store.insert(store.empty(), [i, 0, 0, 0], i);
            state.push((root.node.take(), None));
            if i < RELEASE_CAPACITY as u64 {
                assert!(state.overflow.is_none());
            }
        }
        for expected in (0..4097).rev() {
            let (left, right) = state.pop().unwrap();
            assert!(right.is_none());
            assert!(matches!(&left.unwrap().node, Node::Leaf { value, .. } if *value == expected));
            let mut block = state.overflow.as_deref();
            let mut pairs = state.inline.len;
            while let Some(b) = block {
                assert!(b.len > 0 && b.len <= RELEASE_CAPACITY);
                pairs += b.len;
                block = b.next.as_deref();
            }
            assert_eq!(pairs, expected as usize);
        }
        assert!(state.pop().is_none() && state.overflow.is_none());
        assert!(state.inline.pairs.iter().all(Option::is_none));
        assert_eq!(store.node_count(), 0);
        for _ in 0..10000 {
            state.push((None, None));
            state.pop().unwrap();
            assert!(state.overflow.is_none());
        }
    }
    #[test]
    fn concurrent_root_drops_and_collection_keep_pinned_authority() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Root<u64>>();
        let mut store = Store::<u64>::default();
        let mut root = store.empty();
        for i in 0..1024 {
            root = store.insert(root, [i, 0, 0, 0], i);
        }
        let pin = root.clone();
        let mut versions = Vec::new();
        for i in 0..512 {
            root = store.insert(root, [i, 0, 0, 0], i + 10000);
            versions.push(root.clone());
        }
        std::thread::scope(|scope| {
            for chunk in versions.chunks(64) {
                let owned = chunk.to_vec();
                scope.spawn(move || drop(owned));
            }
            drop(versions);
            for _ in 0..1024 {
                store.release_tick();
            }
        });
        assert_eq!(store.get(&pin, &[0, 0, 0, 0]), Some(0));
        assert_eq!(store.get(&root, &[0, 0, 0, 0]), Some(10000));
        let mut collector = store.collect([pin.clone(), root.clone()].into_iter());
        while !collector.done() {
            collector.tick(&mut store);
        }
        drop(collector);
        assert!(store.contains(&pin) && store.contains(&root));
        drop(pin);
        drop(root);
        while store.release_pending() {
            let before = store.node_count();
            store.release_tick();
            assert!(before - store.node_count() <= 2);
        }
        assert_eq!(store.node_count(), 0);
        assert!(store.queue.state.lock().unwrap().overflow.is_none());
    }
}

#[cfg(test)]
mod prefix_identity_tests {
    use super::*;
    #[test]
    fn memo_normalization_churn_generation_and_release() {
        let mut store = Store::<u64>::default();
        let mut root = store.empty();
        for i in 0..96 {
            root = store.insert(root, [2, i, 4, 0], i);
        }
        for i in 0..96 {
            let a = store.prefix_root(&root, [2, i, 99, 777], 2);
            let b = store.prefix_root(&root, [2, i, 0, 0], 2);
            assert_eq!(a, b);
            assert_eq!(store.get(&a, &[2, i, 4, 0]), Some(i));
            assert!(store.prefix_memo_counts()[3] <= PREFIX_MEMO_LIMIT);
        }
        assert_eq!(store.prefix_memo_counts()[0], 96);
        assert_eq!(store.prefix_memo_counts()[2], 64);
        let old = root.clone();
        drop(store.prefix_root(&root, [2, 95, 4, 0], 3));
        root = store.insert(root, [2, 95, 4, 0], 999);
        for (view, expected) in [(&old, 95), (&root, 999)] {
            let dependency = store.prefix_root(view, [2, 95, 4, 0], 3);
            assert_eq!(store.get(&dependency, &[2, 95, 4, 0]), Some(expected));
        }
        for _ in 0..2 {
            assert!(store.prefix_root(&root, [9, 0, 0, 0], 1).is_empty());
        }
        let foreign = Store::<u64>::default();
        assert!(std::panic::catch_unwind(|| foreign.prefix_root(&root, [2, 95, 4, 0], 3)).is_err());
        let weak = root.downgrade();
        drop((old, root));
        while !store.release_tick() {}
        assert_eq!(store.node_count(), 0);
        assert!(weak.node.as_ref().unwrap().upgrade().is_none());
        let mut root = store.insert(store.empty(), [2, 95, 4, 0], 1000);
        assert_eq!(store.prefix_root(&root, [2, 95, 4, 0], 3), root);
        // Only weak owners remain beyond the caller: mutation must detach.
        root = store.insert(root, [2, 95, 4, 0], 1001);
        assert_eq!(
            store.get(&store.prefix_root(&root, [2, 95, 4, 0], 3), &[2, 95, 4, 0]),
            Some(1001)
        );
        let mut gc = store.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        assert_eq!(&store.prefix_memo_counts()[3..], &[0, 0]);
        assert!(std::panic::catch_unwind(|| store.prefix_root(&root, [2, 95, 4, 0], 3)).is_err());
        drop(root);
        while !store.release_tick() {}
        assert_eq!(store.node_count(), 0);
    }
    #[test]
    fn split_prefix_witnesses_preserve_order_generation_and_stale_rejection() {
        let mut store = Store::<u64>::default();
        let mut root = store.empty();
        for i in 0..256 {
            root = store.insert(root, [2, 3, 4, i], i);
        }
        let [left, right] = store.split_prefix(&root, 128).unwrap();
        assert!(store.split_prefix(&left, 128).is_none());
        assert!(store.split_prefix(&right, 128).is_none());
        for (subtree, start) in [(left.clone(), 0), (right.clone(), 128)] {
            let mut cursor = store.range(subtree, [0; 4], [u64::MAX; 4]);
            for i in start..start + 128 {
                assert_eq!(cursor.next(&store), Some(([2, 3, 4, i], i)));
            }
            assert_eq!(cursor.next(&store), None);
        }
        let witness = left.downgrade();
        drop((left, right));
        root = store.insert(root, [2, 3, 4, 255], 9000);
        let [left, right] = store.split_prefix(&root, 128).unwrap();
        assert!(witness.matches(&left));
        drop((left, right));
        root = store.insert(root, [2, 3, 4, 0], 9001);
        assert!(!witness.matches(&store.split_prefix(&root, 128).unwrap()[0]));
        let foreign = Store::<u64>::default();
        assert!(std::panic::catch_unwind(|| foreign.split_prefix(&root, 128)).is_err());
        let mut gc = store.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        assert!(std::panic::catch_unwind(|| store.split_prefix(&root, 128)).is_err());
        drop(root);
        while !store.release_tick() {}
        assert_eq!(store.node_count(), 0);
    }
    #[test]
    fn exact_prefix_witness_survives_other_updates_and_detects_unique_mutation() {
        let mut store = Store::default();
        let mut root = store.empty();
        for i in 0..16 {
            root = store.insert(root, [2, 3, 4, i], i);
        }
        let witness = store.prefix_root(&root, [2, 3, 4, 0], 3).downgrade();
        // No owning subtree/snapshot reference remains: only the weak witness.
        root = store.insert(root, [2, 3, 5, 0], 99);
        root = store.insert(root, [1, 0, 0, 0], 100);
        assert!(witness.matches(&store.prefix_root(&root, [2, 3, 4, 0], 3)));
        assert!(store.prefix_root(&root, [2, 3, 6, 0], 3).is_empty());
        root = store.insert(root, [2, 3, 4, 7], 777);
        assert!(!witness.matches(&store.prefix_root(&root, [2, 3, 4, 0], 3)));
        assert_eq!(store.get(&root, &[2, 3, 4, 7]), Some(777));
        drop(root);
        while !store.release_tick() {}
        assert_eq!(
            store.node_count(),
            0,
            "weak witness must not retain payloads"
        );
    }
}

#[cfg(test)]
mod page_archive_tests {
    use super::*;

    #[test]
    fn interrupted_page_registration_retraces_all_values_and_releases_final_archive() {
        for cutoff in 0..24 {
            let mut store = Store::default();
            let mut root = store.empty();
            for n in 0..16 {
                root = store.insert(root, [1, 2, 3, n], n);
            }
            let archive = root.clone();
            root = store.insert(root, [1, 2, 3, 7], 77);
            let mut registration =
                store.collect_archived([root.clone()].into_iter(), vec![archive.clone()], true);
            for _ in 0..cutoff {
                registration.tick(&mut store);
            }
            drop(registration);
            // An aborted archive registration restarts with the full root set.
            let mut registration =
                store.collect_archived([root.clone()].into_iter(), vec![archive.clone()], true);
            let mut seen = std::collections::BTreeSet::new();
            while !registration.done() {
                if let Some(pair) = registration.tick(&mut store) {
                    seen.insert(pair);
                }
            }
            drop(registration);
            for n in 0..16 {
                assert!(seen.contains(&([1, 2, 3, n], n)));
            }
            assert!(seen.contains(&([1, 2, 3, 7], 77)));
            // Completed archive protection lets the next pass skip its scalars.
            let mut gc = store.collect_archived(std::iter::empty(), vec![], false);
            while !gc.done() {
                assert_eq!(gc.tick(&mut store), None);
            }
            drop(gc);
            assert!(store.contains(&archive));
            assert!(!store.contains(&root));
            drop(root);
            let mut gc = store.collect(std::iter::empty());
            while !gc.done() {
                gc.tick(&mut store);
            }
            drop(gc);
            assert!(!store.contains(&archive));
            drop(archive);
            while !store.release_tick() {}
            assert_eq!(store.node_count(), 0);
        }
    }
}
