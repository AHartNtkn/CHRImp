//! Arc-owned persistent indexes. Explicit leaf tracing protects scalar payloads;
//! completed collection epochs invalidate untraced roots. Child release is deferred.
use crate::{condition::Condition, gc::GcLease};
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
    allocated: AtomicUsize,
    live: AtomicUsize,
    peak: AtomicUsize,
    unique: AtomicUsize,
    copied: AtomicUsize,
    released: AtomicUsize,
}
struct Batch<V: Value> {
    left: Option<Arc<Record<V>>>,
    right: Option<Arc<Record<V>>>,
    next: Option<Box<Batch<V>>>,
}
struct Queue<V: Value> {
    head: Mutex<Option<Box<Batch<V>>>>,
    count: AtomicUsize,
}
impl<V: Value> Queue<V> {
    fn new() -> Self {
        Self {
            head: Mutex::new(None),
            count: AtomicUsize::new(0),
        }
    }
    fn push(&self, left: Option<Arc<Record<V>>>, right: Option<Arc<Record<V>>>) {
        let mut head = self.head.lock().unwrap();
        let next = head.take();
        *head = Some(Box::new(Batch { left, right, next }));
        self.count.fetch_add(1, Relaxed);
    }
    fn pop(&self) -> Option<Box<Batch<V>>> {
        let mut head = self.head.lock().unwrap();
        let mut batch = head.take()?;
        *head = batch.next.take();
        self.count.fetch_sub(1, Relaxed);
        Some(batch)
    }
}
impl<V: Value> Drop for Queue<V> {
    fn drop(&mut self) {
        let head = self.head.get_mut().unwrap();
        while let Some(mut batch) = head.take() {
            *head = batch.next.take();
            drop(batch);
        }
    }
}
#[derive(Clone)]
enum Node<V: Value> {
    Leaf {
        key: Key,
        value: V,
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
    queue: Weak<Queue<V>>,
    stats: Arc<Stats>,
}
impl<V: Value> Record<V> {
    fn children(&mut self) -> (Option<Arc<Self>>, Option<Arc<Self>>) {
        match &mut self.node {
            Node::Leaf { .. } => (None, None),
            Node::Branch { left, right, .. } => (left.node.take(), right.node.take()),
        }
    }
}
impl<V: Value> Drop for Record<V> {
    fn drop(&mut self) {
        self.stats.live.fetch_sub(1, Relaxed);
        self.stats.released.fetch_add(1, Relaxed);
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
    frozen: Arc<AtomicBool>,
    queue: Arc<Queue<V>>,
    stats: Arc<Stats>,
}
impl<V: Value> Default for Store<V> {
    fn default() -> Self {
        Self {
            owner: NEXT_STORE
                .fetch_update(Relaxed, Relaxed, |x| x.checked_add(1))
                .expect("index identity exhausted"),
            epoch: 0,
            completed: 0,
            frozen: Arc::new(AtomicBool::new(false)),
            queue: Arc::new(Queue::new()),
            stats: Arc::new(Stats::default()),
        }
    }
}
fn right(key: &Key, bit: u8) -> bool {
    key[bit as usize / 64] & (1 << (63 - bit % 64)) != 0
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
    pub(crate) fn assert_mutable(&self) {
        GcLease::assert_mutable(&self.frozen);
    }
    pub fn empty(&self) -> Root<V> {
        Root::empty()
    }
    pub fn contains(&self, root: &Root<V>) -> bool {
        root.is_empty()
            || (root.owner == self.owner
                && root.node.as_ref().unwrap().marked.load(Relaxed) >= self.completed)
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
        if let Some(mut batch) = self.queue.pop() {
            drop(batch.left.take());
            drop(batch.right.take());
            false
        } else {
            true
        }
    }
    pub fn mutation_counts(&self) -> (usize, usize) {
        (
            self.stats.unique.load(Relaxed),
            self.stats.copied.load(Relaxed),
        )
    }
    fn record<'a>(&self, root: &'a Root<V>) -> &'a Record<V> {
        assert!(
            self.contains(root) && !root.is_empty(),
            "stale or foreign index root"
        );
        root.node.as_deref().unwrap()
    }
    fn node(&self, root: &Root<V>) -> Node<V> {
        self.record(root).node.clone()
    }
    fn allocate(&mut self, node: Node<V>) -> Root<V> {
        let leaves = Self::leaf_count(&node);
        self.stats.allocated.fetch_add(1, Relaxed);
        let live = self.stats.live.fetch_add(1, Relaxed) + 1;
        self.stats.peak.fetch_max(live, Relaxed);
        Root {
            owner: self.owner,
            node: Some(Arc::new(Record {
                leaves,
                node,
                marked: AtomicU64::new(self.epoch),
                queue: Arc::downgrade(&self.queue),
                stats: self.stats.clone(),
            })),
        }
    }
    fn unique(&mut self, root: &mut Root<V>) {
        assert!(self.contains(root));
        if Arc::get_mut(root.node.as_mut().unwrap()).is_some() {
            self.stats.unique.fetch_add(1, Relaxed);
        } else {
            self.stats.copied.fetch_add(1, Relaxed);
            *root = self.allocate(self.node(root));
        }
    }
    fn leaf_count(node: &Node<V>) -> usize {
        match node {
            Node::Leaf { .. } => 1,
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
    fn insert_node(&mut self, mut root: Root<V>, key: Key, value: V) -> Root<V> {
        if root.is_empty() {
            return self.allocate(Node::Leaf { key, value });
        }
        let (prefix, bit) = match &self.record(&root).node {
            Node::Leaf { key, .. } => (*key, 256),
            Node::Branch { prefix, bit, .. } => (*prefix, *bit as usize),
        };
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
    pub fn range(&self, root: Root<V>, low: Key, high: Key) -> Cursor<V> {
        assert!(self.contains(&root), "stale or foreign index root");
        let pending = if root.is_empty() {
            vec![]
        } else {
            vec![root.clone()]
        };
        Cursor {
            root,
            pending,
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
        let lease = GcLease::acquire(&self.frozen);
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("index collection epoch exhausted");
        Collector {
            owner: self.owner,
            epoch: self.epoch,
            _lease: lease,
            roots,
            pending: Vec::new(),
            marking: true,
            done: false,
        }
    }
}
pub struct Cursor<V: Value = Condition> {
    root: Root<V>,
    pending: Vec<Root<V>>,
    low: Key,
    high: Key,
    visits: u64,
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
        while let Some(root) = self.pending.pop() {
            self.visits += 1;
            match store.node(&root) {
                Node::Leaf { key, value } => {
                    if key >= self.low && key <= self.high {
                        return Some((key, value));
                    }
                }
                Node::Branch {
                    prefix,
                    bit,
                    left,
                    right,
                } => {
                    let (low, high) = bounds(prefix, bit);
                    if low <= self.high && high >= self.low {
                        self.pending.extend([right, left]);
                    }
                }
            }
        }
        None
    }
}
#[derive(Clone)]
enum FilterFrame<V: Value> {
    Visit(Root<V>),
    AfterLeft { root: Root<V> },
    AfterRight { root: Root<V>, left: Root<V> },
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
        assert_eq!(self.owner, store.owner, "foreign index filter");
        store.assert_mutable();
        assert!(store.contains(&self.base), "stale index filter root");
        if let Some(leaf) = &self.leaf {
            leaf.replacement
                .expect("filter Leaf requires replace before tick");
        }
        if let Some(leaf) = self.leaf.take() {
            let replacement = leaf.replacement.unwrap();
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
        match self.frame.take() {
            None => FilterStatus::Complete(self.last.clone()),
            Some(FilterFrame::Visit(root)) if root.is_empty() => self.returned(root),
            Some(FilterFrame::Visit(root)) => match store.node(&root) {
                Node::Leaf { key, value } => {
                    self.leaf = Some(FilterLeaf {
                        root,
                        key,
                        value,
                        replacement: None,
                    });
                    FilterStatus::Leaf { key, value }
                }
                Node::Branch { left, .. } => {
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
        }
    }
}
pub struct Collector<I, V: Value = Condition> {
    owner: u32,
    epoch: u64,
    _lease: GcLease,
    roots: I,
    pending: Vec<Root<V>>,
    marking: bool,
    done: bool,
}
impl<I: Iterator<Item = Root<V>>, V: Value> Collector<I, V> {
    pub(crate) fn validate(&self, store: &Store<V>) {
        assert_eq!(self.owner, store.owner, "foreign index collector");
        assert_eq!(self.epoch, store.epoch, "stale index collector");
    }
    pub fn done(&self) -> bool {
        self.done
    }
    pub fn tick(&mut self, store: &mut Store<V>) -> Option<(Key, V)> {
        self.validate(store);
        if self.done {
            return None;
        }
        if self.marking {
            if let Some(root) = self.pending.pop().or_else(|| self.roots.next()) {
                assert!(store.contains(&root), "stale or foreign collection root");
                if !root.is_empty() {
                    let record = store.record(&root);
                    if record.marked.swap(self.epoch, Relaxed) != self.epoch {
                        match &record.node {
                            Node::Leaf { key, value } => return Some((*key, *value)),
                            Node::Branch { left, right, .. } => {
                                self.pending.extend([right.clone(), left.clone()])
                            }
                        }
                    }
                }
            } else {
                store.completed = self.epoch;
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
    fn maximal_filter_stack_is_256_and_completion_releases_capacity() {
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
        assert_eq!(peak, 256);
        assert_eq!(store.node_count(), before);
        assert_eq!(filter.frames.capacity(), 0);
        assert!(filter.frame.is_none());
        assert!(filter.leaf.is_none());
    }
}
