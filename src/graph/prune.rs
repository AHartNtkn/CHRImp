//! Semantic liveness of occurrences and the upward identity paths they use.

use super::{Graph, INCIDENCE};
use crate::condition::{Arena, Condition, Job, Operation, poll};
use crate::identity::{CHILD, PARENT, RANK};
use crate::store::{Cursor, Filter, FilterStatus, Key, Root};
use std::collections::{BTreeMap, VecDeque};

// A single weak witness cannot keep index payloads or Boolean nodes alive.
// Exact ordered seeds include support: dropping a body reference can make an
// otherwise unchanged identity path dead. Large seed frontiers use full pruning.
const MAX_SEEDS: usize = 64;
// Four unsigned varints encode every exact key (4..40 bytes). This limits
// cold-mutation traffic as well as the number of keys: at most 64 key events.
const MAX_DIRTY_BYTES: usize = 256;
enum Frontier {
    Encoded(Vec<u8>),
    Decoded(Vec<Key>),
}
impl Frontier {
    fn empty() -> Self {
        Self::Encoded(Vec::new())
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::Encoded(v) => v.is_empty(),
            Self::Decoded(v) => v.is_empty(),
        }
    }
    fn push(&mut self, key: Key) -> bool {
        let Self::Encoded(bytes) = self else {
            unreachable!()
        };
        let mut encoded = [0u8; 40];
        let mut len = 0;
        for mut word in key {
            loop {
                let low = (word & 127) as u8;
                word >>= 7;
                encoded[len] = low | if word == 0 { 0 } else { 128 };
                len += 1;
                if word == 0 {
                    break;
                }
            }
        }
        let needed = bytes.len() + len;
        if needed > MAX_DIRTY_BYTES {
            return false;
        }
        if needed > bytes.capacity() {
            bytes.reserve_exact(needed.next_power_of_two().max(8) - bytes.len());
        }
        bytes.extend_from_slice(&encoded[..len]);
        true
    }
    fn decode(&mut self) {
        let Self::Encoded(bytes) = self else {
            unreachable!()
        };
        let mut rest = bytes.as_slice();
        let mut keys = Vec::with_capacity(bytes.len() / 4);
        while !rest.is_empty() {
            let key = std::array::from_fn(|_| {
                let mut word = 0;
                let mut shift = 0;
                loop {
                    let byte = rest[0];
                    rest = &rest[1..];
                    word |= u64::from(byte & 127) << shift;
                    if byte & 128 == 0 {
                        break;
                    }
                    shift += 7;
                }
                word
            });
            keys.push(key);
        }
        keys.sort_unstable();
        keys.dedup();
        *self = Self::Decoded(keys);
    }
    fn keys(&self) -> &[Key] {
        let Self::Decoded(keys) = self else {
            unreachable!()
        };
        keys
    }
}
pub(super) struct Certificate {
    root: crate::store::WeakRoot,
    active: Condition,
    order: u64,
    arena_owner: u32,
    seeds: VecDeque<(u64, Condition)>,
    identity_free: bool,
    dirty: Frontier,
}
impl Certificate {
    pub(super) fn valid(&self, store: &crate::store::Store<Condition>) -> bool {
        self.root.valid(store)
    }
    pub(super) fn record(&mut self, key: Key, support: Option<Condition>) -> bool {
        if matches!(key[0], PARENT | CHILD | RANK) {
            return false;
        }
        if (key[0] <= INCIDENCE || key[0] == super::constructors::ATTACHMENT)
            && support.is_some_and(|c| c != self.active && self.active != Condition::TRUE)
        {
            return self.dirty.push(key);
        }
        true
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Certificate,
    NoopSeeds,
    FrontierSeeds,
    Delta,
    DeltaHit,
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
    certificate: Option<Certificate>,
    identity_free: bool,
    delta: bool,
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
    /// Consume the old weak witness before mutation, so the certificate itself
    /// does not force a root copy. Only identity-free graphs admit local deltas:
    /// no changed occurrence can invalidate an upward identity dependency.
    pub(super) fn take_liveness_delta(&mut self, root: &Root) -> Option<Certificate> {
        let mut c = self.liveness.take()?;
        if !c.identity_free || !c.root.matches(root) {
            return None;
        }
        c.root = self.empty().downgrade();
        Some(c)
    }
    pub(super) fn finish_liveness_delta(&mut self, mut c: Option<Certificate>, root: &Root) {
        if let Some(c) = &mut c {
            c.root = root.downgrade();
        }
        self.liveness = c;
    }
    pub fn prune(&self, root: Root, active: Condition) -> Prune {
        Prune {
            certificate: None,
            identity_free: false,
            delta: false,
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
                #[cfg(feature = "diagnostics")]
                graph.field_work.update(|mut w| {
                    w.liveness_probes += 1;
                    w
                });
                if graph.liveness.as_ref().is_some_and(|c| {
                    c.root.matches(&self.base)
                        && c.active == self.active
                        && c.order == arena.representation_epoch()
                        && c.arena_owner == arena.owner()
                        && (c.identity_free
                            || (c.seeds.len() == self.seeds.len()
                                && c.seeds.iter().zip(&self.seeds).all(|(a, b)| {
                                    #[cfg(feature = "diagnostics")]
                                    graph.field_work.update(|mut w| {
                                        w.liveness_seed_comparisons += 1;
                                        w
                                    });
                                    a == b
                                })))
                }) {
                    self.filter = None;
                    if graph.liveness.as_ref().unwrap().dirty.is_empty() {
                        #[cfg(feature = "diagnostics")]
                        graph.field_work.update(|mut w| {
                            w.liveness_hits += 1;
                            w
                        });
                        self.phase = Phase::NoopSeeds;
                    } else {
                        self.certificate = graph.liveness.take();
                        self.certificate.as_mut().unwrap().dirty.decode();
                        self.filter = Some(graph.index.filter(self.base.clone()));
                        self.identity_free = true;
                        self.delta = true;
                        self.phase = Phase::FrontierSeeds;
                    }
                    return None;
                }
                graph.invalidate_liveness();
                // An empty namespace interval is certified by the trie bounds,
                // not by enumerating occurrence leaves. At most two 256-bit
                // boundary paths (and their rejected siblings) are visited.
                self.identity_free = graph
                    .index
                    .range(
                        self.base.clone(),
                        [PARENT, 0, 0, 0],
                        [RANK, u64::MAX, u64::MAX, u64::MAX],
                    )
                    .next(&graph.index)
                    .is_none();
                if self.identity_free && self.active == Condition::TRUE {
                    self.filter = None;
                    self.phase = Phase::NoopSeeds;
                } else {
                    self.certificate =
                        (self.identity_free || self.seeds.len() <= MAX_SEEDS).then(|| {
                            Certificate {
                                root: self.base.downgrade(),
                                active: self.active,
                                order: arena.representation_epoch(),
                                arena_owner: arena.owner(),
                                seeds: if self.identity_free {
                                    VecDeque::new()
                                } else {
                                    self.seeds.clone()
                                },
                                identity_free: self.identity_free,
                                dirty: Frontier::empty(),
                            }
                        });
                    #[cfg(feature = "diagnostics")]
                    graph.field_work.update(|mut w| {
                        w.liveness_empty_frontiers += u64::from(self.identity_free);
                        w.liveness_full_frontiers += u64::from(!self.identity_free);
                        w.liveness_seed_copies += self
                            .certificate
                            .as_ref()
                            .map_or(0, |c| c.seeds.len() as u64);
                        w
                    });
                    self.phase = if self.identity_free {
                        Phase::FrontierSeeds
                    } else {
                        Phase::Seeds
                    };
                }
            }
            Phase::FrontierSeeds => {
                if let Some((_, support)) = self.seeds.pop_front() {
                    assert!(arena.contains(support), "stale or foreign prune seed");
                } else {
                    self.seeds = VecDeque::new();
                    self.phase = if self.delta {
                        Phase::Delta
                    } else {
                        Phase::Occurrences
                    };
                }
            }
            Phase::Delta => {
                #[cfg(feature = "diagnostics")]
                graph.field_work.update(|mut w| {
                    w.liveness_dirty_filter_ticks += 1;
                    w
                });
                let dirty = self.certificate.as_ref().unwrap().dirty.keys();
                match self
                    .filter
                    .as_mut()
                    .unwrap()
                    .tick_keys(&mut graph.index, dirty)
                {
                    FilterStatus::Pending => {}
                    FilterStatus::Leaf { key, value } if dirty.binary_search(&key).is_ok() => {
                        #[cfg(feature = "diagnostics")]
                        graph.field_work.update(|mut w| {
                            w.liveness_dirty_visits += 1;
                            w
                        });
                        self.key = key;
                        self.boolean = Some(arena.start(Operation::And(value, self.active)));
                        self.phase = Phase::DeltaHit;
                    }
                    FilterStatus::Leaf { value, .. } => self.replace(value),
                    FilterStatus::Complete(root) => {
                        self.staged = root;
                        self.filter = None;
                        self.certificate.as_mut().unwrap().dirty = Frontier::empty();
                        self.phase = Phase::Cleanup;
                    }
                }
            }
            Phase::DeltaHit => {
                if let Some(c) = poll(&mut self.boolean, arena) {
                    self.replace(c);
                    self.phase = Phase::Delta;
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
                if let Some(c) = poll(&mut self.boolean, arena) {
                    self.enqueue(arena, self.variable, c, Phase::Seeds);
                }
            }
            Phase::Occurrences => match self.filter.as_mut().unwrap().tick(&mut graph.index) {
                FilterStatus::Pending => {}
                FilterStatus::Leaf { key, value } => {
                    if key[0] <= INCIDENCE || key[0] == super::constructors::ATTACHMENT {
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
                    self.phase = if self.identity_free {
                        Phase::Cleanup
                    } else {
                        Phase::Next
                    };
                }
            },
            Phase::OccurrenceHit => {
                if let Some(c) = poll(&mut self.boolean, arena) {
                    self.replace(c);
                    if self.key[0] == INCIDENCE && !self.identity_free {
                        self.enqueue(arena, self.key[1], c, Phase::Occurrences);
                    } else {
                        self.phase = Phase::Occurrences;
                    }
                }
            }
            Phase::Enqueue => {
                if let Some(c) = poll(&mut self.boolean, arena) {
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
                if let Some(c) = poll(&mut self.boolean, arena) {
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
                if let Some(c) = poll(&mut self.boolean, arena) {
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
                if let Some(c) = poll(&mut self.boolean, arena) {
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
                if let Some(c) = poll(&mut self.boolean, arena) {
                    self.boolean = Some(arena.start(Operation::And(c, self.active)));
                    self.phase = Phase::IdentityActive;
                }
            }
            Phase::IdentityActive => {
                if let Some(c) = poll(&mut self.boolean, arena) {
                    self.replace(c);
                    self.phase = Phase::Identity;
                }
            }
            Phase::Cleanup => {
                if self.marked.pop_first().is_none() {
                    self.base = self.staged.clone();
                    if let Some(mut c) = self.certificate.take() {
                        c.root = self.staged.downgrade();
                        // A reorder during a public prune invalidates reuse.
                        if c.order == arena.representation_epoch() && !self.staged.is_empty() {
                            graph.liveness = Some(c);
                        }
                    }
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
    fn dirty_journal_preserves_full_width_keys_and_bounds_cold_growth() {
        let keys = [
            [0; 4],
            [u64::MAX; 4],
            [128, 127, 1 << 63, 1 << 32],
            [7, 0, 0, u64::MAX],
        ];
        let mut f = Frontier::empty();
        for key in keys {
            assert!(f.push(key));
        }
        assert!(f.push(keys[0]));
        f.decode();
        let mut expected = keys;
        expected.sort_unstable();
        assert_eq!(f.keys(), expected);
        let mut f = Frontier::empty();
        for _ in 0..6 {
            assert!(f.push([u64::MAX; 4]));
        }
        assert!(!f.push([u64::MAX; 4]));
        let Frontier::Encoded(bytes) = &f else {
            unreachable!()
        };
        assert!(bytes.capacity() <= MAX_DIRTY_BYTES);
        f.decode();
        assert_eq!(f.keys(), [[u64::MAX; 4]]);
    }
    fn finish(g: &mut Graph, a: &mut Arena, mut p: Prune) -> (Root, usize) {
        for steps in 1..100_000 {
            if let Some(root) = p.tick(g, a) {
                return (root, steps);
            }
        }
        panic!("prune did not finish");
    }

    #[test]
    fn repeated_liveness_reuses_work_but_seed_loss_reclaims_identity() {
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, active) = a.fresh_choice();
        let root = g.write(g.empty(), [PARENT, 17, 29, 0], active);
        let root = g.write(root, [CHILD, 29, 17, 0], active);
        let mut p = g.prune(root, active);
        p.seed(17, active);
        let (root, first) = finish(&mut g, &mut a, p);
        assert_eq!(g.index.get(&root, &[PARENT, 17, 29, 0]), Some(active));
        let mut p = g.prune(root.clone(), active);
        p.seed(17, active);
        let (same, repeated) = finish(&mut g, &mut a, p);
        assert_eq!(same, root);
        assert!(repeated * 4 < first, "{first} -> {repeated}");
        let p = g.prune(root, active);
        let (empty, _) = finish(&mut g, &mut a, p);
        assert_eq!(empty, g.empty());
    }

    #[test]
    fn universal_seed_cache_does_not_bypass_foreign_arena_rejection() {
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, condition) = a.fresh_choice();
        let root = g.write(g.empty(), [PARENT, 17, 29, 0], condition);
        let mut p = g.prune(root, Condition::TRUE);
        p.seed(17, Condition::TRUE);
        let (root, _) = finish(&mut g, &mut a, p);
        let mut foreign = Arena::default();
        let mut p = g.prune(root, Condition::TRUE);
        p.seed(17, Condition::TRUE);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                finish(&mut g, &mut foreign, p)
            }))
            .is_err()
        );
    }

    #[test]
    fn liveness_witness_releases_after_untraced_root_collection() {
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, active) = a.fresh_choice();
        let root = g.write(g.empty(), [PARENT, 17, 29, 0], active);
        let mut p = g.prune(root, active);
        p.seed(17, active);
        let (root, _) = finish(&mut g, &mut a, p);
        assert!(g.liveness.is_some());
        // Even an untraced strong caller handle loses authority at collection.
        let mut gc = g.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut g);
        }
        drop(gc);
        assert!(g.liveness.is_none());
        drop(root);
        for _ in 0..1000 {
            if g.release_tick() {
                break;
            }
        }
        assert_eq!(g.index.node_count(), 0);
    }

    #[test]
    fn conditional_identity_free_frontier_does_not_mark_variables() {
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, active) = a.fresh_choice();
        let mut root = g.empty();
        for i in 0..128 {
            root = g.write(root, [INCIDENCE, i, i, 0], Condition::TRUE);
        }
        let mut p = g.prune(root, active);
        let root = loop {
            let result = p.tick(&mut g, &mut a);
            assert!(
                p.marked.is_empty(),
                "identity-free graph has no upward paths"
            );
            if let Some(root) = result {
                break root;
            }
        };
        for i in 0..128 {
            assert_eq!(g.index.get(&root, &[INCIDENCE, i, i, 0]), Some(active));
        }
        let p = g.prune(root, active.not());
        assert_eq!(finish(&mut g, &mut a, p).0, g.empty());
    }

