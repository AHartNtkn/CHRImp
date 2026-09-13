//! Snapshot-local support summaries. Construction and search yield at each
//! Boolean operation and tree action; payloads are rows, never answers.
use crate::condition::{Arena, Condition, Job, Operation, poll};
use crate::gc::discard_slot;
use crate::trace::{Cursor, Step, Trace};

#[derive(Clone, Copy)]
struct Node {
    support: Condition,
    children: Option<(usize, usize)>,
    value: u64,
}
#[derive(Default)]
pub(super) struct Index {
    nodes: Vec<Node>,
    level: Vec<usize>,
    next: Vec<usize>,
    pair: usize,
    boolean: Option<Job>,
}
impl Index {
    pub fn insert(&mut self, value: u64, support: Condition) {
        self.level.push(self.nodes.len());
        self.nodes.push(Node {
            support,
            children: None,
            value,
        });
    }
    pub fn build(&mut self, a: &mut Arena) -> bool {
        if self.level.len() <= 1 {
            return true;
        }
        if self.pair == self.level.len() {
            self.level = std::mem::take(&mut self.next);
            self.pair = 0;
        } else if self.pair + 1 == self.level.len() {
            self.next.push(self.level[self.pair]);
            self.pair += 1;
        } else {
            let l = self.level[self.pair];
            let r = self.level[self.pair + 1];
            if self.boolean.is_none() {
                self.boolean =
                    Some(a.start(Operation::Or(self.nodes[l].support, self.nodes[r].support)));
            } else if let Some(support) = poll(&mut self.boolean, a) {
                self.next.push(self.nodes.len());
                self.nodes.push(Node {
                    support,
                    children: Some((l, r)),
                    value: 0,
                });
                self.pair += 2;
            }
        }
        false
    }
    pub fn search(&self, scope: Condition) -> Search {
        Search {
            pending: self
                .level
                .first()
                .map(|&i| vec![(i, scope)])
                .unwrap_or_default(),
            current: None,
            boolean: None,
        }
    }
    pub fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.nodes
            .iter()
            .map(|n| n.support)
            .chain(self.boolean.iter().flat_map(Job::roots))
    }
    pub fn discard_tick(&mut self) -> bool {
        self.nodes = Vec::new();
        self.level = Vec::new();
        self.next = Vec::new();
        if discard_slot(&mut self.boolean, |child| child.discard_tick()) {
            return false;
        }
        true
    }
}
pub(super) enum Found {
    Pending,
    Value(u64),
    Done,
}
pub(super) struct Search {
    pending: Vec<(usize, Condition)>,
    current: Option<(usize, Condition)>,
    boolean: Option<Job>,
}
impl Search {
    // Each child history narrows the current scope. Subtrees already rejected
    // remain irrelevant, so siblings share just the unvisited node IDs.
    pub fn continuation(&self) -> std::sync::Arc<Vec<usize>> {
        std::sync::Arc::new(self.pending.iter().map(|&(i, _)| i).collect())
    }
    pub fn resume(scope: Condition, pending: &[usize]) -> Self {
        Self {
            pending: pending.iter().map(|&i| (i, scope)).collect(),
            current: None,
            boolean: None,
        }
    }

    pub fn tick(&mut self, index: &Index, a: &mut Arena) -> Found {
        if let Some((i, _)) = self.current {
            if let Some(scope) = poll(&mut self.boolean, a) {
                self.current = None;
                if scope != Condition::FALSE {
                    let node = index.nodes[i];
                    if let Some((l, r)) = node.children {
                        self.pending.extend([(r, scope), (l, scope)]);
                    } else {
                        return Found::Value(node.value);
                    }
                }
            }
        } else if let Some((i, scope)) = self.pending.pop() {
            let node = index.nodes[i];
            self.current = Some((i, scope));
            self.boolean = Some(a.start(Operation::And(scope, node.support)));
        } else {
            return Found::Done;
        }
        Found::Pending
    }
    pub fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.pending
            .iter()
            .chain(self.current.iter())
            .map(|(_, c)| *c)
            .chain(self.boolean.iter().flat_map(Job::roots))
    }
    pub fn discard_tick(&mut self) -> bool {
        self.pending = Vec::new();
        self.current = None;
        if discard_slot(&mut self.boolean, |child| child.discard_tick()) {
            return false;
        }
        true
    }
}
impl Trace for Index {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        match cursor.phase {
            0 => cursor.vector(self.nodes.len(), |i, c| {
                if c.phase == 0 {
                    c.fields(&[self.nodes[i].support])
                } else {
                    Step::Done
                }
            }),
            1 => cursor.optional(self.boolean.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Search {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        match cursor.phase {
            0 => cursor.vector(self.pending.len(), |i, c| {
                if c.phase == 0 {
                    c.fields(&[self.pending[i].1])
                } else {
                    Step::Done
                }
            }),
            1 => match self.current {
                Some((_, c)) => cursor.fields(&[c]),
                None => cursor.fields(&[]),
            },
            2 => cursor.optional(self.boolean.as_ref()),
            _ => Step::Done,
        }
    }
}
