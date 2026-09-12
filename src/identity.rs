//! Conditional variable equivalence, with nonbinding reads and staged unions.
//!
//! Parent and rank partitions are disjoint within one variable's record. Union
//! by rank bounds projected forest depth; opposite parent directions in disjoint
//! alternatives are permitted. Every condition operation can yield. Reading
//! identity never alters the graph or identifies variables to satisfy a head.

use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::graph::Graph;
use crate::store::{Cursor, Key, Root};
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};

pub(crate) const PARENT: u64 = 4;
pub(crate) const CHILD: u64 = 5;
pub(crate) const RANK: u64 = 6;

fn poll(job: &mut Option<Job>, a: &mut Arena) -> Option<Condition> {
    if let Progress::Complete(c) = job.as_mut().expect("condition continuation").tick(a) {
        *job = None;
        Some(c)
    } else {
        None
    }
}
#[derive(Clone, Copy)]
enum PartPhase {
    Scan,
    Hit,
    Subtract,
    Emit,
    Done,
}
struct Partition {
    cursor: Cursor,
    remaining: Condition,
    default: u64,
    member: u64,
    edge: Condition,
    hit: Condition,
    boolean: Option<Job>,
    phase: PartPhase,
}
impl Partition {
    fn new(
        g: &Graph,
        root: Root,
        namespace: u64,
        variable: u64,
        default: u64,
        scope: Condition,
    ) -> Self {
        Self {
            cursor: g.index.range(
                root,
                [namespace, variable, 0, 0],
                [namespace, variable, u64::MAX, 0],
            ),
            remaining: scope,
            default,
            member: default,
            edge: Condition::FALSE,
            hit: Condition::FALSE,
            boolean: None,
            phase: PartPhase::Scan,
        }
    }
    fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.remaining, self.edge, self.hit]
            .into_iter()
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
    }
    fn discard_tick(&mut self) -> bool {
        self.remaining = Condition::FALSE;
        self.edge = Condition::FALSE;
        self.hit = Condition::FALSE;
        if let Some(job) = &mut self.boolean {
            if job.discard_tick() {
                self.boolean = None;
            }
            return false;
        }
        true
    }
    fn tick(&mut self, g: &Graph, a: &mut Arena) -> ResolveStatus {
        match self.phase {
            PartPhase::Scan => {
                if self.remaining == Condition::FALSE {
                    self.phase = PartPhase::Done;
                    return ResolveStatus::Done;
                }
                if let Some((key, c)) = self.cursor.next(&g.index) {
                    self.member = key[2];
                    self.edge = c;
                    self.boolean = Some(a.start(Operation::And(self.remaining, c)));
                    self.phase = PartPhase::Hit;
                } else {
                    self.member = self.default;
                    self.hit = self.remaining;
                    self.remaining = Condition::FALSE;
                    self.phase = PartPhase::Emit;
                }
            }
            PartPhase::Hit => {
                if let Some(hit) = poll(&mut self.boolean, a) {
                    self.hit = hit;
                    if hit == Condition::FALSE {
                        self.phase = PartPhase::Scan;
                    } else {
                        self.boolean =
                            Some(a.start(Operation::Difference(self.remaining, self.edge)));
                        self.phase = PartPhase::Subtract;
                    }
                }
            }
            PartPhase::Subtract => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.remaining = c;
                    self.phase = PartPhase::Emit;
                }
            }
            PartPhase::Emit => {
                self.phase = PartPhase::Scan;
                return ResolveStatus::Found {
                    variable: self.member,
                    support: self.hit,
                };
            }
            PartPhase::Done => return ResolveStatus::Done,
        }
        ResolveStatus::Pending
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveStatus {
    Pending,
    Found { variable: u64, support: Condition },
    Done,
}
#[derive(Clone, Copy)]
enum ResolvePhase {
    Uniform,
    Next,
    Novel,
    Remember,
    Walk,
    Enqueue,
    Cleanup,
    Done,
}

