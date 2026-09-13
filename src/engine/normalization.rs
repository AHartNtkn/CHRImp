//! Direct attachment normalization with resumable source application boundaries.
use super::*;
use crate::gc::discard_slot;
use crate::graph::constructors::{Attach, AttachmentStatus, Transfer};
use crate::identity::{Resolve, ResolveStatus, UnionLink};
use crate::program::constructors::Constructors;
use crate::trace::{Cursor as TraceCursor, Step, Trace};

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct NormalizationStats {
    pub conditional_dispatches: u64,
    pub known_arm_admissions: u64,
    pub generative_dispatches: u64,
    pub transactions: u64,
    pub steps: u64,
    pub attachment_overlaps: u64,
    /// Source field equality obligations admitted by consistency applications.
    pub field_equalities: u64,
    pub coalescences: u64,
    pub clashes: u64,
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
        self.applications += s.applications;
    }
}
#[derive(Clone)]
pub(super) struct Configuration {
    pub plan: Arc<Constructors>,
    pub source: Arc<Prepared>,
}
impl Engine {
    /// Includes work in unfinished normalization transactions.
    pub fn normalization_stats(&self) -> NormalizationStats {
        let mut stats = self.normalization_stats.clone();
        for s in self.queue.iter().chain(self.parked.values()) {
            if let Task::Body(b) = &s.task
                && let Some(n) = &b.normalizer
            {
                stats.add(&n.stats);
            }
        }
        stats
    }
    pub(super) fn normalization_tick(&mut self, id: u64, b: &mut Body) -> bool {
        let n = b.normalizer.as_mut().unwrap();
        if !n.done {
            let event = n.tick(&mut self.graph, &mut self.arena);
            if let Some(event) = event {
                self.state.graph = n.root.clone();
                self.semantic_regions |= self.active != n.active;
                self.active = n.active;
                match event {
                    Event::Application {
                        rule,
                        variables,
                        scope,
                    } => {
                        let event = self.ids.event();
                        let instruction = self.code.rules[rule].body;
                        self.applications += 1;
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.rules[rule].applied += 1;
                        }
                        if matches!(self.code.instructions[instruction], Instruction::Fail) {
                            // This same lane owner executes terminal failure next;
                            // no mutation can see the transient consumed attachment.
                            b.event = event;
                            b.instruction = instruction;
                            b.variables = variables;
                            b.scope = scope;
                            b.source_complete = false;
                            self.replace_body_record(id, b);
                        } else {
                            self.body(event, instruction, variables, scope);
                        }
                        self.step_application(event, rule, scope);
                        self.record(SnapshotKind::Application { rule, event }, self.active);
                    }
                    Event::Failure(scope) => {
                        b.source_complete = true;
                        self.sync_obligation(id, b);
                        #[cfg(feature = "diagnostics")]
                        {
                            self.diagnostics.fail_applications += 1;
                            self.diagnostics.fail_support_changes += 1;
                        }
                        self.record(SnapshotKind::Failure, scope);
                    }
                }
            }
            return false;
        }
        // Source RHS tasks and induced activations are admitted before the
        // transaction's conservative completion obligation retires.
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
        self.normalization_stats.add(&n.stats);
        b.normalizer = None;
        b.source_complete = true;
        self.finish_body_record(id);
        true
    }
}
enum Event {
    Application {
        rule: usize,
        variables: Arc<Vec<u64>>,
        scope: Condition,
    },
    Failure(Condition),
}
struct Apply {
    rule: usize,
    heads: [u64; 2],
    scope: Condition,
    variables: Vec<u64>,
    head: usize,
    port: usize,
    consume: usize,
    boolean: Option<Job>,
    update: Option<Update>,
}
enum Work {
    Links(VecDeque<UnionLink>),
    Resolve(u64, Condition, Option<Box<Resolve>>),
    Attach(u64, u64, Condition, Option<Attach>),
    Transfer(u64, u64, Condition, Option<Transfer>),
    Wake(Box<Wake>),
    Apply(Box<Apply>),
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
        n.activations.push_back((root, id, scope, false));
        if n.config
            .plan
            .relations
            .contains(&g.fact(n.root.clone(), id).unwrap().relation)
        {
            n.work.push_back(Work::Resolve(id, scope, None));
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
        if let Some(delta) = merge.take_delta() {
            n.work
                .push_back(Work::Wake(Box::new(Wake::from_delta(g, delta))));
        }
        let links = merge.take_links();
        if !links.is_empty() {
            n.work.push_front(Work::Links(links));
        }
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
    fn tick(&mut self, g: &mut Graph, a: &mut Arena) -> Option<Event> {
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
            let support = support?;
            *scope = support;
            if support == Condition::FALSE {
                self.work.pop_front();
                return None;
            }
        }
        let Some(mut work) = self.work.pop_front() else {
            self.done = true;
            return None;
        };
        let (done, event) = self.step(&mut work, g, a);
        if !done {
            self.work.push_front(work);
        }
        event
    }
    fn step(&mut self, w: &mut Work, g: &mut Graph, a: &mut Arena) -> (bool, Option<Event>) {
        let done = match w {
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
                    *job = Some(Box::new(Resolve::new(
                        g,
                        self.root.clone(),
                        g.arguments(*id)[0],
                        *scope,
                    )));
                    return (false, None);
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
                    return (false, None);
                }
                match job.as_mut().unwrap().tick(g, a, &mut self.root) {
                    AttachmentStatus::Pending => false,
                    AttachmentStatus::Done => true,
                    AttachmentStatus::Overlap(other, hit) => {
                        self.stats.attachment_overlaps += 1;
                        let left = g
                            .fact(self.root.clone(), other)
                            .expect("attached survivor")
                            .relation;
                        let right = g
                            .fact(self.root.clone(), *id)
                            .expect("incoming constructor")
                            .relation;
                        let rule = if left == right {
                            self.config.plan.consistency[&left]
                        } else {
                            self.config.plan.clashes[&(left.min(right), left.max(right))]
                        };
                        let r = &self.config.source.rules[rule];
                        let heads = if r.heads[0].relation == left {
                            [other, *id]
                        } else {
                            [*id, other]
                        };
                        self.work.push_front(Work::Apply(Box::new(Apply {
                            rule,
                            heads,
                            scope: hit,
                            variables: Vec::new(),
                            head: 0,
                            port: 0,
                            consume: r.kept,
                            boolean: None,
                            update: None,
                        })));
                        false
                    }
                }
            }
            Work::Transfer(win, lose, scope, job) => {
                if job.is_none() {
                    *job = Some(Transfer::new(g, self.root.clone(), *lose, *scope));
                    return (false, None);
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
            Work::Wake(wake) => match wake.tick(g, a) {
                WakeStatus::Pending => false,
                WakeStatus::Done => true,
                WakeStatus::Found {
                    occurrence,
                    support,
                } => {
                    self.activations
                        .push_back((wake.root(), occurrence, support, true));
                    false
                }
            },
            Work::Apply(app) => {
                let rule = &self.config.source.rules[app.rule];
                if app.variables.len() < rule.variables.len() {
                    app.variables.push(0);
                    return (false, None);
                }
                if app.head < 2 {
                    let head = &rule.heads[app.head];
                    if app.port == head.args.len() {
                        app.head += 1;
                        app.port = 0;
                    } else {
                        // Only the key is shared between the recognized heads.
                        if app.head == 0 || app.port != 0 {
                            app.variables[head.args[app.port]] =
                                g.arguments(app.heads[app.head])[app.port];
                        }
                        app.port += 1;
                    }
                    return (false, None);
                }
                if app.consume < 2 {
                    if let Some(update) = &mut app.update {
                        if let UpdateStatus::Complete(root) = update.tick(g) {
                            self.root = root;
                            app.update = None;
                            app.consume += 1;
                        }
                    } else if app.boolean.is_some() {
                        if let Some(c) = poll(&mut app.boolean, a) {
                            app.update = Some(
                                g.set_liveness(self.root.clone(), app.heads[app.consume], c)
                                    .expect("recognized live head"),
                            );
                        }
                    } else {
                        let old = g
                            .fact(self.root.clone(), app.heads[app.consume])
                            .expect("recognized head")
                            .support;
                        app.boolean = Some(a.start(Operation::Difference(old, app.scope)));
                    }
                    return (false, None);
                }
                self.stats.applications += 1;
                if rule.kept == 1 {
                    self.stats.coalescences += 1;
                    self.stats.field_equalities += (rule.heads[0].args.len() - 1) as u64;
                } else {
                    self.stats.clashes += 1;
                    self.work.push_front(Work::Fail(app.scope, None));
                }
                return (
                    true,
                    Some(Event::Application {
                        rule: app.rule,
                        variables: Arc::new(std::mem::take(&mut app.variables)),
                        scope: app.scope,
                    }),
                );
            }
            Work::Fail(scope, job) => {
                if job.is_none() {
                    *job = Some(a.start(Operation::Difference(self.active, *scope)));
                    return (false, None);
                }
                if let Some(active) = poll(job, a) {
                    self.active = active;
                    return (true, Some(Event::Failure(*scope)));
                }
                false
            }
        };
        (done, None)
    }
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
                Work::Wake(j) => roots.push(j.root()),
                Work::Apply(j) => {
                    if let Some(u) = &j.update {
                        roots.extend(u.roots());
                    }
                }
                _ => {}
            }
            return Some(roots);
        }
        self.activations
            .get(index - self.work.len())
            .map(|x| vec![x.0.clone()])
    }
    pub fn discard_tick(&mut self) -> bool {
        if discard_slot(&mut self.gate, |child| child.discard_tick()) {
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
            Self::Resolve(_, s, None)
            | Self::Attach(_, _, s, None)
            | Self::Transfer(_, _, s, None)
            | Self::Fail(s, None) => Some(s),
            Self::Apply(a) if a.variables.is_empty() => Some(&mut a.scope),
            _ => None,
        }
    }
    fn discard_tick(&mut self) -> bool {
        match self {
            Self::Links(l) => l.pop_front().is_none(),
            Self::Resolve(_, _, Some(j)) => j.discard_tick(),
            Self::Attach(_, _, _, Some(j)) => j.discard_tick(),
            Self::Transfer(_, _, _, Some(j)) => j.discard_tick(),
            Self::Wake(j) => j.discard_tick(),
            Self::Apply(a) => a.boolean.as_mut().is_none_or(|j| j.discard_tick()),
            Self::Fail(_, Some(j)) => j.discard_tick(),
            _ => true,
        }
    }
}
impl Trace for Work {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[match self {
                Self::Resolve(_, s, _)
                | Self::Attach(_, _, s, _)
                | Self::Transfer(_, _, s, _)
                | Self::Fail(s, _) => *s,
                Self::Apply(a) => a.scope,
                _ => Condition::FALSE,
            }]),
            1 => match self {
                Self::Links(l) => c.vector(l.len(), |i, child| {
                    if child.phase == 0 {
                        child.fields(&[l[i].support])
                    } else {
                        Step::Done
                    }
                }),
                Self::Resolve(_, _, j) => c.optional(j.as_deref()),
                Self::Attach(_, _, _, j) => c.optional(j.as_ref()),
                Self::Transfer(_, _, _, j) => c.optional(j.as_ref()),
                Self::Wake(j) => c.optional(Some(j.as_ref())),
                Self::Apply(a) => c.optional(a.boolean.as_ref()),
                Self::Fail(_, j) => c.optional(j.as_ref()),
            },
            _ => Step::Done,
        }
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
