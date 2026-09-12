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
}
impl Resolve {
    pub fn new(g: &Graph, root: Root, variable: u64, scope: Condition) -> Self {
        assert!(g.index.contains(root), "stale or foreign identity root");
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
            phase: ResolvePhase::Next,
            visits: 0,
        }
    }
    pub fn root(&self) -> Root {
        self.root
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
    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> ResolveStatus {
        match self.phase {
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
                        self.root,
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
}
impl Equal {
    pub fn new(g: &Graph, root: Root, x: u64, y: u64, scope: Condition) -> Self {
        Self {
            root,
            left: Resolved::new(g, root, x, scope),
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
        self.root
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.left
            .roots()
            .chain(self.right.roots())
            .chain([self.carry, self.result])
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
    }
    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> Option<Condition> {
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
    edits: VecDeque<Edit>,
    boolean: Option<Job>,
    phase: MergePhase,
}
impl Merge {
    pub fn new(g: &Graph, root: Root, x: u64, y: u64, scope: Condition) -> Self {
        Self {
            base: root,
            staged: root,
            left: Resolved::new(g, root, x, scope),
            right: Resolved::new(g, root, y, scope),
            l: None,
            r: None,
            scope,
            pair: Condition::FALSE,
            left_rank: None,
            right_rank: None,
            rank: 0,
            changed: Condition::FALSE,
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
        [self.base, self.staged]
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
    pub fn tick(&mut self, g: &mut Graph, a: &mut Arena) -> Option<Root> {
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
                        self.left_rank = Some(Partition::new(g, self.base, RANK, l, 0, c));
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
                    self.right_rank = Some(Partition::new(g, self.base, RANK, r, 0, support));
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
                            self.staged = g.write(self.staged, key, c);
                        } else {
                            self.changed = c;
                        }
                    }
                } else if let Some(edit) = self.edits.front() {
                    let old = edit.key.map_or(self.changed, |key| {
                        g.index.get(self.staged, &key).unwrap_or(Condition::FALSE)
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
            MergePhase::Done => return Some(self.staged),
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
            _ => Step::Done,
        }
    }
}