pub struct Resolve {
    root: Root,
    initial: Option<(u64, Condition)>,
    pending: BTreeMap<u64, Condition>,
    queue: VecDeque<u64>,
    visited: BTreeMap<u64, Condition>,
    variable: u64,
    fresh: Condition,
    parent: u64,
    carry: Condition,
    boolean: Option<Job>,
    partition: Option<Partition>,
    phase: ResolvePhase,
    visits: u64,
    discarding: bool,
}
impl Resolve {
    pub fn new(g: &Graph, root: Root, variable: u64, scope: Condition) -> Self {
        assert!(g.index.contains(&root), "stale or foreign identity root");
        Self {
            root,
            initial: Some((variable, scope)),
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            visited: BTreeMap::new(),
            variable,
            fresh: Condition::FALSE,
            parent: variable,
            carry: scope,
            boolean: None,
            partition: None,
            phase: ResolvePhase::Uniform,
            visits: 0,
            discarding: false,
        }
    }
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    pub fn visits(&self) -> u64 {
        self.visits
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.fresh, self.carry]
            .into_iter()
            .chain(self.initial.iter().map(|p| p.1))
            .chain(self.pending.values().copied())
            .chain(self.visited.values().copied())
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
            .chain(self.partition.iter().flat_map(|p| p.roots()))
    }
    /// Cancel immediately, then release one child-job step or one map entry
    /// per call. The immutable graph root remains owned by this token.
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.initial = None;
        self.fresh = Condition::FALSE;
        self.carry = Condition::FALSE;
        self.queue = VecDeque::new(); // Scalar variable IDs, without child owners.
        if let Some(job) = self.boolean.as_mut() {
            if job.discard_tick() {
                self.boolean = None;
            }
            return false;
        }
        if let Some(partition) = self.partition.as_mut() {
            if let Some(job) = partition.boolean.as_mut() {
                if job.discard_tick() {
                    partition.boolean = None;
                }
            } else {
                self.partition = None; // Cursor backing contains only scalar roots.
            }
            return false;
        }
        if self.pending.pop_first().is_some() {
            return false;
        }
        if self.visited.pop_first().is_some() {
            return false;
        }
        true
    }

    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> ResolveStatus {
        assert!(!self.discarding, "identity resolve has been discarded");
        match self.phase {
            ResolvePhase::Uniform => {
                let (variable, scope) = self.initial.take().expect("uniform identity frontier");
                assert!(a.contains(scope), "stale or foreign condition operand");
                if scope == Condition::FALSE {
                    self.carry = Condition::FALSE;
                    self.phase = ResolvePhase::Done;
                    return ResolveStatus::Done;
                }
                let mut cursor = g.index.range(
                    self.root.clone(),
                    [PARENT, variable, 0, 0],
                    [PARENT, variable, u64::MAX, 0],
                );
                if let Some((key, edge)) = cursor.next(&g.index) {
                    // Parent partitions are disjoint. An edge covering the
                    // whole scope certifies one hop in every projected forest.
                    // The pinned root keeps that certificate valid while paused.
                    if edge == Condition::TRUE || edge == scope {
                        self.visits += 1;
                        self.initial = Some((key[2], scope));
                    } else {
                        self.initial = Some((variable, scope));
                        self.phase = ResolvePhase::Next;
                    }
                    return ResolveStatus::Pending;
                }
                self.visits += 1;
                self.carry = Condition::FALSE;
                self.phase = ResolvePhase::Done;
                return ResolveStatus::Found {
                    variable,
                    support: scope,
                };
            }
            ResolvePhase::Next => {
                let next = self.initial.take().or_else(|| {
                    self.queue
                        .pop_front()
                        .map(|v| (v, self.pending.remove(&v).expect("queued variable")))
                });
                if let Some((v, c)) = next {
                    self.variable = v;
                    self.carry = c;
                    let old = self.visited.get(&v).copied().unwrap_or(Condition::FALSE);
                    self.boolean = Some(a.start(Operation::Difference(c, old)));
                    self.phase = ResolvePhase::Novel;
                } else {
                    self.phase = ResolvePhase::Cleanup;
                }
            }
            ResolvePhase::Novel => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.fresh = c;
                    if c == Condition::FALSE {
                        self.phase = ResolvePhase::Next;
                    } else {
                        let old = self
                            .visited
                            .get(&self.variable)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(a.start(Operation::Or(old, c)));
                        self.phase = ResolvePhase::Remember;
                    }
                }
            }
            ResolvePhase::Remember => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.visited.insert(self.variable, c);
                    self.visits += 1;
                    self.partition = Some(Partition::new(
                        g,
                        self.root.clone(),
                        PARENT,
                        self.variable,
                        self.variable,
                        self.fresh,
                    ));
                    self.phase = ResolvePhase::Walk;
                }
            }
            ResolvePhase::Walk => match self.partition.as_mut().expect("parent scan").tick(g, a) {
                ResolveStatus::Found { variable, support } => {
                    if variable == self.variable {
                        return ResolveStatus::Found { variable, support };
                    }
                    self.parent = variable;
                    self.carry = support;
                    let old = self
                        .pending
                        .get(&variable)
                        .copied()
                        .unwrap_or(Condition::FALSE);
                    self.boolean = Some(a.start(Operation::Or(old, support)));
                    self.phase = ResolvePhase::Enqueue;
                }
                ResolveStatus::Done => {
                    self.partition = None;
                    self.phase = ResolvePhase::Next;
                }
                ResolveStatus::Pending => {}
            },
            ResolvePhase::Enqueue => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    if self.pending.insert(self.parent, c).is_none() {
                        self.queue.push_back(self.parent);
                    }
                    self.phase = ResolvePhase::Walk;
                }
            }
            ResolvePhase::Cleanup => {
                if self.pending.pop_first().is_none() && self.visited.pop_first().is_none() {
                    self.queue = VecDeque::new();
                    self.phase = ResolvePhase::Done;
                    return ResolveStatus::Done;
                }
            }
            ResolvePhase::Done => return ResolveStatus::Done,
        }
        ResolveStatus::Pending
    }
}

