//! Experimental normalization transactions. Source execution defaults to Baseline.
use super::*;
use crate::graph::constructors::{Attach, AttachmentStatus, Transfer};
use crate::identity::{Resolve, ResolveStatus, UnionLink};
use crate::program::constructors::Constructors;
use crate::trace::{Cursor as TraceCursor, Step, Trace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizationMode {
    Baseline,
    Priority,
    Direct,
    /// Direct normalization plus selection of original constructor-led arms.
    Dispatch,
}
#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct NormalizationStats {
    /// Symbolic classification starts, not concrete histories or CPU work.
    pub conditional_dispatches: u64,
    pub known_arm_admissions: u64,
    pub generative_dispatches: u64,
    pub transactions: u64,
    pub steps: u64,
    pub attachment_overlaps: u64,
    pub field_equalities: u64,
    pub coalescences: u64,
    pub clashes: u64,
    pub generic_matching_ticks: u64,
    pub generic_candidate_visits: u64,
    pub generic_commit_ticks: u64,
    pub applications: u64,
}
impl NormalizationStats {
    fn add(&mut self, s: &Self) {
        self.conditional_dispatches += s.conditional_dispatches;
        self.known_arm_admissions += s.known_arm_admissions;
        self.generative_dispatches += s.generative_dispatches;
        self.transactions += s.transactions;
        self.steps += s.steps;
        self.attachment_overlaps += s.attachment_overlaps;
        self.field_equalities += s.field_equalities;
        self.coalescences += s.coalescences;
        self.clashes += s.clashes;
        self.generic_matching_ticks += s.generic_matching_ticks;
        self.generic_candidate_visits += s.generic_candidate_visits;
        self.generic_commit_ticks += s.generic_commit_ticks;
        self.applications += s.applications;
    }
}
#[derive(Clone)]
pub(super) struct Configuration {
    pub mode: NormalizationMode,
    pub plan: Arc<Constructors>,
    pub source: Arc<Prepared>,
}
impl Engine {
    /// Experimental source execution, output and cancellation. Historical per-rule
    /// stepping through fused normalization transactions is not supported.
    /// Non-baseline modes admit only the whole-program subsystem checked by
    /// `Constructors::recognize`; Baseline uses ordinary execution.
    pub fn with_normalization(
        code: Arc<Prepared>,
        mode: NormalizationMode,
    ) -> Result<Self, String> {
        if mode == NormalizationMode::Baseline {
            return Ok(Self::new(code));
        }
        let plan = Arc::new(Constructors::recognize(&code)?);
        let consumer = Arc::new(plan.consumer_code(&code));
        let mut e = Self::new(consumer);
        e.normalization = Some(Configuration {
            mode,
            plan,
            source: code,
        });
        Ok(e)
    }
    /// Includes work in an unfinished transaction; these are counts, not CPU shares.
    pub fn normalization_stats(&self) -> NormalizationStats {
        let mut stats = self.normalization_stats.clone();
        for s in self.queue.iter().chain(self.parked.values()) {
            if let Task::Body(b) = &s.task {
                if let Some(n) = &b.normalizer {
                    stats.add(&n.stats);
                }
            }
        }
        stats
    }
    pub(super) fn normalization_tick(&mut self, id: u64, b: &mut Body) -> bool {
        let n = b.normalizer.as_mut().unwrap();
        if !n.done {
            n.tick(
                &mut self.graph,
                &mut self.arena,
                &mut self.history,
                &mut self.ids,
            );
            return false;
        }
        // Keep the owning obligation until every resulting activation has been admitted.
        self.state.graph = n.root.clone();
        self.semantic_regions |= self.active != n.active;
        self.active = n.active;
        if let Some((root, occ, scope, identity_only)) = n.activations.pop_front() {
            let (root, scope) = if identity_only {
                (root, scope)
            } else {
                let root = self.state.graph.clone();
                let support = self
                    .graph
                    .fact(root.clone(), occ)
                    .map_or(Condition::FALSE, |f| f.support);
                (root, support)
            };
            if scope != Condition::FALSE && self.graph.fact(root.clone(), occ).is_some() {
                self.spawn(scope, self.activation(root, occ, scope, identity_only));
            }
            return false;
        }
        self.applications += n.stats.applications;
        self.normalization_stats.add(&n.stats);
        b.normalizer = None;
        self.finish_body_record(id);
        true
    }
}

