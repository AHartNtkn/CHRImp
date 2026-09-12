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
    pub(crate) index: Store<Condition>,
    lookup: BTreeMap<Key, u64>,
    records: BTreeMap<u64, Record>,
    next_id: u64,
    epoch: u64,
}

impl History {
    pub fn index_allocations(&self) -> usize {
        self.index.allocations()
    }
    pub fn release_tick(&mut self) -> bool {
        self.index.release_tick()
    }
    pub fn empty(&self) -> Root {
        self.index.empty()
    }
    /// Empty roots are universal; nonempty roots must belong to this store.
    pub fn contains(&self, root: Root) -> bool {
        self.index.contains(&root)
    }
    pub fn support(&self, root: Root, rule: usize, heads: &Arc<Vec<u64>>) -> Condition {
        assert!(self.contains(root.clone()), "stale or foreign history root");
        let key = Key {
            rule,
            heads: heads.clone(),
        };
        self.lookup
            .get(&key)
            .and_then(|&id| self.index.get(&root, &[id, 0, 0, 0]))
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
        self.index.assert_mutable();
        assert!(self.contains(root.clone()), "stale or foreign history root");
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
    /// The nested index lease freezes writes until this token is dropped,
    /// including during metadata sweep and after completion.
    pub fn collect<I: Iterator<Item = Root>>(&mut self, roots: I) -> Collector<I> {
        self.index.assert_mutable();
        let epoch = self
            .epoch
            .checked_add(1)
            .expect("history collection epoch exhausted");
        let index = self.index.collect(roots);
        self.epoch = epoch;
        Collector {
            index,
            epoch,
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

pub struct Collector<I> {
    index: store::Collector<I>,
    epoch: u64,
    sweep: Option<u64>,
    done: bool,
}
impl<I: Iterator<Item = Root>> Collector<I> {
    pub fn done(&self) -> bool {
        self.done
    }
    /// Advance one store action or one metadata record. Tuple lookup/removal
    /// retains the normal comparison cost of that record's ordered heads.
    pub fn tick(&mut self, history: &mut History) -> Option<Condition> {
        self.index.validate(&history.index);
        assert_eq!(self.epoch, history.epoch, "stale history collector");
        if self.done {
            return None;
        }
        if !self.index.done() {
            if let Some((key, support)) = self.index.tick(&mut history.index) {
                history
                    .records
                    .get_mut(&key[0])
                    .expect("live history metadata")
                    .marked = self.epoch;
                return Some(support);
            }
        } else {
            let next = match self.sweep {
                Some(id) => history.records.range((Excluded(id), Unbounded)).next(),
                None => history.records.first_key_value(),
            };
            if let Some((&id, record)) = next {
                if record.marked != self.epoch {
                    history.lookup.remove(&record.key);
                    history.records.remove(&id);
                }
                self.sweep = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}

/// Filter once-only support against unfinished execution and the surviving heads.
/// This requires a fixed graph root and exclusive ownership of history publication.
pub struct Prune {
    graph: Root,
    filter: store::Filter<Condition>,
    active: Condition,
    remaining: Condition,
    heads: Option<Arc<Vec<u64>>>,
    head: usize,
    boolean: Option<crate::condition::Job>,
}
impl History {
    pub fn prune(
        &self,
        g: &crate::graph::Graph,
        graph: Root,
        history: Root,
        active: Condition,
    ) -> Prune {
        assert!(g.index.contains(&graph), "stale or foreign graph root");
        Prune {
            graph,
            filter: self.index.filter(history),
            active,
            remaining: Condition::FALSE,
            heads: None,
            head: 0,
            boolean: None,
        }
    }
}
impl Prune {
    pub fn graph_root(&self) -> Root {
        self.graph.clone()
    }
    pub fn history_roots(&self) -> impl Iterator<Item = Root> + '_ {
        self.filter.roots()
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.active, self.remaining]
            .into_iter()
            .chain(self.boolean.iter().flat_map(crate::condition::Job::roots))
            .chain(self.filter.values())
    }
    pub fn tick(
        &mut self,
        g: &crate::graph::Graph,
        h: &mut History,
        a: &mut crate::condition::Arena,
    ) -> Option<Root> {
        use crate::condition::{Operation, Progress};
        if let Some(job) = &mut self.boolean {
            if let Progress::Complete(c) = job.tick(a) {
                self.remaining = c;
                self.boolean = None;
            }
        } else if let Some(heads) = &self.heads {
            if self.remaining == Condition::FALSE || self.head == heads.len() {
                self.filter
                    .replace((self.remaining != Condition::FALSE).then_some(self.remaining));
                self.heads = None;
            } else {
                let live = g
                    .fact(self.graph.clone(), heads[self.head])
                    .map_or(Condition::FALSE, |f| f.support);
                self.head += 1;
                self.boolean = Some(a.start(Operation::And(self.remaining, live)));
            }
        } else {
            match self.filter.tick(&mut h.index) {
                store::FilterStatus::Pending => {}
                store::FilterStatus::Complete(root) => return Some(root),
                store::FilterStatus::Leaf { key, value } => {
                    self.heads = Some(
                        h.records
                            .get(&key[0])
                            .expect("rooted history record")
                            .key
                            .heads
                            .clone(),
                    );
                    self.head = 0;
                    self.remaining = value;
                    self.boolean = Some(a.start(Operation::And(value, self.active)));
                }
            }
        }
        None
    }
}
impl crate::trace::Trace for Prune {
    fn trace(&self, c: &mut crate::trace::Cursor) -> crate::trace::Step {
        use crate::trace::Step;
        match c.phase {
            0 => c.fields(&[self.active, self.remaining]),
            1 => c.optional(self.boolean.as_ref()),
            2 => match self.filter.values().next() {
                Some(value) => c.fields(&[value]),
                None => c.advance(),
            },
            _ => Step::Done,
        }
    }
}
