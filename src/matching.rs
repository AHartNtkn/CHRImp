//! Lazy, nonbinding, occurrence-distinct multihead matching.
//!
//! This enumerates eligible rule applications, not search alternatives. The
//! executor commits applications and creates alternatives only for disjunction.
//! Cursors read a pinned immutable root; commitment must revalidate current
//! liveness, identity and propagation eligibility before using a candidate.

use crate::condition::{Arena, Condition, Job, Operation, Progress};
use crate::gc::discard_slot;
use crate::graph::{Graph, Occurrences};
use crate::identity::{Equal, ResolveStatus};
use crate::members::Members;
use crate::program::Prepared;
use crate::store::Root;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub struct Match {
    pub occurrences: Vec<u64>,
    pub bindings: Vec<u64>,
    pub support: Condition,
}
#[derive(Debug, PartialEq, Eq)]
pub enum MatchStatus {
    Pending,
    Found(Match),
    Done,
}
#[derive(Debug, PartialEq, Eq)]
pub enum MatchError {
    InvalidRoot,
    InvalidRule,
    InvalidAnchor,
}
impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRoot => "stale or foreign matching root",
            Self::InvalidRule => "unknown rule",
            Self::InvalidAnchor => "anchor head is outside the rule",
        })
    }
}
impl std::error::Error for MatchError {}

#[derive(Clone, Copy)]
enum Lookup {
    Relation,
    Port { port: usize, variable: u64 },
    Tuple(u64),
}

enum SourceKind {
    Shared(crate::graph::restriction::Subscriber),
    One {
        occurrence: Option<u64>,
        relation: usize,
    },
    Relation(Occurrences),
    Port {
        members: Box<Members>,
        bucket: Option<Occurrences>,
        membership: Condition,
        relation: usize,
        port: usize,
    },
}
enum SourceStatus {
    Pending,
    Found(u64, Condition),
    Done,
}
struct Source {
    root: Root,
    scope: Condition,
    kind: SourceKind,
    boolean: Option<Job>,
    occurrence: u64,
}
impl Source {
    fn new(
        g: &Graph,
        root: Root,
        relation: usize,
        scope: Condition,
        lookup: Lookup,
        anchor: Option<u64>,
    ) -> Self {
        let kind = if let Some(id) = anchor {
            SourceKind::One {
                occurrence: Some(id),
                relation,
            }
        } else {
            match lookup {
                Lookup::Port { port, variable } => SourceKind::Port {
                    members: Box::new(Members::new(g, root.clone(), variable, scope)),
                    bucket: None,
                    membership: Condition::FALSE,
                    relation,
                    port,
                },
                Lookup::Relation => SourceKind::Relation(
                    g.relation(root.clone(), relation)
                        .expect("prepared relation"),
                ),
                Lookup::Tuple(hash) => SourceKind::Relation(g.tuple(root.clone(), relation, hash)),
            }
        };
        Self {
            root,
            scope,
            kind,
            boolean: None,
            occurrence: 0,
        }
    }
    fn roots(&self) -> impl Iterator<Item = Condition> + '_ {
        let (members, c) = match &self.kind {
            SourceKind::Port {
                members,
                membership,
                ..
            } => (Some(members), *membership),
            _ => (None, Condition::FALSE),
        };
        [self.scope, c]
            .into_iter()
            .chain(self.boolean.iter().flat_map(|j| j.roots()))
            .chain(members.into_iter().flat_map(|m| m.condition_roots()))
    }
    fn discard_tick(&mut self) -> bool {
        self.scope = Condition::FALSE;
        if discard_slot(&mut self.boolean, |child| child.discard_tick()) {
            return false;
        }
        if let SourceKind::Port { members, .. } = &mut self.kind {
            return members.discard_tick();
        }
        true
    }
    fn tick(&mut self, g: &Graph, a: &mut Arena) -> SourceStatus {
        if let Some(job) = self.boolean.as_mut() {
            if let Progress::Complete(c) = job.tick(a) {
                self.boolean = None;
                return if c == Condition::FALSE {
                    SourceStatus::Pending
                } else {
                    SourceStatus::Found(self.occurrence, c)
                };
            }
            return SourceStatus::Pending;
        }
        let next = match &mut self.kind {
            SourceKind::Shared(subscriber) => match subscriber.tick(g) {
                crate::graph::restriction::Status::Pending => return SourceStatus::Pending,
                crate::graph::restriction::Status::Done => return SourceStatus::Done,
                crate::graph::restriction::Status::Found(id) => {
                    let fact = g
                        .fact(self.root.clone(), id)
                        .expect("unchanged restriction bucket");
                    Some((id, self.scope, fact.support))
                }
            },
            SourceKind::One {
                occurrence,
                relation,
            } => {
                let Some(id) = occurrence.take() else {
                    return SourceStatus::Done;
                };
                let Some(fact) = g
                    .fact(self.root.clone(), id)
                    .filter(|f| f.relation == *relation)
                else {
                    return SourceStatus::Done;
                };
                Some((id, self.scope, fact.support))
            }
            SourceKind::Relation(cursor) => {
                let Some((id, support)) = cursor.next(g) else {
                    return SourceStatus::Done;
                };
                Some((id, self.scope, support))
            }
            SourceKind::Port {
                members,
                bucket,
                membership,
                relation,
                port,
            } => {
                if let Some(cursor) = bucket {
                    if let Some((id, support)) = cursor.next(g) {
                        Some((id, *membership, support))
                    } else {
                        *bucket = None;
                        None
                    }
                } else {
                    match members.tick(g, a) {
                        ResolveStatus::Found { variable, support } => {
                            *membership = support;
                            *bucket = Some(
                                g.port(self.root.clone(), *relation, *port, variable)
                                    .expect("prepared port"),
                            );
                            None
                        }
                        ResolveStatus::Done => return SourceStatus::Done,
                        ResolveStatus::Pending => None,
                    }
                }
            }
        };
        if let Some((id, scope, support)) = next {
            // This row is read at the source's pinned root.
            if let Some(c) = a.direct(Operation::And(scope, support)) {
                return if c == Condition::FALSE {
                    SourceStatus::Pending
                } else {
                    SourceStatus::Found(id, c)
                };
            }
            self.occurrence = id;
            self.boolean = Some(a.start(Operation::And(scope, support)));
        }
        SourceStatus::Pending
    }
}
struct Frame {
    head: usize,
    source: Source,
    before: usize,
    hit: Condition,
    verified_port: Option<usize>,
}
#[derive(Clone, Copy)]
enum Phase {
    Select,
    Candidate,
    Distinct,
    Ports,
    Equality,
    Copy,
    Rollback,
    Done,
}