struct Resolved {
    resolve: Resolve,
    map: BTreeMap<u64, Condition>,
    boolean: Option<Job>,
    variable: u64,
    carry: Condition,
    done: bool,
}
impl Resolved {
    fn new(g: &Graph, r: Root, v: u64, c: Condition) -> Self {
        Self {
            resolve: Resolve::new(g, r, v, c),
            map: BTreeMap::new(),
            boolean: None,
            variable: v,
            carry: c,
            done: false,
        }
    }
    fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.resolve
            .condition_roots()
            .chain(self.map.values().copied())
            .chain([self.carry])
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
    }
    fn discard_tick(&mut self) -> bool {
        self.carry = Condition::FALSE;
        if let Some(job) = &mut self.boolean {
            if job.discard_tick() {
                self.boolean = None;
            }
            return false;
        }
        if !self.resolve.discard_tick() {
            return false;
        }
        self.map.pop_first().is_none()
    }
    fn tick(&mut self, g: &Graph, a: &mut Arena) -> bool {
        if self.done {
            return true;
        }
        if self.boolean.is_some() {
            if let Some(c) = poll(&mut self.boolean, a) {
                self.map.insert(self.variable, c);
            }
        } else {
            match self.resolve.tick(g, a) {
                ResolveStatus::Found { variable, support } => {
                    self.variable = variable;
                    self.carry = support;
                    let old = self.map.get(&variable).copied().unwrap_or(Condition::FALSE);
                    self.boolean = Some(a.start(Operation::Or(old, support)));
                }
                ResolveStatus::Done => self.done = true,
                ResolveStatus::Pending => {}
            }
        }
        self.done
    }
}

