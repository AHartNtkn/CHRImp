//! Live inspection-owned selection prefixes, independent of snapshot scope.
//!
//! A prefix denotes the conjunction of every selected birth region and arm.
//! Keys include exact immutable condition operands, not merely choice IDs.
//! Path ownership retains operands, but expanded results belong only to reader
//! endpoints and computation frontiers. Consumed ancestors release their result.
//! No entry survives its last owner. Snapshot ownership pins
//! the birth coordinates; collection also traces operands and suspended jobs.
use super::*;
use crate::trace::{Cursor, Step, Trace};
use std::ops::Bound::{Excluded, Unbounded};

type Key = (Option<u64>, Condition, Condition);
struct Prefix {
    key: Key,
    owners: usize,
    endpoints: usize,
    consumed: bool,
    arm: bool,
    job: Option<Job>,
    result: Option<Condition>,
}
#[derive(Default)]
pub(super) struct Restrictions {
    keys: BTreeMap<Key, u64>,
    nodes: BTreeMap<u64, Prefix>,
    next: u64,
    #[cfg(test)]
    pub(super) jobs_started: usize,
}
impl Restrictions {
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }
    pub(super) fn acquire(
        &mut self,
        parent: Option<u64>,
        support: Condition,
        decision: Condition,
    ) -> u64 {
        let key = (parent, support, decision);
        if let Some(&id) = self.keys.get(&key) {
            self.nodes.get_mut(&id).unwrap().owners += 1;
            return id;
        }
        let id = self.next;
        self.next = id.checked_add(1).expect("restriction identity exhausted");
        self.keys.insert(key, id);
        self.nodes.insert(
            id,
            Prefix {
                key,
                owners: 1,
                endpoints: 0,
                consumed: false,
                arm: false,
                job: None,
                result: None,
            },
        );
        id
    }
    pub(super) fn endpoint(&mut self, id: u64) {
        self.nodes.get_mut(&id).unwrap().endpoints += 1;
    }
    pub(super) fn release_endpoint(&mut self, id: u64) {
        let node = self.nodes.get_mut(&id).unwrap();
        node.endpoints -= 1;
        if node.endpoints == 0 && node.consumed {
            node.result = None;
        }
    }
    pub(super) fn result(&self, id: u64) -> Option<Condition> {
        self.nodes[&id].result
    }
    fn consume_parent(&mut self, id: u64) {
        if let Some(parent) = self.nodes[&id].key.0 {
            let parent = self.nodes.get_mut(&parent).unwrap();
            parent.consumed = true;
            if parent.endpoints == 0 {
                parent.result = None;
            }
        }
    }
    /// A reader carries its own predecessor, so pruning a cached ancestor
    /// cannot invalidate another reader or its independently advanced job.
    pub(super) fn tick(
        &mut self,
        id: u64,
        parent: Condition,
        arena: &mut Arena,
    ) -> Option<Condition> {
        if let Some(result) = self.nodes[&id].result {
            self.consume_parent(id);
            return Some(result);
        }
        let node = self.nodes.get_mut(&id).unwrap();
        if node.job.is_none() {
            node.arm = false;
            node.job = Some(arena.start(Operation::And(parent, node.key.1)));
            #[cfg(test)]
            {
                self.jobs_started += 1;
            }
        } else if let Some(result) = poll(&mut node.job, arena) {
            if node.arm {
                if node.endpoints > 0 || !node.consumed {
                    node.result = Some(result);
                }
                self.consume_parent(id);
                return Some(result);
            }
            node.arm = true;
            node.job = Some(arena.start(Operation::And(result, node.key.2)));
            #[cfg(test)]
            {
                self.jobs_started += 1;
            }
        }
        None
    }
    /// Release in reverse path order. The last owner takes any suspended job
    /// into its own traced, budgeted discard lane. It is no longer discoverable
    /// by newly admitted readers once disposal begins.
    pub(super) fn release(&mut self, id: u64) -> Option<Job> {
        let node = self.nodes.get_mut(&id).unwrap();
        if node.owners > 1 {
            node.owners -= 1;
            return None;
        }
        let node = self.nodes.remove(&id).unwrap();
        self.keys.remove(&node.key);
        node.job
    }
}
impl Trace for Prefix {
    fn trace(&self, c: &mut Cursor) -> Step {
        match c.phase {
            0 => c.fields(&[
                self.key.1,
                self.key.2,
                self.result.unwrap_or(Condition::FALSE),
            ]),
            1 => c.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Restrictions {
    fn trace(&self, c: &mut Cursor) -> Step {
        let next = match c.key {
            Some(id) => self.nodes.range((Excluded(id), Unbounded)).next(),
            None => self.nodes.first_key_value(),
        };
        let Some((&id, node)) = next else {
            return Step::Done;
        };
        let step = c.optional(Some(node));
        if c.phase == 1 {
            c.phase = 0;
            c.key = Some(id);
        }
        step
    }
}