    #[test]
    fn sparse_mutation_prunes_only_dirty_frontier_and_identity_write_invalidates() {
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, active) = a.fresh_choice();
        let mut root = g.empty();
        for i in 0..128 {
            root = g.write(root, [INCIDENCE, i, i, 0], active);
        }
        let p = g.prune(root, active);
        let (root, _) = finish(&mut g, &mut a, p);
        let root = g.write(root, [INCIDENCE, 200, 200, 0], Condition::TRUE);
        let p = g.prune(root, active);
        let (root, steps) = finish(&mut g, &mut a, p);
        assert!(steps < 128, "sparse delta scanned unchanged graph: {steps}");
        assert_eq!(g.index.get(&root, &[INCIDENCE, 200, 200, 0]), Some(active));
        // A newly added upward edge requires the full dependency traversal.
        let root = g.write(root, [PARENT, 200, 201, 0], active);
        let p = g.prune(root, active);
        let (root, _) = finish(&mut g, &mut a, p);
        assert_eq!(g.index.get(&root, &[PARENT, 200, 201, 0]), Some(active));
    }
    #[test]
    fn failed_support_cannot_retain_a_constructor_attachment() {
        use crate::graph::constructors::ATTACHMENT;
        let mut g = Graph::new(&[]);
        let mut a = Arena::default();
        let (_, choice) = a.fresh_choice();
        let key = [ATTACHMENT, 17, 0, 23];
        let root = g.write(g.empty(), key, choice);
        let mut prune = g.prune(root, choice.not());
        let result = loop {
            if let Some(r) = prune.tick(&mut g, &mut a) {
                break r;
            }
        };
        assert!(g.index.get(&result, &key).is_none());
    }
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