#[derive(Clone, Copy)]
enum EqualPhase {
    Left,
    Right,
    Pair,
    Intersect,
    Accumulate,
    Cleanup,
    Done,
}
/// Supported identity test. Both inputs and their graph root remain unchanged.
pub struct Equal {
    root: Root,
    left: Resolved,
    right: Resolved,
    last: Option<u64>,
    boolean: Option<Job>,
    carry: Condition,
    result: Condition,
    phase: EqualPhase,
    discard: u8,
}
impl Equal {
    pub fn new(g: &Graph, root: Root, x: u64, y: u64, scope: Condition) -> Self {
        Self {
            discard: 0,
            root: root.clone(),
            left: Resolved::new(g, root.clone(), x, scope),
            right: Resolved::new(g, root, y, scope),
            last: None,
            boolean: None,
            carry: scope,
            result: if x == y { scope } else { Condition::FALSE },
            phase: if x == y || scope == Condition::FALSE {
                EqualPhase::Done
            } else {
                EqualPhase::Left
            },
        }
    }
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.left
            .roots()
            .chain(self.right.roots())
            .chain([self.carry, self.result])
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
    }
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.carry = Condition::FALSE;
            self.result = Condition::FALSE;
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
                if self.left.discard_tick() {
                    self.discard = 3;
                }
            }
            3 => {
                if self.right.discard_tick() {
                    self.discard = 4;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> Option<Condition> {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        match self.phase {
            EqualPhase::Left => {
                if self.left.tick(g, a) {
                    self.phase = EqualPhase::Right;
                }
            }
            EqualPhase::Right => {
                if self.right.tick(g, a) {
                    self.phase = EqualPhase::Pair;
                }
            }
            EqualPhase::Pair => {
                let next = match self.last {
                    Some(v) => self.left.map.range((Excluded(v), Unbounded)).next(),
                    None => self.left.map.first_key_value(),
                };
                if let Some((&v, &c)) = next {
                    self.last = Some(v);
                    if let Some(&d) = self.right.map.get(&v) {
                        self.boolean = Some(a.start(Operation::And(c, d)));
                        self.phase = EqualPhase::Intersect;
                    }
                } else {
                    self.phase = EqualPhase::Cleanup;
                }
            }
            EqualPhase::Intersect => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.carry = c;
                    self.boolean = Some(a.start(Operation::Or(self.result, c)));
                    self.phase = EqualPhase::Accumulate;
                }
            }
            EqualPhase::Accumulate => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.result = c;
                    self.phase = EqualPhase::Pair;
                }
            }
            EqualPhase::Cleanup => {
                if self.left.map.pop_first().is_none() && self.right.map.pop_first().is_none() {
                    self.phase = EqualPhase::Done;
                }
            }
            EqualPhase::Done => return Some(self.result),
        }
        None
    }
}

