//! Read-only merge-delta activation on a frozen graph root.
//!
//! Members supply supported aliases; incidence prefixes supply occurrences.
//! Each occurrence is emitted only on support not already returned by this walk.

use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::graph::{Graph, Occurrences};
use crate::identity::{MergeDelta, ResolveStatus};
use crate::members::Members;
use crate::store::Root;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeStatus {
    Pending,
    Found { occurrence: u64, support: Condition },
    Done,
}
#[derive(Clone, Copy)]
enum Phase {
    Members,
    Scan,
    Intersect,
    Novel,
    Remember,
    Cleanup,
    Done,
}

/// Pass the merged graph root and changed support. Retain `root()` for graph
/// collection and `condition_roots()` for condition collection while suspended.
/// Callers retain returned supports; disjoint fragments may arrive separately.
pub struct Wake {
    members: Members,
    seeds: VecDeque<(u64, Condition)>,
    cursor: Option<Occurrences>,
    member_support: Condition,
    occurrence: u64,
    fresh: Condition,
    seen: BTreeMap<u64, Condition>,
    boolean: Option<Job>,
    phase: Phase,
    discard: u8,
}
impl Wake {
    pub fn new(g: &Graph, root: Root, variable: u64, scope: Condition) -> Self {
        Self {
            discard: 0,
            members: Members::new(g, root, variable, scope),
            seeds: VecDeque::new(),
            cursor: None,
            member_support: Condition::FALSE,
            occurrence: 0,
            fresh: Condition::FALSE,
            seen: BTreeMap::new(),
            boolean: None,
            phase: Phase::Members,
        }
    }
    /// Consume the delta's paired root and seeds. This is subtree activation;
    /// `new` independently enumerates a full class for low-level callers.
    /// On each seed context the new parent leaves the loser, so its descendants
    /// are exactly its old class. Every newly satisfied head equality has a
    /// port in that class, in a repeated head-variable slot. Those occurrences
    /// therefore provide complete anchors even with identity-only dispatch.
    pub fn from_delta(g: &Graph, delta: MergeDelta) -> Self {
        let (root, mut seeds) = delta.into_parts();
        let (variable, scope) = seeds.pop_front().expect("changed merge seed");
        Self {
            discard: 0,
            members: Members::subtree(g, root, variable, scope),
            seeds,
            cursor: None,
            member_support: Condition::FALSE,
            occurrence: 0,
            fresh: Condition::FALSE,
            seen: BTreeMap::new(),
            boolean: None,
            phase: Phase::Members,
        }
    }
    pub fn root(&self) -> Root {
        self.members.root()
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.members
            .condition_roots()
            .chain([self.member_support, self.fresh])
            .chain(self.seen.values().copied())
            .chain(self.seeds.iter().map(|(_, c)| *c))
            .chain(self.boolean.iter().flat_map(Job::roots))
    }
    fn poll(&mut self, a: &mut Arena) -> Option<Condition> {
        if let Progress::Complete(c) = self
            .boolean
            .as_mut()
            .expect("condition continuation")
            .tick(a)
        {
            self.boolean = None;
            Some(c)
        } else {
            None
        }
    }
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.fresh = Condition::FALSE;
            self.member_support = Condition::FALSE;
            self.cursor = None;
        }
        match self.discard {
            1 => {
                if let Some(j) = &mut self.boolean {
                    if j.discard_tick() {
                        self.boolean = None;
                    }
                } else {
                    self.discard = 2;
                }
            }
            2 => {
                if self.members.discard_tick() {
                    self.discard = 3;
                }
            }
            3 => {
                if self.seen.pop_first().is_none() {
                    self.discard = 4;
                }
            }
            4 => {
                if self.seeds.pop_front().is_none() {
                    self.seeds = VecDeque::new();
                    self.discard = 5;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> WakeStatus {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        match self.phase {
            Phase::Members => match self.members.tick(g, a) {
                ResolveStatus::Found { variable, support } => {
                    self.member_support = support;
                    self.cursor = Some(g.incidence(self.root(), variable));
                    self.phase = Phase::Scan;
                }
                ResolveStatus::Pending => {}
                ResolveStatus::Done => {
                    if let Some((v, c)) = self.seeds.pop_front() {
                        self.members = Members::subtree(g, self.root(), v, c);
                        return WakeStatus::Pending;
                    }
                    self.seeds = VecDeque::new();
                    self.member_support = Condition::FALSE;
                    self.fresh = Condition::FALSE;
                    self.phase = Phase::Cleanup;
                }
            },
            Phase::Scan => {
                if let Some((occurrence, support)) =
                    self.cursor.as_mut().expect("incidence cursor").next(g)
                {
                    self.occurrence = occurrence;
                    self.boolean = Some(a.start(Operation::And(self.member_support, support)));
                    self.phase = Phase::Intersect;
                } else {
                    self.cursor = None;
                    self.phase = Phase::Members;
                }
            }
            Phase::Intersect => {
                if let Some(c) = self.poll(a) {
                    if c == Condition::FALSE {
                        self.phase = Phase::Scan;
                    } else {
                        let seen = self
                            .seen
                            .get(&self.occurrence)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(a.start(Operation::Difference(c, seen)));
                        self.phase = Phase::Novel;
                    }
                }
            }
            Phase::Novel => {
                if let Some(c) = self.poll(a) {
                    self.fresh = c;
                    if c == Condition::FALSE {
                        self.phase = Phase::Scan;
                    } else {
                        let seen = self
                            .seen
                            .get(&self.occurrence)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(a.start(Operation::Or(seen, c)));
                        self.phase = Phase::Remember;
                    }
                }
            }
            Phase::Remember => {
                if let Some(c) = self.poll(a) {
                    self.seen.insert(self.occurrence, c);
                    self.phase = Phase::Scan;
                    return WakeStatus::Found {
                        occurrence: self.occurrence,
                        support: self.fresh,
                    };
                }
            }
            Phase::Cleanup => {
                if self.seen.pop_first().is_none() {
                    self.phase = Phase::Done;
                    return WakeStatus::Done;
                }
            }
            Phase::Done => return WakeStatus::Done,
        }
        WakeStatus::Pending
    }
}

impl Trace for Wake {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.optional(Some(&self.members)),
            1 => cursor.fields(&[self.member_support, self.fresh]),
            2 => cursor.values(&self.seen),
            3 => cursor.optional(self.boolean.as_ref()),
            4 => cursor.vector(self.seeds.len(), |i, child| {
                if child.phase == 0 {
                    child.fields(&[self.seeds[i].1])
                } else {
                    Step::Done
                }
            }),
            _ => Step::Done,
        }
    }
}