enum Work {
    Links(VecDeque<UnionLink>),
    Fields(Arc<Vec<u64>>, Arc<Vec<u64>>, Condition, usize),
    Body(usize, Arc<Vec<u64>>, Condition, usize),
    Resolve(u64, Condition, Option<Resolve>),
    Attach(u64, u64, Condition, Option<Attach>),
    Transfer(u64, u64, Condition, Option<Transfer>),
    Equal(u64, u64, Condition, Option<Merge>),
    Wake(Box<Wake>),
    Anchor(Root, u64, Condition, usize),
    Search {
        rule: usize,
        matches: Option<Box<Matches>>,
        commit: Option<Box<Commit>>,
    },
    Consume(u64, Condition, Option<Job>, Option<Update>),
    Fail(Condition, Option<Job>),
}
pub(super) struct Normalizer {
    config: Configuration,
    base: Root,
    pub root: Root,
    pub active: Condition,
    work: VecDeque<Work>,
    gate: Option<Job>,
    pub activations: VecDeque<(Root, u64, Condition, bool)>,
    pub stats: NormalizationStats,
    pub done: bool,
}
impl Normalizer {
    pub fn post(
        config: Configuration,
        g: &Graph,
        root: Root,
        active: Condition,
        id: u64,
        scope: Condition,
    ) -> Self {
        let mut n = Self::new(config, root.clone(), active);
        // The final live support is checked again when this activation is published.
        n.activations.push_back((root.clone(), id, scope, false));
        if n.config
            .plan
            .relations
            .contains(&g.fact(root.clone(), id).unwrap().relation)
        {
            n.work.push_back(if n.direct() {
                Work::Resolve(id, scope, None)
            } else {
                Work::Anchor(root, id, scope, 0)
            });
        }
        n
    }
    pub fn merged(
        config: Configuration,
        g: &Graph,
        root: Root,
        active: Condition,
        merge: &mut Merge,
    ) -> Self {
        let mut n = Self::new(config, root, active);
        n.merged_work(g, merge);
        n
    }
    fn new(config: Configuration, root: Root, active: Condition) -> Self {
        Self {
            config,
            base: root.clone(),
            root,
            active,
            work: VecDeque::new(),
            gate: None,
            activations: VecDeque::new(),
            stats: NormalizationStats {
                transactions: 1,
                ..Default::default()
            },
            done: false,
        }
    }
    fn direct(&self) -> bool {
        matches!(
            self.config.mode,
            NormalizationMode::Direct | NormalizationMode::Dispatch
        )
    }
    fn merged_work(&mut self, g: &Graph, merge: &mut Merge) {
        // Root-paired wakes also feed the generic priority control. They are never rebased.
        if let Some(delta) = merge.take_delta() {
            self.work
                .push_front(Work::Wake(Box::new(Wake::from_delta(g, delta))));
        }
        let links = merge.take_links();
        if !links.is_empty() {
            self.work.push_front(Work::Links(links));
        }
    }
    pub fn tick(&mut self, g: &mut Graph, a: &mut Arena, h: &mut History, ids: &mut FreshIds) {
        self.stats.steps += 1;
        if let Some(scope) = self.work.front_mut().and_then(Work::admission_scope) {
            let support = if self.gate.is_some() {
                poll(&mut self.gate, a)
            } else {
                match a.direct(Operation::And(*scope, self.active)) {
                    Some(c) => Some(c),
                    None => {
                        self.gate = Some(a.start(Operation::And(*scope, self.active)));
                        None
                    }
                }
            };
            let Some(support) = support else {
                return;
            };
            *scope = support;
            if support == Condition::FALSE {
                self.work.pop_front();
                return;
            }
        }
        if let Some(mut work) = self.work.pop_front() {
            if !self.step(&mut work, g, a, h, ids) {
                self.work.push_front(work);
            }
        } else {
            self.done = true;
        }
    }
    fn step(
        &mut self,
        w: &mut Work,
        g: &mut Graph,
        a: &mut Arena,
        h: &mut History,
        ids: &mut FreshIds,
    ) -> bool {
        match w {
            Work::Fields(left, right, scope, next) => {
                if *next == left.len() {
                    return true;
                }
                self.work
                    .push_front(Work::Fields(left.clone(), right.clone(), *scope, *next + 1));
                self.work
                    .push_front(Work::Equal(left[*next], right[*next], *scope, None));
                self.stats.field_equalities += 1;
                true
            }
            Work::Body(i, vars, scope, next) => {
                match &self.config.source.instructions[*i] {
                    Instruction::True => {}
                    Instruction::Fail => self.work.push_front(Work::Fail(*scope, None)),
                    Instruction::Equal(x, y) => {
                        self.stats.field_equalities += 1;
                        self.work
                            .push_front(Work::Equal(vars[*x], vars[*y], *scope, None));
                    }
                    Instruction::And(items) => {
                        if let Some(&child) = items.get(*next) {
                            self.work
                                .push_front(Work::Body(*i, vars.clone(), *scope, *next + 1));
                            self.work
                                .push_front(Work::Body(child, vars.clone(), *scope, 0));
                        }
                    }
                    _ => unreachable!("recognized normalization body"),
                }
                true
            }
            Work::Links(links) => {
                if let Some(UnionLink {
                    winner,
                    loser,
                    support,
                }) = links.pop_front()
                {
                    self.work
                        .push_front(Work::Transfer(winner, loser, support, None));
                    false
                } else {
                    true
                }
            }
            Work::Resolve(id, scope, job) => {
                if job.is_none() {
                    let v = g.arguments(*id)[0];
                    *job = Some(Resolve::new(g, self.root.clone(), v, *scope));
                    return false;
                }
                match job.as_mut().unwrap().tick(g, a) {
                    ResolveStatus::Pending => false,
                    ResolveStatus::Done => true,
                    ResolveStatus::Found { variable, support } => {
                        self.work
                            .push_front(Work::Attach(variable, *id, support, None));
                        false
                    }
                }
            }
            Work::Attach(rep, id, scope, job) => {
                if job.is_none() {
                    *job = Some(Attach::new(g, self.root.clone(), *rep, *id, *scope));
                    return false;
                }
                match job.as_mut().unwrap().tick(g, a, &mut self.root) {
                    AttachmentStatus::Pending => false,
                    AttachmentStatus::Done => true,
                    AttachmentStatus::Overlap(other, hit) => {
                        self.stats.attachment_overlaps += 1;
                        let x = g.fact(self.root.clone(), other).expect("attached survivor");
                        let y = g
                            .fact(self.root.clone(), *id)
                            .expect("incoming constructor");
                        if x.relation != y.relation {
                            self.work.push_front(Work::Fail(hit, None));
                        } else {
                            self.work.push_front(Work::Fields(
                                g.arguments(other),
                                g.arguments(*id),
                                hit,
                                1,
                            ));
                            self.work.push_front(Work::Consume(*id, hit, None, None));
                        }
                        false
                    }
                }
            }
            Work::Transfer(win, lose, scope, job) => {
                if job.is_none() {
                    *job = Some(Transfer::new(g, self.root.clone(), *lose, *scope));
                    return false;
                }
                match job.as_mut().unwrap().tick(g, a, &mut self.root) {
                    AttachmentStatus::Pending => false,
                    AttachmentStatus::Done => true,
                    AttachmentStatus::Overlap(id, hit) => {
                        self.work.push_front(Work::Attach(*win, id, hit, None));
                        false
                    }
                }
            }
            Work::Equal(x, y, scope, job) => {
                if job.is_none() {
                    let merge = Merge::new(g, self.root.clone(), *x, *y, *scope);
                    *job = Some(if self.direct() {
                        merge.with_links()
                    } else {
                        merge
                    });
                    return false;
                }
                let merge = job.as_mut().unwrap();
                if let Some(root) = merge.tick(g, a) {
                    self.root = root;
                    self.merged_work(g, merge);
                    true
                } else {
                    false
                }
            }
            Work::Wake(wake) => match wake.tick(g, a) {
                WakeStatus::Pending => false,
                WakeStatus::Done => true,
                WakeStatus::Found {
                    occurrence,
                    support,
                } => {
                    self.activations
                        .push_back((wake.root(), occurrence, support, true));
                    if !self.direct() {
                        self.work
                            .push_back(Work::Anchor(wake.root(), occurrence, support, 0));
                    }
                    false
                }
            },
            Work::Anchor(root, id, scope, next) => {
                let Some(fact) = g.fact(root.clone(), *id) else {
                    return true;
                };
                let triggers = &self.config.source.triggers[fact.relation];
                if *next == triggers.len() {
                    return true;
                }
                let (rule, head) = triggers[*next];
                *next += 1;
                if self.config.plan.rules.contains(&rule) {
                    let matches = Matches::new(
                        g,
                        root.clone(),
                        self.config.source.clone(),
                        rule,
                        *scope,
                        Some((head, *id)),
                    )
                    .unwrap();
                    self.work.push_back(Work::Search {
                        rule,
                        matches: Some(Box::new(matches)),
                        commit: None,
                    });
                }
                false
            }
            Work::Search {
                rule,
                matches,
                commit,
            } => {
                if let Some(c) = commit {
                    self.stats.generic_commit_ticks += 1;
                    match c.tick(g, a, h, ids) {
                        CommitStatus::Pending => false,
                        CommitStatus::Rejected => {
                            *commit = None;
                            false
                        }
                        CommitStatus::Applied(c) => {
                            self.root = c.state.graph;
                            self.stats.applications += 1;
                            if self.config.source.rules[*rule].kept == 1 {
                                self.stats.coalescences += 1;
                            }
                            self.work.push_back(Work::Search {
                                rule: *rule,
                                matches: matches.take(),
                                commit: None,
                            });
                            self.work.push_front(Work::Body(
                                c.application.body,
                                c.application.variables,
                                c.application.support,
                                0,
                            ));
                            true
                        }
                        CommitStatus::Done => unreachable!(),
                    }
                } else {
                    let m = matches.as_mut().unwrap();
                    let before = m.candidate_visits();
                    let status = m.tick(g, a);
                    self.stats.generic_matching_ticks += 1;
                    self.stats.generic_candidate_visits += m.candidate_visits() - before;
                    match status {
                        MatchStatus::Pending => false,
                        MatchStatus::Done => true,
                        MatchStatus::Found(candidate) => {
                            *commit = Some(Box::new(
                                Commit::new(
                                    g,
                                    h,
                                    StateRoot {
                                        graph: self.root.clone(),
                                        history: h.empty(),
                                    },
                                    self.config.source.clone(),
                                    *rule,
                                    candidate,
                                    self.active,
                                )
                                .unwrap(),
                            ));
                            false
                        }
                    }
                }
            }
            Work::Consume(id, scope, boolean, update) => {
                if let Some(u) = update {
                    if let UpdateStatus::Complete(root) = u.tick(g) {
                        self.root = root;
                        self.stats.coalescences += 1;
                        self.stats.applications += 1;
                        return true;
                    }
                    return false;
                }
                if boolean.is_none() {
                    let old = g
                        .fact(self.root.clone(), *id)
                        .map_or(Condition::FALSE, |f| f.support);
                    *boolean = Some(a.start(Operation::Difference(old, *scope)));
                    return false;
                }
                if let Some(c) = poll(boolean, a) {
                    if g.fact(self.root.clone(), *id).is_none() {
                        return true;
                    }
                    *update = Some(g.set_liveness(self.root.clone(), *id, c).unwrap());
                }
                false
            }
            Work::Fail(scope, job) => {
                if job.is_none() {
                    *job = Some(a.start(Operation::Difference(self.active, *scope)));
                    return false;
                }
                if let Some(c) = poll(job, a) {
                    if self.active != c {
                        self.stats.clashes += 1;
                        if self.direct() {
                            self.stats.applications += 1;
                        }
                    }
                    self.active = c;
                    true
                } else {
                    false
                }
            }
        }
    }
    // At most one suspended operation's fixed-size root group per collection tick.
    pub fn root_group(&self, index: usize) -> Option<Vec<Root>> {
        if index == 0 {
            return Some(vec![self.base.clone(), self.root.clone()]);
        }
        let index = index - 1;
        if let Some(w) = self.work.get(index) {
            let mut roots = vec![];
            match w {
                Work::Resolve(_, _, Some(j)) => roots.push(j.root()),
                Work::Attach(_, _, _, Some(j)) => roots.push(j.root()),
                Work::Transfer(_, _, _, Some(j)) => roots.push(j.root()),
                Work::Equal(_, _, _, Some(j)) => roots.extend(j.roots()),
                Work::Wake(j) => roots.push(j.root()),
                Work::Anchor(r, ..) => roots.push(r.clone()),
                Work::Search {
                    matches, commit, ..
                } => {
                    if let Some(m) = matches {
                        roots.push(m.root());
                    }
                    if let Some(c) = commit {
                        roots.extend(c.graph_roots());
                    }
                }
                Work::Consume(_, _, _, Some(j)) => roots.extend(j.roots()),
                _ => {}
            }
            return Some(roots);
        }
        self.activations
            .get(index - self.work.len())
            .map(|x| vec![x.0.clone()])
    }
    pub fn discard_tick(&mut self) -> bool {
        if let Some(gate) = &mut self.gate {
            if gate.discard_tick() {
                self.gate = None;
            }
            return false;
        }
        if let Some(w) = self.work.front_mut() {
            if w.discard_tick() {
                self.work.pop_front();
            }
            return false;
        }
        self.activations.pop_front().is_none()
    }
}
impl Work {
    fn admission_scope(&mut self) -> Option<&mut Condition> {
        match self {
            Self::Fields(_, _, scope, _) | Self::Body(_, _, scope, _) => Some(scope),
            Self::Resolve(_, scope, None)
            | Self::Attach(_, _, scope, None)
            | Self::Transfer(_, _, scope, None)
            | Self::Equal(_, _, scope, None)
            | Self::Consume(_, scope, None, None)
            | Self::Fail(scope, None) => Some(scope),
            Self::Anchor(_, _, scope, 0) => Some(scope),
            _ => None,
        }
    }
    fn discard_tick(&mut self) -> bool {
        match self {
            Self::Links(links) => links.pop_front().is_none(),
            Self::Resolve(_, _, Some(j)) => j.discard_tick(),
            Self::Attach(_, _, _, Some(j)) => j.discard_tick(),
            Self::Transfer(_, _, _, Some(j)) => j.discard_tick(),
            Self::Equal(_, _, _, Some(j)) => j.discard_tick(),
            Self::Wake(j) => j.discard_tick(),
            Self::Search {
                matches, commit, ..
            } => {
                if let Some(c) = commit {
                    if c.discard_tick() {
                        *commit = None;
                    }
                    false
                } else if let Some(m) = matches {
                    m.discard_tick()
                } else {
                    true
                }
            }
            Self::Consume(_, _, Some(j), _) | Self::Fail(_, Some(j)) => j.discard_tick(),
            _ => true,
        }
    }
}
impl Trace for Work {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        if c.phase == 0 {
            return c.fields(&[match self {
                Self::Fields(_, _, s, _)
                | Self::Body(_, _, s, _)
                | Self::Resolve(_, s, _)
                | Self::Attach(_, _, s, _)
                | Self::Transfer(_, _, s, _)
                | Self::Equal(_, _, s, _)
                | Self::Anchor(_, _, s, _)
                | Self::Consume(_, s, _, _)
                | Self::Fail(s, _) => *s,
                _ => Condition::FALSE,
            }]);
        }
        if c.phase == 1 {
            return match self {
                Self::Links(links) => c.vector(links.len(), |i, child| {
                    if child.phase == 0 {
                        child.fields(&[links[i].support])
                    } else {
                        Step::Done
                    }
                }),
                Self::Resolve(_, _, j) => c.optional(j.as_ref()),
                Self::Attach(_, _, _, j) => c.optional(j.as_ref()),
                Self::Transfer(_, _, _, j) => c.optional(j.as_ref()),
                Self::Equal(_, _, _, j) => c.optional(j.as_ref()),
                Self::Wake(j) => c.optional(Some(j.as_ref())),
                Self::Search { matches, .. } => c.optional(matches.as_deref()),
                Self::Consume(_, _, j, _) | Self::Fail(_, j) => c.optional(j.as_ref()),
                _ => c.advance(),
            };
        }
        if c.phase == 2 {
            if let Self::Search { commit, .. } = self {
                return c.optional(commit.as_deref());
            }
        }
        Step::Done
    }
}
impl Trace for Normalizer {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.active]),
            1 => c.vector(self.work.len(), |i, child| self.work[i].trace(child)),
            2 => c.vector(self.activations.len(), |i, child| {
                if child.phase == 0 {
                    child.fields(&[self.activations[i].2])
                } else {
                    Step::Done
                }
            }),
            3 => c.optional(self.gate.as_ref()),
            _ => Step::Done,
        }
    }
}