pub struct Matches {
    root: Root,
    code: Arc<Prepared>,
    rule: usize,
    scope: Condition,
    anchor: Option<(usize, u64)>,
    bindings: Vec<Option<u64>>,
    occurrences: Vec<Option<u64>>,
    trail: Vec<usize>,
    frames: Vec<Frame>,
    select_head: usize,
    select_port: usize,
    score: usize,
    lookup: Lookup,
    tuple_hash: u64,
    tuple_bound: bool,
    key_count: usize,
    select_members: Option<Members>,
    select_count: usize,
    relation_count: usize,
    select_discard: bool,
    best: Option<(usize, usize, usize, Lookup)>,
    candidate: u64,
    current: Condition,
    position: usize,
    arguments: Option<Arc<Vec<u64>>>,
    equality: Option<Equal>,
    output: Option<Match>,
    pop_frame: bool,
    phase: Phase,
    candidate_visits: u64,
    discard: u8,
}
impl Matches {
    pub fn new(
        g: &Graph,
        root: Root,
        code: Arc<Prepared>,
        rule: usize,
        scope: Condition,
        anchor: Option<(usize, u64)>,
    ) -> Result<Self, MatchError> {
        if !g.index.contains(&root) {
            return Err(MatchError::InvalidRoot);
        }
        let plan = code.rules.get(rule).ok_or(MatchError::InvalidRule)?;
        if anchor.is_some_and(|(head, _)| head >= plan.heads.len()) {
            return Err(MatchError::InvalidAnchor);
        }
        let bindings = vec![None; plan.head_variables];
        let occurrences = vec![None; plan.heads.len()];
        Ok(Self {
            discard: 0,
            root,
            code,
            rule,
            scope,
            anchor,
            bindings,
            occurrences,
            trail: Vec::new(),
            frames: Vec::new(),
            select_head: 0,
            select_port: 0,
            score: 0,
            lookup: Lookup::Relation,
            tuple_hash: 0,
            tuple_bound: false,
            key_count: usize::MAX,
            select_members: None,
            select_count: 0,
            relation_count: 0,
            select_discard: false,
            best: None,
            candidate: 0,
            current: scope,
            position: 0,
            arguments: None,
            equality: None,
            output: None,
            pop_frame: false,
            phase: if scope == Condition::FALSE {
                Phase::Done
            } else {
                Phase::Select
            },
            candidate_visits: 0,
        })
    }
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    pub fn candidate_visits(&self) -> u64 {
        self.candidate_visits
    }
    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        [self.scope, self.current]
            .into_iter()
            .chain(
                self.frames
                    .iter()
                    .flat_map(|f| [f.hit].into_iter().chain(f.source.roots())),
            )
            .chain(self.equality.iter().flat_map(|e| e.condition_roots()))
            .chain(
                self.select_members
                    .iter()
                    .flat_map(Members::condition_roots),
            )
            .chain(self.output.iter().map(|o| o.support))
    }
    fn select(&mut self) {
        self.select_head = 0;
        self.select_port = 0;
        self.score = 0;
        self.lookup = Lookup::Relation;
        self.key_count = usize::MAX;
        self.best = None;
        self.phase = Phase::Select;
    }
    fn push_frame(&mut self, g: &Graph, head: usize, lookup: Lookup, anchor: Option<u64>) {
        let atom = &self.code.rules[self.rule].heads[head];
        let scope = self.frames.last().map_or(self.scope, |f| f.hit);
        let verified_port = match lookup {
            Lookup::Port { port, .. } => Some(port),
            _ => None,
        };
        let mut source = Source::new(g, self.root.clone(), atom.relation, scope, lookup, anchor);
        if anchor.is_none() && atom.args.len() <= crate::graph::restriction::MAX_PORTS {
            if let Lookup::Port { port, .. } = lookup {
                let mut bound = [None; crate::graph::restriction::MAX_PORTS];
                for (i, &slot) in atom.args.iter().enumerate() {
                    bound[i] = self.bindings[slot];
                }
                if let Some(subscriber) = g.restriction(&self.root, atom.relation, port, bound) {
                    source.kind = SourceKind::Shared(subscriber);
                }
            }
        }
        self.frames.push(Frame {
            head,
            source,
            before: self.trail.len(),
            hit: scope,
            verified_port,
        });
        self.phase = Phase::Candidate;
    }
    fn rollback(&mut self, pop: bool) {
        self.pop_frame = pop;
        self.arguments = None;
        self.equality = None;
        self.phase = Phase::Rollback;
    }
    pub fn discard_tick(&mut self) -> bool {
        if self.discard == 0 {
            self.discard = 1;
            self.scope = Condition::FALSE;
            self.current = Condition::FALSE;
            self.bindings = Vec::new();
            self.occurrences = Vec::new();
            self.trail = Vec::new();
            self.arguments = None;
            self.output = None;
        }
        match self.discard {
            1 => {
                if let Some(m) = &mut self.select_members {
                    if m.discard_tick() {
                        self.select_members = None;
                    }
                } else if let Some(e) = &mut self.equality {
                    if e.discard_tick() {
                        self.equality = None;
                    }
                } else {
                    self.discard = 2;
                }
            }
            2 => {
                if let Some(frame) = self.frames.last_mut() {
                    if frame.source.discard_tick() {
                        self.frames.pop();
                    }
                } else {
                    self.frames = Vec::new();
                    self.discard = 3;
                }
            }
            _ => return true,
        }
        false
    }

    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> MatchStatus {
        assert_eq!(self.discard, 0, "discarded continuation cannot resume");
        match self.phase {
            Phase::Select => {
                if self.frames.len() == self.occurrences.len() {
                    self.output = Some(Match {
                        occurrences: Vec::new(),
                        bindings: Vec::new(),
                        support: self.frames.last().map_or(self.scope, |f| f.hit),
                    });
                    self.position = 0;
                    self.phase = Phase::Copy;
                } else if let Some((head, id)) = self.anchor.take() {
                    self.push_frame(g, head, Lookup::Relation, Some(id));
                } else if self.select_head == self.occurrences.len() {
                    let (_, _, head, lookup) = self.best.expect("unmatched head");
                    self.push_frame(g, head, lookup, None);
                } else if self.occurrences[self.select_head].is_some() {
                    self.select_head += 1;
                } else {
                    let atom = &self.code.rules[self.rule].heads[self.select_head];
                    if self.select_port == 0 && self.select_members.is_none() {
                        self.relation_count = g.relation_count(&self.root, atom.relation);
                        self.key_count = self.relation_count;
                        self.tuple_hash = 0;
                        self.tuple_bound = g.has_tuple_index(atom.relation);
                    }
                    if self.select_port < atom.args.len() {
                        if let Some(members) = &mut self.select_members {
                            // Do not enumerate a large class merely to plan a small
                            // relation lookup. A relation scan checks every port using
                            // Equal, preserving conditional and nonbinding semantics.
                            self.select_discard |= members.visits()
                                > self
                                    .relation_count
                                    .saturating_mul(2 + self.occurrences.len() + atom.args.len())
                                    as u64;
                            if self.select_discard {
                                if members.discard_tick() {
                                    self.select_members = None;
                                    self.select_discard = false;
                                    self.select_port += 1;
                                }
                                return MatchStatus::Pending;
                            }
                            match members.tick(g, a) {
                                ResolveStatus::Found { variable, .. } => {
                                    // Count all supported alias buckets, not just the raw
                                    // binding. Disjoint membership fragments may count a
                                    // bucket twice, matching the source's traversal cost.
                                    self.select_count =
                                        self.select_count.saturating_add(g.port_count(
                                            &self.root,
                                            atom.relation,
                                            self.select_port,
                                            variable,
                                        ));
                                }
                                ResolveStatus::Done => {
                                    if self.select_count < self.key_count {
                                        self.lookup = Lookup::Port {
                                            port: self.select_port,
                                            variable: self.bindings[atom.args[self.select_port]]
                                                .unwrap(),
                                        };
                                        self.key_count = self.select_count;
                                    }
                                    self.select_members = None;
                                    self.select_port += 1;
                                }
                                ResolveStatus::Pending => {}
                            }
                        } else if let Some(variable) = self.bindings[atom.args[self.select_port]] {
                            self.score += 1;
                            self.tuple_hash = crate::graph::tuple_hash(self.tuple_hash, variable);
                            self.tuple_bound &= g.singleton(&self.root, variable);
                            if self.relation_count <= 1 {
                                self.select_port += 1;
                                return MatchStatus::Pending;
                            }
                            self.select_count = 0;
                            let scope = self.frames.last().map_or(self.scope, |f| f.hit);
                            self.select_members =
                                Some(Members::new(g, self.root.clone(), variable, scope));
                        } else {
                            self.tuple_bound = false;
                            self.select_port += 1;
                        }
                    } else {
                        if self.tuple_bound {
                            let count = g.tuple_count(&self.root, atom.relation, self.tuple_hash);
                            if count < self.key_count {
                                self.key_count = count;
                                self.lookup = Lookup::Tuple(self.tuple_hash);
                            }
                        }
                        if self.best.is_none_or(|(score, count, _, _)| {
                            self.score > score || (self.score == score && self.key_count < count)
                        }) {
                            self.best =
                                Some((self.score, self.key_count, self.select_head, self.lookup));
                        }
                        self.select_head += 1;
                        self.select_port = 0;
                        self.score = 0;
                        self.lookup = Lookup::Relation;
                        self.key_count = usize::MAX;
                    }
                }
            }
            Phase::Candidate => match self
                .frames
                .last_mut()
                .expect("candidate head")
                .source
                .tick(g, a)
            {
                SourceStatus::Found(id, c) => {
                    self.candidate_visits += 1;
                    self.candidate = id;
                    self.current = c;
                    self.position = 0;
                    self.phase = Phase::Distinct;
                }
                SourceStatus::Done => self.rollback(true),
                SourceStatus::Pending => {}
            },
            Phase::Distinct => {
                if self.position == self.occurrences.len() {
                    self.arguments = Some(g.arguments(self.candidate));
                    self.position = 0;
                    self.phase = Phase::Ports;
                } else if self.occurrences[self.position] == Some(self.candidate) {
                    self.rollback(false);
                } else {
                    self.position += 1;
                }
            }
            Phase::Ports => {
                let frame = self.frames.last_mut().expect("current head");
                let atom = &self.code.rules[self.rule].heads[frame.head];
                if self.position == atom.args.len() {
                    self.occurrences[frame.head] = Some(self.candidate);
                    frame.hit = self.current;
                    self.arguments = None;
                    self.select();
                } else {
                    let slot = atom.args[self.position];
                    let actual = self.arguments.as_ref().expect("candidate ports")[self.position];
                    if let Some(expected) = self.bindings[slot] {
                        // The indexed membership lookup already proved its chosen
                        // port under this candidate's support. Other ports still
                        // require their own existing-identity checks.
                        if expected == actual || frame.verified_port == Some(self.position) {
                            self.position += 1;
                            return MatchStatus::Pending;
                        }
                        self.equality = Some(Equal::new(
                            g,
                            self.root.clone(),
                            expected,
                            actual,
                            self.current,
                        ));
                        self.phase = Phase::Equality;
                    } else {
                        self.bindings[slot] = Some(actual);
                        self.trail.push(slot);
                        self.position += 1;
                    }
                }
            }
            Phase::Equality => {
                if let Some(c) = self.equality.as_mut().expect("identity guard").tick(g, a) {
                    self.current = c;
                    self.equality = None;
                    if c == Condition::FALSE {
                        self.rollback(false);
                    } else {
                        self.position += 1;
                        self.phase = Phase::Ports;
                    }
                }
            }
            Phase::Copy => {
                let output = self.output.as_mut().expect("matched tuple");
                if self.position < self.occurrences.len() {
                    output
                        .occurrences
                        .push(self.occurrences[self.position].expect("matched head"));
                    self.position += 1;
                } else if self.position < self.occurrences.len() + self.bindings.len() {
                    output.bindings.push(
                        self.bindings[self.position - self.occurrences.len()]
                            .expect("matched variable"),
                    );
                    self.position += 1;
                } else {
                    let output = self.output.take().expect("matched tuple");
                    self.rollback(false);
                    return MatchStatus::Found(output);
                }
            }
            Phase::Rollback => {
                let frame = self.frames.last().expect("backtracking head");
                if self.trail.len() > frame.before {
                    let slot = self.trail.pop().expect("binding trail");
                    self.bindings[slot] = None;
                } else {
                    self.occurrences[frame.head] = None;
                    if self.pop_frame {
                        self.frames.pop();
                        self.pop_frame = false;
                        if self.frames.is_empty() {
                            self.bindings = Vec::new();
                            self.occurrences = Vec::new();
                            self.trail = Vec::new();
                            self.frames = Vec::new();
                            self.phase = Phase::Done;
                        }
                    } else {
                        self.phase = Phase::Candidate;
                    }
                }
            }
            Phase::Done => return MatchStatus::Done,
        }
        MatchStatus::Pending
    }
}

impl Trace for Source {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        let (members, membership) = match &self.kind {
            SourceKind::Port {
                members,
                membership,
                ..
            } => (Some(members.as_ref()), *membership),
            _ => (None, Condition::FALSE),
        };
        match cursor.phase {
            0 => cursor.fields(&[self.scope, membership]),
            1 => cursor.optional(self.boolean.as_ref()),
            2 => cursor.optional(members),
            _ => Step::Done,
        }
    }
}

impl Trace for Frame {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.hit]),
            1 => cursor.optional(Some(&self.source)),
            _ => Step::Done,
        }
    }
}

impl Trace for Matches {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[self.scope, self.current]),
            1 => cursor.vector(self.frames.len(), |i, child| self.frames[i].trace(child)),
            2 => cursor.optional(self.equality.as_ref()),
            3 => match &self.output {
                Some(output) => cursor.fields(&[output.support]),
                None => cursor.advance(),
            },
            4 => cursor.optional(self.select_members.as_ref()),
            _ => Step::Done,
        }
    }
}
