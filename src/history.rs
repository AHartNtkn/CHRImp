//! Persistent propagation support for a rule and ordered occurrence tuple.
//!
//! The caller computes replacement support and owns current/staged roots. This
//! store preserves once-only records until their roots are released; deciding
//! that an occurrence tuple is dead belongs to the executor.

use crate::condition::Condition;
use crate::store::{self, Cursor, Root, Store};
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    rule: usize,
    heads: Arc<Vec<u64>>,
}
struct Record {
    key: Key,
    marked: u64,
}

#[derive(Default)]
pub struct History {
    index: Store<Condition>,
    lookup: BTreeMap<Key, u64>,
    records: BTreeMap<u64, Record>,
    next_id: u64,
    epoch: u64,
}

impl History {
    pub fn empty(&self) -> Root {
        self.index.empty()
    }
    /// Empty roots are universal; nonempty roots must belong to this store.
    pub fn contains(&self, root: Root) -> bool {
        self.index.contains(root)
    }
    pub fn support(&self, root: Root, rule: usize, heads: &Arc<Vec<u64>>) -> Condition {
        assert!(self.contains(root), "stale or foreign history root");
        let key = Key {
            rule,
            heads: heads.clone(),
        };
        self.lookup
            .get(&key)
            .and_then(|&id| self.index.get(root, &[id, 0, 0, 0]))
            .unwrap_or(Condition::FALSE)
    }
    /// Replace support atomically. An absent FALSE write does not intern a key.
    /// Tuple comparison has its usual lexicographic cost; lookup is logarithmic.
    pub fn set_support(
        &mut self,
        root: Root,
        rule: usize,
        heads: Arc<Vec<u64>>,
        support: Condition,
    ) -> Root {
        assert!(self.contains(root), "stale or foreign history root");
        let key = Key { rule, heads };
        let id = match self.lookup.get(&key) {
            Some(&id) => id,
            None if support == Condition::FALSE => return root,
            None => {
                let id = self.next_id;
                self.next_id = id.checked_add(1).expect("history identity exhausted");
                self.records.insert(
                    id,
                    Record {
                        key: key.clone(),
                        marked: 0,
                    },
                );
                self.lookup.insert(key, id);
                id
            }
        };
        if support == Condition::FALSE {
            self.index.remove(root, &[id, 0, 0, 0])
        } else {
            self.index.insert(root, [id, 0, 0, 0], support)
        }
    }
    pub fn record_count(&self) -> usize {
        self.records.len()
    }
    pub fn node_count(&self) -> usize {
        self.index.node_count()
    }
    pub fn entries(&self, root: Root) -> Entries {
        Entries {
            cursor: self.index.range(root, [0, 0, 0, 0], [u64::MAX, 0, 0, 0]),
        }
    }
    /// Include current, staged, and suspended entry-cursor roots. Feed emitted
    /// supports to condition collection along with other live condition roots.
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<'_, I> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("history collection epoch exhausted");
        Collector {
            index: self.index.collect(roots),
            lookup: &mut self.lookup,
            records: &mut self.records,
            epoch: self.epoch,
            sweep: None,
            done: false,
        }
    }
}

pub struct Entry {
    pub rule: usize,
    pub heads: Arc<Vec<u64>>,
    pub support: Condition,
}
pub struct Entries {
    cursor: Cursor,
}
impl Entries {
    pub fn root(&self) -> Root {
        self.cursor.root()
    }
    pub fn next(&mut self, history: &History) -> Option<Entry> {
        let (key, support) = self.cursor.next(&history.index)?;
        let record = &history.records[&key[0]];
        Some(Entry {
            rule: record.key.rule,
            heads: record.key.heads.clone(),
            support,
        })
    }
}

pub struct Collector<'a, I> {
    index: store::Collector<'a, Condition, I>,
    lookup: &'a mut BTreeMap<Key, u64>,
    records: &'a mut BTreeMap<u64, Record>,
    epoch: u64,
    sweep: Option<u64>,
    done: bool,
}
impl<I: Iterator<Item = Root>> Collector<'_, I> {
    pub fn done(&self) -> bool {
        self.done
    }
    /// Advance one store action or one metadata record. Tuple lookup/removal
    /// retains the normal comparison cost of that record's ordered heads.
    pub fn tick(&mut self) -> Option<Condition> {
        if self.done {
            return None;
        }
        if !self.index.done() {
            if let Some((key, support)) = self.index.tick() {
                self.records
                    .get_mut(&key[0])
                    .expect("live history metadata")
                    .marked = self.epoch;
                return Some(support);
            }
        } else {
            let next = match self.sweep {
                Some(id) => self.records.range((Excluded(id), Unbounded)).next(),
                None => self.records.first_key_value(),
            };
            if let Some((&id, record)) = next {
                if record.marked != self.epoch {
                    self.lookup.remove(&record.key);
                    self.records.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}