#[derive(Clone, Copy)]
enum MergePhase {
    Left,
    Right,
    Pair,
    Support,
    LeftRank,
    RightRank,
    Edits,
    Cleanup,
    Done,
}
struct Edit {
    key: Option<Key>,
    remove: bool,
    context: Condition,
}
/// Owned activation delta from one completed merge. Its seeds cannot be paired
/// with another root. Retain `root()` and trace its conditions across collection;
/// move it into `Wake::from_delta` or finish incremental cancellation.
#[must_use]
pub struct MergeDelta {
    root: Root,
    seeds: VecDeque<(u64, Condition)>,
    discarding: bool,
}
impl MergeDelta {
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.seeds.iter().map(|(_, c)| *c)
    }
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        if self.seeds.pop_front().is_some() {
            return false;
        }
        self.seeds = VecDeque::new();
        true
    }
    pub(crate) fn into_parts(self) -> (Root, VecDeque<(u64, Condition)>) {
        assert!(!self.discarding, "discarded merge delta cannot activate");
        (self.root, self.seeds)
    }
}
impl Trace for MergeDelta {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.vector(self.seeds.len(), |i, child| {
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

/// Staged union: only the returned complete root may replace the query root.
/// Parent/rank writes and reverse incidence publish together. `changed_support`
/// identifies the region requiring equality-sensitive match activation.
pub struct Merge {
    base: Root,
    staged: Root,
    left: Resolved,
    right: Resolved,
    l: Option<u64>,
    r: Option<u64>,
    scope: Condition,
    pair: Condition,
    left_rank: Option<Partition>,
    right_rank: Option<Partition>,
    rank: u64,
    changed: Condition,
    delta: VecDeque<(u64, Condition)>,
    edits: VecDeque<Edit>,
    boolean: Option<Job>,
    phase: MergePhase,
    discard: u8,
}
impl Merge {
    pub fn new(g: &Graph, root: Root, x: u64, y: u64, scope: Condition) -> Self {
        Self {
            discard: 0,
            base: root.clone(),
            staged: root.clone(),
            left: Resolved::new(g, root.clone(), x, scope),
            right: Resolved::new(g, root, y, scope),
            l: None,
            r: None,
            scope,
            pair: Condition::FALSE,
            left_rank: None,
            right_rank: None,
            rank: 0,
            changed: Condition::FALSE,
            delta: VecDeque::new(),
            edits: VecDeque::new(),
            boolean: None,
            phase: if x == y || scope == Condition::FALSE {
                MergePhase::Done
            } else {
                MergePhase::Left
            },
        }
    }
    pub fn roots(&self) -> [Root; 2] {
        [self.base.clone(), self.staged.clone()]
    }
    /// Transfer the completed merge's activation obligation exactly once.
    /// An unchanged merge, or a subsequent take, returns None.
    pub fn take_delta(&mut self) -> Option<MergeDelta> {
        assert_eq!(self.discard, 0, "discarded merge cannot transfer a delta");
        assert!(matches!(self.phase, MergePhase::Done));
        if self.delta.is_empty() {
            return None;
        }
        Some(MergeDelta {
            root: self.staged.clone(),
            seeds: std::mem::take(&mut self.delta),
            discarding: false,
        })
    }
    pub fn changed_support(&self) -> Condition {
        self.changed
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.left
            .roots()
            .chain(self.right.roots())
            .chain([self.scope, self.pair, self.changed])
            .chain(self.left_rank.iter().flat_map(|p| p.roots()))
            .chain(self.right_rank.iter().flat_map(|p| p.roots()))
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
            .chain(self.edits.iter().map(|e| e.context))
            .chain(self.delta.iter().map(|(_, c)| *c))
    }
    fn next_pair(&mut self) -> Option<(u64, Condition, u64, Condition)> {
        if self.l.is_none() {
            self.l = self.left.map.first_key_value().map(|(&v, _)| v);
        }
        let l = self.l?;
        let r = match self.r {
            Some(v) => self.right.map.range((Excluded(v), Unbounded)).next(),
            None => self.right.map.first_key_value(),
        };
        if let Some((&r, &c)) = r {
            self.r = Some(r);
            return Some((l, self.left.map[&l], r, c));
        }
        self.l = self
            .left
            .map
            .range((Excluded(l), Unbounded))
            .next()
            .map(|(&v, _)| v);
        self.r = None;
        let l = self.l?;
        let (&r, &c) = self.right.map.first_key_value()?;
        self.r = Some(r);
        Some((l, self.left.map[&l], r, c))
    }
    fn stage_links(&mut self, rank_right: u64, context: Condition) {
        let l = self.l.expect("left representative");
        let r = self.r.expect("right representative");
        let (winner, loser) = if self.rank > rank_right || (self.rank == rank_right && l < r) {
            (l, r)
        } else {
            (r, l)
        };
        self.delta.push_back((loser, context));
        self.edits.push_back(Edit {
            key: Some([PARENT, loser, winner, 0]),
            remove: false,
            context,
        });
        self.edits.push_back(Edit {
            key: Some([CHILD, winner, loser, 0]),
            remove: false,
            context,
        });
        if self.rank == rank_right {
            if self.rank != 0 {
                self.edits.push_back(Edit {
                    key: Some([RANK, winner, self.rank, 0]),
                    remove: true,
                    context,
                });
            }
            self.edits.push_back(Edit {
                key: Some([
                    RANK,
                    winner,
                    self.rank.checked_add(1).expect("rank exhausted"),
                    0,
                ]),
                remove: false,
                context,
            });
        }
        self.edits.push_back(Edit {
            key: None,
            remove: false,
            context,
        });
        self.phase = MergePhase::Edits;
    }
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.scope = Condition::FALSE;
            self.pair = Condition::FALSE;
            self.changed = Condition::FALSE;
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
                if self.left.discard_tick() {
                    self.discard = 3;
                }
            }
            3 => {
                if self.right.discard_tick() {
                    self.discard = 4;
                }
            }
            4 => {
                if let Some(p) = &mut self.left_rank {
                    if p.discard_tick() {
                        self.left_rank = None;
                    }
                } else {
                    self.discard = 5;
                }
            }
            5 => {
                if let Some(p) = &mut self.right_rank {
                    if p.discard_tick() {
                        self.right_rank = None;
                    }
                } else {
                    self.discard = 6;
                }
            }
            6 => {
                if self.edits.pop_front().is_none() {
                    self.edits = VecDeque::new();
                    self.discard = 7;
                }
            }
            7 => {
                if self.delta.pop_front().is_none() {
                    self.delta = VecDeque::new();
                    self.discard = 8;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(&mut self, g: &mut Graph, a: &mut Arena) -> Option<Root> {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        match self.phase {
            MergePhase::Left => {
                if self.left.tick(g, a) {
                    self.phase = MergePhase::Right;
                }
            }
            MergePhase::Right => {
                if self.right.tick(g, a) {
                    self.phase = MergePhase::Pair;
                }
            }
            MergePhase::Pair => {
                if let Some((x, c, y, d)) = self.next_pair() {
                    if x != y {
                        self.boolean = Some(a.start(Operation::And(c, d)));
                        self.phase = MergePhase::Support;
                    }
                } else {
                    self.phase = MergePhase::Cleanup;
                }
            }
            MergePhase::Support => {
                if let Some(c) = poll(&mut self.boolean, a) {
                    self.pair = c;
                    if c == Condition::FALSE {
                        self.phase = MergePhase::Pair;
                    } else {
                        let l = self.l.expect("left representative");
                        self.left_rank = Some(Partition::new(g, self.base.clone(), RANK, l, 0, c));
                        self.phase = MergePhase::LeftRank;
                    }
                }
            }
            MergePhase::LeftRank => match self.left_rank.as_mut().expect("left rank").tick(g, a) {
                ResolveStatus::Found {
                    variable: rank,
                    support,
                } => {
                    self.rank = rank;
                    let r = self.r.expect("right representative");
                    self.right_rank =
                        Some(Partition::new(g, self.base.clone(), RANK, r, 0, support));
                    self.phase = MergePhase::RightRank;
                }
                ResolveStatus::Done => {
                    self.left_rank = None;
                    self.phase = MergePhase::Pair;
                }
                ResolveStatus::Pending => {}
            },
            MergePhase::RightRank => match self.right_rank.as_mut().expect("right rank").tick(g, a)
            {
                ResolveStatus::Found {
                    variable: rank,
                    support,
                } => self.stage_links(rank, support),
                ResolveStatus::Done => {
                    self.right_rank = None;
                    self.phase = MergePhase::LeftRank;
                }
                ResolveStatus::Pending => {}
            },
            MergePhase::Edits => {
                if self.boolean.is_some() {
                    if let Some(c) = poll(&mut self.boolean, a) {
                        let edit = self.edits.pop_front().expect("pending edit");
                        if let Some(key) = edit.key {
                            self.staged = g.write(std::mem::take(&mut self.staged), key, c);
                        } else {
                            self.changed = c;
                        }
                    }
                } else if let Some(edit) = self.edits.front() {
                    let old = edit.key.map_or(self.changed, |key| {
                        g.index.get(&self.staged, &key).unwrap_or(Condition::FALSE)
                    });
                    self.boolean = Some(a.start(if edit.remove {
                        Operation::Difference(old, edit.context)
                    } else {
                        Operation::Or(old, edit.context)
                    }));
                } else {
                    self.phase = MergePhase::RightRank;
                }
            }
            MergePhase::Cleanup => {
                if self.left.map.pop_first().is_none() && self.right.map.pop_first().is_none() {
                    self.edits = VecDeque::new();
                    self.phase = MergePhase::Done;
                }
            }
            MergePhase::Done => return Some(self.staged.clone()),
        }
        None
    }
}

impl Trace for Partition {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.remaining, self.edge, self.hit]),
            1 => cursor.optional(self.boolean.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Resolve {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.fresh, self.carry]),
            1 => match self.initial {
                Some((_, support)) => cursor.fields(&[support]),
                None => cursor.fields(&[]),
            },
            2 => cursor.values(&self.pending),
            3 => cursor.values(&self.visited),
            4 => cursor.optional(self.boolean.as_ref()),
            5 => cursor.optional(self.partition.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Resolved {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.optional(Some(&self.resolve)),
            1 => cursor.values(&self.map),
            2 => cursor.fields(&[self.carry]),
            3 => cursor.optional(self.boolean.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Equal {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.optional(Some(&self.left)),
            1 => cursor.optional(Some(&self.right)),
            2 => cursor.fields(&[self.carry, self.result]),
            3 => cursor.optional(self.boolean.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Trace for Merge {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.optional(Some(&self.left)),
            1 => cursor.optional(Some(&self.right)),
            2 => cursor.fields(&[self.scope, self.pair, self.changed]),
            3 => cursor.optional(self.left_rank.as_ref()),
            4 => cursor.optional(self.right_rank.as_ref()),
            5 => cursor.optional(self.boolean.as_ref()),
            6 => cursor.vector(self.edits.len(), |i, child| {
                if child.phase == 0 {
                    child.fields(&[self.edits[i].context])
                } else {
                    Step::Done
                }
            }),
            7 => cursor.vector(self.delta.len(), |i, child| {
                if child.phase == 0 {
                    child.fields(&[self.delta[i].1])
                } else {
                    Step::Done
                }
            }),
            _ => Step::Done,
        }
    }
}
