//! Semantic liveness of occurrences and the upward identity paths they use.

use super::{Graph, INCIDENCE};
use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::identity::{CHILD, PARENT, RANK};
use crate::store::{Cursor, Filter, FilterStatus, Key, Root};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy)]
enum Phase {
    Certificate,
    NoopSeeds,
    Seeds,
    SeedActive,
    Occurrences,
    OccurrenceHit,
    Enqueue,
    Next,
    Novel,
    Remember,
    Scan,
    Hit,
    Identity,
    IdentityMark,
    IdentityActive,
    Cleanup,
    Done,
}

/// Owned semantic pruning against an immutable graph snapshot. Seed all explicit
/// query/body references before ticking. Physical collectors must retain both
/// root iterators, including while a condition operation or filter is suspended.
pub struct Prune {
    base: Root,
    staged: Root,
    active: Condition,
    started: bool,
    seeds: VecDeque<(u64, Condition)>,
    pending: BTreeMap<u64, Condition>,
    marked: BTreeMap<u64, Condition>,
    filter: Option<Filter<Condition>>,
    cursor: Option<Cursor>,
    boolean: Option<Job>,
    // Completed empty/terminal continuations retain owner identity and enforce
    // the existing GC leases without exposing owner internals across modules.
    graph_guard: Filter<Condition>,
    arena_guard: Option<Job>,
    variable: u64,
    target: u64,
    key: Key,
    fresh: Condition,
    phase: Phase,
    after_enqueue: Phase,
}

impl Graph {
    pub fn prune(&self, root: Root, active: Condition) -> Prune {
        Prune {
            base: root.clone(),
            staged: root.clone(),
            active,
            started: false,
            seeds: VecDeque::new(),
            pending: BTreeMap::new(),
            marked: BTreeMap::new(),
            filter: Some(self.index.filter(root)),
            cursor: None,
            boolean: None,
            graph_guard: self.index.filter(self.empty()),
            arena_guard: None,
            variable: 0,
            target: 0,
            key: [0; 4],
            fresh: Condition::FALSE,
            phase: Phase::Certificate,
            after_enqueue: Phase::Seeds,
        }
    }
}

impl Prune {
    /// Each call adds one scoped external reference; duplicates are unioned
    /// resumably during execution rather than requiring an arena at seed time.
    pub fn seed(&mut self, variable: u64, support: Condition) {
        assert!(!self.started, "prune seeds must precede the first tick");
        self.seeds.push_back((variable, support));
    }

