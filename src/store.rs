//! Persistent ordered indexes for coherent shared graph roots.
//!
//! A compressed binary trie copies only the changed key path. Four-word keys
//! cover graph namespaces and compound port postings without lossy hashing.
//! Paths have at most 256 branches regardless of index size. Node ownership is
//! explicit so releasing a snapshot never recursively destroys a large graph.

use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::atomic::{AtomicU32, Ordering};

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
            root != EMPTY && self.contains(root),
            "stale or foreign index root"
        );
        self.nodes[&root.id].node
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
        if root == EMPTY {
            return self.allocate(Node::Leaf { key, value });
        }
        let mut path = Vec::new();
        let mut cursor = root;
        let (found, old) = loop {
            match self.node(cursor) {
                Node::Leaf { key, value } => break (key, value),
                Node::Branch {
                    bit,
                    left,
                    right: r,
                    ..
                } => {
                    path.push(cursor);
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
                .position(
                    |&root| matches!(self.node(root), Node::Branch { bit, .. } if bit >= split),
                )
                .unwrap_or(path.len());
            let subtree = path.get(index).copied().unwrap_or(cursor);
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

    fn rebuild(&mut self, path: &[Root], key: &Key, mut replacement: Root) -> Root {
        for &ancestor in path.iter().rev() {
            let Node::Branch {
                prefix,
                bit,
                left,
                right: r,
            } = self.node(ancestor)
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
                Node::Branch {
                    bit,
                    left,
                    right: r,
                    ..
                } => {
                    path.push(cursor);
                    cursor = if right(key, bit) { r } else { left };
                }
            }
        }
        let Some(parent) = path.pop() else {
            return EMPTY;
        };
        let Node::Branch {
            bit,
            left,
            right: r,
            ..
        } = self.node(parent)
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

    /// Supply all current, staged and explicitly inspected roots. Cursor roots
    /// retain their whole coherent index version. Leaf events expose payloads
    /// so the graph owner can trace occurrence and condition dependencies.
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<'_, V, I> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("index collection epoch exhausted");
        Collector {
            store: self,
            roots,
            pending: Vec::new(),
            sweep: None,
            marking: true,
            done: false,
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

pub struct Collector<'a, V, I> {
    store: &'a mut Store<V>,
    roots: I,
    pending: Vec<Root>,
    sweep: Option<u64>,
    marking: bool,
    done: bool,
}

impl<V: Copy + Eq, I: Iterator<Item = Root>> Collector<'_, V, I> {
    pub fn done(&self) -> bool {
        self.done
    }

    /// Mark or reclaim one node. Each reachable physical leaf is reported once
    /// across all roots; map operations retain their usual size-dependent cost.
    pub fn tick(&mut self) -> Option<(Key, V)> {
        if self.done {
            return None;
        }
        if self.marking {
            if let Some(root) = self.pending.pop().or_else(|| self.roots.next()) {
                assert!(
                    self.store.contains(root),
                    "stale or foreign collection root"
                );
                if root != EMPTY {
                    let record = self.store.nodes.get_mut(&root.id).expect("live root");
                    if record.marked != self.store.epoch {
                        record.marked = self.store.epoch;
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
                Some(id) => self.store.nodes.range((Excluded(id), Unbounded)).next(),
                None => self.store.nodes.first_key_value(),
            };
            if let Some((&id, record)) = next {
                if record.marked != self.store.epoch {
                    self.store.nodes.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}
