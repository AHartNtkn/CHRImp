//! Persistent ordered indexes for coherent shared graph roots.
//!
//! A compressed binary trie copies only the changed key path. Four-word keys
//! cover graph namespaces and compound port postings without lossy hashing.
//! Paths have at most 256 branches regardless of index size. Node ownership is
//! explicit so releasing a snapshot never recursively destroys a large graph.

use crate::gc::GcLease;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

mod substitute;
pub use substitute::Substitution;

pub type Key = [u64; 4];
static NEXT_STORE: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    owner: u32,
    id: u64,
}
const EMPTY: Root = Root { owner: 0, id: 0 };

#[derive(Clone, Copy)]
enum Node<V> {
    Leaf {
        key: Key,
        value: V,
    },
    Branch {
        prefix: Key,
        bit: u8,
        left: Root,
        right: Root,
    },
}
struct Record<V> {
    node: Node<V>,
    marked: u64,
}

pub struct Store<V> {
    owner: u32,
    next_node: u64,
    nodes: BTreeMap<u64, Record<V>>,
    epoch: u64,
    frozen: Arc<AtomicBool>,
}

impl<V> Default for Store<V> {
    fn default() -> Self {
        Self {
            owner: NEXT_STORE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("index identity exhausted"),
            next_node: 0,
            nodes: BTreeMap::new(),
            epoch: 0,
            frozen: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn right(key: &Key, bit: u8) -> bool {
    key[bit as usize / 64] & (1 << (63 - bit % 64)) != 0
}
fn difference(a: &Key, b: &Key) -> Option<u8> {
    a.iter().zip(b).enumerate().find_map(|(i, (&a, &b))| {
        (a != b).then(|| (i * 64 + (a ^ b).leading_zeros() as usize) as u8)
    })
}
fn bounds(mut prefix: Key, bit: u8) -> (Key, Key) {
    let word = bit as usize / 64;
    let mask = if bit.is_multiple_of(64) {
        0
    } else {
        u64::MAX << (64 - bit % 64)
    };
    prefix[word] &= mask;
    let mut high = prefix;
    high[word] |= !mask;
    for i in word + 1..4 {
        prefix[i] = 0;
        high[i] = u64::MAX;
    }
    (prefix, high)
}

impl<V: Copy + Eq> Store<V> {
    pub(crate) fn assert_mutable(&self) {
        GcLease::assert_mutable(&self.frozen);
    }

    pub fn empty(&self) -> Root {
        EMPTY
    }
    pub fn contains(&self, root: Root) -> bool {
        root == EMPTY || (root.owner == self.owner && self.nodes.contains_key(&root.id))
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn node(&self, root: Root) -> Node<V> {
        assert!(
            root != EMPTY && root.owner == self.owner,
            "stale or foreign index root"
        );
        self.nodes
            .get(&root.id)
            .expect("stale or foreign index root")
            .node
    }
    fn allocate(&mut self, node: Node<V>) -> Root {
        let id = self.next_node;
        self.next_node = id.checked_add(1).expect("index node identity exhausted");
        self.nodes.insert(id, Record { node, marked: 0 });
        Root {
            owner: self.owner,
            id,
        }
    }

    pub fn get(&self, mut root: Root, key: &Key) -> Option<V> {
        while root != EMPTY {
            match self.node(root) {
                Node::Leaf { key: found, value } => return (found == *key).then_some(value),
                Node::Branch {
                    bit,
                    left,
                    right: r,
                    ..
                } => root = if right(key, bit) { r } else { left },
            }
        }
        None
    }

    pub fn insert(&mut self, root: Root, key: Key, value: V) -> Root {
        self.assert_mutable();
        if root == EMPTY {
            return self.allocate(Node::Leaf { key, value });
        }
        // Reuse visited branch values when splitting and rebuilding the path.
        let mut path = Vec::new();
        let mut cursor = root;
        let (found, old) = loop {
            match self.node(cursor) {
                Node::Leaf { key, value } => break (key, value),
                node @ Node::Branch {
                    bit,
                    left,
                    right: r,
                    ..
                } => {
                    path.push((cursor, node));
                    cursor = if right(&key, bit) { r } else { left };
                }
            }
        };
        if found == key && old == value {
            return root;
        }
        let mut replacement = self.allocate(Node::Leaf { key, value });
        if let Some(split) = difference(&found, &key) {
            let index = path
                .iter()
                .position(|&(_, node)| matches!(node, Node::Branch { bit, .. } if bit >= split))
                .unwrap_or(path.len());
            let subtree = path.get(index).map_or(cursor, |&(root, _)| root);
            path.truncate(index);
            let (left, r) = if right(&key, split) {
                (subtree, replacement)
            } else {
                (replacement, subtree)
            };
            replacement = self.allocate(Node::Branch {
                prefix: key,
                bit: split,
                left,
                right: r,
            });
        }
        self.rebuild(&path, &key, replacement)
    }

    fn rebuild(&mut self, path: &[(Root, Node<V>)], key: &Key, mut replacement: Root) -> Root {
        for &(_, node) in path.iter().rev() {
            let Node::Branch {
                prefix,
                bit,
                left,
                right: r,
            } = node
            else {
                unreachable!("branch path")
            };
            let (left, r) = if right(key, bit) {
                (left, replacement)
            } else {
                (replacement, r)
            };
            replacement = self.allocate(Node::Branch {
                prefix,
                bit,
                left,
                right: r,
            });
        }
        replacement
    }

    pub fn remove(&mut self, root: Root, key: &Key) -> Root {
        self.assert_mutable();
        if root == EMPTY {
            return root;
        }
        let mut path = Vec::new();
        let mut cursor = root;
        loop {
            match self.node(cursor) {
                Node::Leaf { key: found, .. } => {
                    if found != *key {
                        return root;
                    }
                    break;
                }
                node @ Node::Branch {
                    bit,
                    left,
                    right: r,
                    ..
                } => {
                    path.push((cursor, node));
                    cursor = if right(key, bit) { r } else { left };
                }
            }
        }
        let Some((_, parent)) = path.pop() else {
            return EMPTY;
        };
        let Node::Branch {
            bit,
            left,
            right: r,
            ..
        } = parent
        else {
            unreachable!("branch path")
        };
        let sibling = if right(key, bit) { left } else { r };
        self.rebuild(&path, key, sibling)
    }

    pub fn range(&self, root: Root, low: Key, high: Key) -> Cursor {
        assert!(self.contains(root), "stale or foreign index root");
        Cursor {
            root,
            pending: if root == EMPTY || low > high {
                Vec::new()
            } else {
                vec![root]
            },
            low,
            high,
            visits: 0,
        }
    }

    /// Visit each leaf once and rebuild changed branches bottom-up. Keep the
    /// token's roots and pending values traced whenever collection intervenes.
    pub fn filter(&self, root: Root) -> Filter<V> {
        assert!(self.contains(root), "stale or foreign index root");
        Filter {
            owner: self.owner,
            base: root,
            frame: Some(FilterFrame::Visit(root)),
            frames: Vec::new(),
            last: EMPTY,
            leaf: None,
        }
    }

    /// Supply all current, staged and explicitly inspected roots. Cursor roots
    /// retain their whole coherent index version. Leaf events expose payloads
    /// so the graph owner can trace occurrence and condition dependencies.
    /// The owner remains read-only until the returned token is dropped, even
    /// after completion. Dropping an unfinished token safely aborts collection.
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<I> {
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
            sweep: None,
            marking: true,
            done: false,
        }
    }
}

#[derive(Clone, Copy)]
enum FilterFrame {
    Visit(Root),
    AfterLeft { root: Root },
    AfterRight { root: Root, left: Root },
}

struct FilterLeaf<V> {
    root: Root,
    key: Key,
    value: V,
    replacement: Option<Option<V>>,
}

/// An owned filter continuation. Call `replace` exactly once after every Leaf.
/// Dropping this token abandons the staged result without changing its input.
pub struct Filter<V> {
    owner: u32,
    base: Root,
    frame: Option<FilterFrame>,
    // Only branch continuations are stacked, so even a full-width path has
    // at most 256 entries. The active Visit is held separately.
    frames: Vec<FilterFrame>,
    last: Root,
    leaf: Option<FilterLeaf<V>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterStatus<V> {
    Pending,
    Leaf { key: Key, value: V },
    Complete(Root),
}

impl<V: Copy + Eq> Filter<V> {
    pub fn replace(&mut self, value: Option<V>) {
        let leaf = self.leaf.as_mut().expect("filter replace requires a Leaf");
        assert!(leaf.replacement.is_none(), "filter Leaf already replaced");
        leaf.replacement = Some(value);
    }

    pub fn roots(&self) -> impl Iterator<Item = Root> + '_ {
        [self.base, self.last].into_iter().chain(
            self.frames
                .iter()
                .chain(self.frame.iter())
                .filter_map(|frame| match *frame {
                    FilterFrame::AfterRight { left, .. } => Some(left),
                    _ => None,
                }),
        )
    }

    /// Values supplied by the caller but not yet installed in a store leaf.
    pub fn values(&self) -> impl Iterator<Item = V> + '_ {
        self.leaf
            .iter()
            .filter_map(|leaf| leaf.replacement.flatten())
    }

    fn returned(&mut self, root: Root) -> FilterStatus<V> {
        self.last = root;
        self.frame = self.frames.pop();
        if self.frame.is_none() {
            self.base = root;
            self.frames = Vec::new();
            FilterStatus::Complete(root)
        } else {
            FilterStatus::Pending
        }
    }

    /// Perform one traversal transition or allocate at most one changed node.
    /// GC may run between ticks, but its lease must be dropped before ticking.
    pub fn tick(&mut self, store: &mut Store<V>) -> FilterStatus<V> {
        assert_eq!(self.owner, store.owner, "foreign index filter");
        store.assert_mutable();
        assert!(store.contains(self.base), "stale index filter root");
        if let Some(leaf) = &self.leaf {
            let replacement = leaf
                .replacement
                .expect("filter Leaf requires replace before tick");
            let root = match replacement {
                None => EMPTY,
                Some(value) if value == leaf.value => leaf.root,
                Some(value) => store.allocate(Node::Leaf {
                    key: leaf.key,
                    value,
                }),
            };
            self.leaf = None;
            return self.returned(root);
        }
        match self.frame {
            None => FilterStatus::Complete(self.last),
            Some(FilterFrame::Visit(root)) if root == EMPTY => self.returned(EMPTY),
            Some(FilterFrame::Visit(root)) => match store.node(root) {
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
                let Node::Branch { right, .. } = store.node(root) else {
                    unreachable!("filter branch")
                };
                self.frames.push(FilterFrame::AfterRight {
                    root,
                    left: self.last,
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
                } = store.node(root)
                else {
                    unreachable!("filter branch")
                };
                let right = self.last;
                let result = if left == old_left && right == old_right {
                    root
                } else if left == EMPTY {
                    right
                } else if right == EMPTY {
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

pub struct Cursor {
    root: Root,
    pending: Vec<Root>,
    low: Key,
    high: Key,
    visits: u64,
}

impl Cursor {
    pub fn root(&self) -> Root {
        self.root
    }
    pub fn visits(&self) -> u64 {
        self.visits
    }

    /// Advance to one row, crossing at most two bounded key paths between rows.
    /// No tuple product or unrelated predicate bucket is materialized.
    pub fn next<V: Copy + Eq>(&mut self, store: &Store<V>) -> Option<(Key, V)> {
        while let Some(root) = self.pending.pop() {
            self.visits += 1;
            match store.node(root) {
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

pub struct Collector<I> {
    owner: u32,
    epoch: u64,
    _lease: GcLease,
    roots: I,
    pending: Vec<Root>,
    sweep: Option<u64>,
    marking: bool,
    done: bool,
}

impl<I: Iterator<Item = Root>> Collector<I> {
    pub(crate) fn validate<V: Copy + Eq>(&self, store: &Store<V>) {
        assert_eq!(self.owner, store.owner, "foreign index collector");
        assert_eq!(self.epoch, store.epoch, "stale index collector");
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// Mark or reclaim one node. Each reachable physical leaf is reported once
    /// across all roots; map operations retain their usual size-dependent cost.
    pub fn tick<V: Copy + Eq>(&mut self, store: &mut Store<V>) -> Option<(Key, V)> {
        self.validate(store);
        if self.done {
            return None;
        }
        if self.marking {
            if let Some(root) = self.pending.pop().or_else(|| self.roots.next()) {
                if root != EMPTY {
                    assert_eq!(root.owner, store.owner, "stale or foreign collection root");
                    let record = store
                        .nodes
                        .get_mut(&root.id)
                        .expect("stale or foreign collection root");
                    if record.marked != self.epoch {
                        record.marked = self.epoch;
                        match record.node {
                            Node::Leaf { key, value } => return Some((key, value)),
                            Node::Branch { left, right, .. } => self.pending.extend([right, left]),
                        }
                    }
                }
            } else {
                self.marking = false;
            }
        } else {
            let next = match self.sweep {
                Some(id) => store.nodes.range((Excluded(id), Unbounded)).next(),
                None => store.nodes.first_key_value(),
            };
            if let Some((&id, record)) = next {
                if record.marked != self.epoch {
                    store.nodes.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
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
        let mut filter = store.filter(root);
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