    pub fn graph_roots(&self) -> impl Iterator<Item = Root> + '_ {
        [self.base.clone(), self.staged.clone()]
            .into_iter()
            .chain(self.filter.iter().flat_map(Filter::roots))
            .chain(self.cursor.iter().map(Cursor::root))
    }

    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.active, self.fresh]
            .into_iter()
            .chain(self.seeds.iter().map(|(_, c)| *c))
            .chain(self.pending.values().copied())
            .chain(self.marked.values().copied())
            .chain(self.filter.iter().flat_map(Filter::values))
            .chain(self.boolean.iter().flat_map(Job::roots))
            .chain(self.arena_guard.iter().flat_map(Job::roots))
    }

    fn poll(&mut self, arena: &mut Arena) -> Option<Condition> {
        match self
            .boolean
            .as_mut()
            .expect("prune condition job")
            .tick(arena)
        {
            Progress::Pending => None,
            Progress::Complete(c) => {
                self.boolean = None;
                Some(c)
            }
        }
    }

    fn enqueue(&mut self, arena: &Arena, variable: u64, support: Condition, after: Phase) {
        self.target = variable;
        self.after_enqueue = after;
        if support == Condition::FALSE {
            self.phase = after;
        } else {
            let old = self
                .pending
                .get(&variable)
                .copied()
                .unwrap_or(Condition::FALSE);
            self.boolean = Some(arena.start(Operation::Or(old, support)));
            self.phase = Phase::Enqueue;
        }
    }

    fn replace(&mut self, support: Condition) {
        self.filter
            .as_mut()
            .expect("prune filter")
            .replace((support != Condition::FALSE).then_some(support));
    }

    /// One leaf/traversal transition, BDD step, or map cleanup entry per call.
    /// Index/cursor/map operations have their ordinary bounded-key or logarithmic
    /// costs. Old snapshots remain immutable; only the returned root is complete.
    pub fn tick(&mut self, graph: &mut Graph, arena: &mut Arena) -> Option<Root> {
        self.graph_guard.tick(&mut graph.index);
        if let Some(guard) = self.arena_guard.as_mut() {
            guard.tick(arena);
        } else {
            assert!(
                arena.contains(self.active),
                "stale or foreign prune support"
            );
            let mut guard = arena.start(Operation::And(Condition::TRUE, Condition::TRUE));
            guard.tick(arena);
            self.arena_guard = Some(guard);
        }
        assert!(graph.index.contains(&self.base), "stale prune graph root");
        self.started = true;
        match self.phase {
            Phase::Certificate => {
                // An empty namespace interval is certified by the trie bounds,
                // not by enumerating occurrence leaves. At most two 256-bit
                // boundary paths (and their rejected siblings) are visited.
                let noop = self.active == Condition::TRUE
                    && graph
                        .index
                        .range(
                            self.base.clone(),
                            [PARENT, 0, 0, 0],
                            [RANK, u64::MAX, u64::MAX, u64::MAX],
                        )
                        .next(&graph.index)
                        .is_none();
                if noop {
                    self.filter = None;
                    self.phase = Phase::NoopSeeds;
                } else {
                    self.phase = Phase::Seeds;
                }
            }
            Phase::NoopSeeds => {
                if let Some(&(_, support)) = self.seeds.front() {
                    assert!(arena.contains(support), "stale or foreign prune seed");
                    self.seeds.pop_front();
                } else {
                    self.seeds = VecDeque::new();
                    self.active = Condition::FALSE;
                    self.phase = Phase::Done;
                }
            }
            Phase::Seeds => {
                if let Some((variable, support)) = self.seeds.front() {
                    let (variable, support) = (*variable, *support);
                    self.boolean = Some(arena.start(Operation::And(support, self.active)));
                    self.seeds.pop_front();
                    self.variable = variable;
                    self.phase = Phase::SeedActive;
                } else {
                    self.seeds = VecDeque::new();
                    self.phase = Phase::Occurrences;
                }
            }
            Phase::SeedActive => {
                if let Some(c) = self.poll(arena) {
                    self.enqueue(arena, self.variable, c, Phase::Seeds);
                }
            }
            Phase::Occurrences => match self.filter.as_mut().unwrap().tick(&mut graph.index) {
                FilterStatus::Pending => {}
                FilterStatus::Leaf { key, value } => {
                    if key[0] <= INCIDENCE {
                        self.key = key;
                        self.boolean = Some(arena.start(Operation::And(value, self.active)));
                        self.phase = Phase::OccurrenceHit;
                    } else {
                        self.replace(value);
                    }
                }
                FilterStatus::Complete(root) => {
                    self.staged = root;
                    self.filter = None;
                    self.phase = Phase::Next;
                }
            },
            Phase::OccurrenceHit => {
                if let Some(c) = self.poll(arena) {
                    self.replace(c);
                    if self.key[0] == INCIDENCE {
                        self.enqueue(arena, self.key[1], c, Phase::Occurrences);
                    } else {
                        self.phase = Phase::Occurrences;
                    }
                }
            }
            Phase::Enqueue => {
                if let Some(c) = self.poll(arena) {
                    self.pending.insert(self.target, c);
                    self.phase = self.after_enqueue;
                }
            }
            Phase::Next => {
                if let Some((&variable, &support)) = self.pending.first_key_value() {
                    let old = self
                        .marked
                        .get(&variable)
                        .copied()
                        .unwrap_or(Condition::FALSE);
                    self.boolean = Some(arena.start(Operation::Difference(support, old)));
                    self.pending.pop_first();
                    self.variable = variable;
                    self.phase = Phase::Novel;
                } else {
                    self.filter = Some(graph.index.filter(self.staged.clone()));
                    self.phase = Phase::Identity;
                }
            }
            Phase::Novel => {
                if let Some(c) = self.poll(arena) {
                    self.fresh = c;
                    if c == Condition::FALSE {
                        self.phase = Phase::Next;
                    } else {
                        let old = self
                            .marked
                            .get(&self.variable)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(arena.start(Operation::Or(old, c)));
                        self.phase = Phase::Remember;
                    }
                }
            }
            Phase::Remember => {
                if let Some(c) = self.poll(arena) {
                    self.marked.insert(self.variable, c);
                    self.cursor = Some(graph.index.range(
                        self.staged.clone(),
                        [PARENT, self.variable, 0, 0],
                        [PARENT, self.variable, u64::MAX, 0],
                    ));
                    self.phase = Phase::Scan;
                }
            }
            Phase::Scan => {
                if let Some((key, support)) = self.cursor.as_mut().unwrap().next(&graph.index) {
                    self.target = key[2];
                    self.boolean = Some(arena.start(Operation::And(self.fresh, support)));
                    self.phase = Phase::Hit;
                } else {
                    self.cursor = None;
                    self.phase = Phase::Next;
                }
            }
            Phase::Hit => {
                if let Some(c) = self.poll(arena) {
                    self.enqueue(arena, self.target, c, Phase::Scan);
                }
            }
            Phase::Identity => match self.filter.as_mut().unwrap().tick(&mut graph.index) {
                FilterStatus::Pending => {}
                FilterStatus::Leaf { key, value } => {
                    if matches!(key[0], PARENT | CHILD | RANK) {
                        let variable = if key[0] == CHILD { key[2] } else { key[1] };
                        let marked = self
                            .marked
                            .get(&variable)
                            .copied()
                            .unwrap_or(Condition::FALSE);
                        self.boolean = Some(arena.start(Operation::And(value, marked)));
                        self.phase = Phase::IdentityMark;
                    } else {
                        self.replace(value);
                    }
                }
                FilterStatus::Complete(root) => {
                    self.staged = root.clone();
                    self.base = root;
                    self.filter = None;
                    self.phase = Phase::Cleanup;
                }
            },
            Phase::IdentityMark => {
                if let Some(c) = self.poll(arena) {
                    self.boolean = Some(arena.start(Operation::And(c, self.active)));
                    self.phase = Phase::IdentityActive;
                }
            }
            Phase::IdentityActive => {
                if let Some(c) = self.poll(arena) {
                    self.replace(c);
                    self.phase = Phase::Identity;
                }
            }
            Phase::Cleanup => {
                if self.marked.pop_first().is_none() {
                    debug_assert!(self.seeds.is_empty() && self.pending.is_empty());
                    self.active = Condition::FALSE;
                    self.fresh = Condition::FALSE;
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => return Some(self.staged.clone()),
        }
        None
    }
}

#[cfg(test)]
mod certificate_tests {
    use super::*;
    #[test]
    fn certificate_probe_uses_bounded_key_paths() {
        let mut g = Graph::new(&[]);
        let mut root = g.empty();
        for i in 0..4096 {
            root = g.index.insert(
                root,
                [if i % 2 == 0 { 3 } else { 7 }, i, 0, 0],
                Condition::TRUE,
            );
        }
        let mut probe = g.index.range(
            root.clone(),
            [PARENT, 0, 0, 0],
            [RANK, u64::MAX, u64::MAX, u64::MAX],
        );
        assert!(probe.next(&g.index).is_none());
        assert!(probe.visits() <= 1025);
        let root = g
            .index
            .insert(root, [RANK, u64::MAX, u64::MAX, u64::MAX], Condition::TRUE);
        let mut probe = g.index.range(
            root,
            [PARENT, 0, 0, 0],
            [RANK, u64::MAX, u64::MAX, u64::MAX],
        );
        assert!(probe.next(&g.index).is_some());
        assert!(probe.visits() <= 1025);
    }
    #[test]
    fn every_identity_namespace_defeats_certificate() {
        for namespace in [PARENT, CHILD, RANK] {
            let mut g = Graph::new(&[]);
            let mut a = Arena::default();
            let root = g
                .index
                .insert(g.empty(), [namespace, 17, 29, 0], Condition::TRUE);
            let mut job = g.prune(root, Condition::TRUE);
            let mut result = None;
            for _ in 0..1000 {
                if let Some(r) = job.tick(&mut g, &mut a) {
                    result = Some(r);
                    break;
                }
            }
            assert_eq!(
                result,
                Some(g.empty()),
                "namespace {namespace} needs pruning"
            );
        }
    }
    #[test]
    fn certificate_seed_checks_freeze_and_owner_remain_enforced() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let mut foreign = Arena::default();
        let (_, bad) = foreign.fresh_choice();
        let mut job = g.prune(g.empty(), Condition::TRUE);
        job.seed(0, bad);
        assert_eq!(job.tick(&mut g, &mut a), None);
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut g, &mut a))).is_err());
        let mut job = g.prune(g.empty(), Condition::TRUE);
        let gc = g.collect(std::iter::empty());
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut g, &mut a))).is_err());
        drop(gc);
        let gc = a.collect(std::iter::empty());
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut g, &mut a))).is_err());
        drop(gc);
        let mut other = Graph::new(&[]);
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut other, &mut a))).is_err());
        for _ in 0..4 {
            job.tick(&mut g, &mut a);
        }
        assert!(catch_unwind(AssertUnwindSafe(|| job.seed(0, Condition::TRUE))).is_err());
    }
}
