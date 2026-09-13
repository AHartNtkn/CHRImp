//! Experimental single-world foundation. Never selected by ordinary Engine.
//!
//! The certificate deliberately accepts one generative chain, not arbitrary CHR.
//! Distinct signatures separate levels; only cell values can be equated. Passive
//! relations cannot affect execution. A strict producer rank and consuming cell
//! contractions make the FIFO closure finite, including equal input tuples.
use crate::{observe::Output, program::{Atom, Instruction, Prepared}};
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet, VecDeque}, sync::Arc};

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Preparation {
    pub instructions_read: usize,
    pub rules_read: usize,
    pub signatures_checked: usize,
    pub query_atoms_checked: usize,
    pub opcodes_emitted: usize,
}

#[derive(Clone, Debug)]
struct Op { relation: usize, args: [usize; 2], arity: usize }
impl From<&Atom> for Op {
    fn from(a: &Atom) -> Self {
        Self { relation: a.relation, args: [a.args.first().copied().unwrap_or(0), a.args.get(1).copied().unwrap_or(0)], arity: a.args.len() }
    }
}
#[derive(Clone)]
struct Producer { rule: usize, variables: usize, ops: Vec<Op> }
pub struct Certificate {
    producers: Vec<Option<Producer>>,
    contractions: Vec<Option<usize>>,
    query: Vec<Op>,
    pub preparation: Preparation,
}

fn posts<'a>(code: &'a Prepared, pc: usize, cost: &mut Preparation) -> Option<Vec<&'a Atom>> {
    cost.instructions_read += 1;
    match &code.instructions()[pc] {
        Instruction::Post(atom) => Some(vec![atom]),
        Instruction::And(items) => {
            let mut result = Vec::new();
            for &pc in items { result.extend(posts(code, pc, cost)?); }
            Some(result)
        }
        Instruction::True => Some(vec![]),
        _ => None,
    }
}

impl Certificate {
    /// Failure is conservative and returns the work already spent on recognition.
    pub fn recognize(code: &Prepared) -> Result<Self, Preparation> {
        let mut cost = Preparation::default();
        Self::recognize_inner(code, &mut cost).ok_or(cost)
    }
    fn recognize_inner(code: &Prepared, cost: &mut Preparation) -> Option<Self> {
        let n = code.signatures().len();
        for sig in code.signatures() {
            cost.signatures_checked += 1;
            if sig.arity > 2 { return None; }
        }
        let mut producers = vec![None; n];
        let mut contractions = vec![None; n];
        for (i, rule) in code.rules().iter().enumerate() {
            cost.rules_read += 1;
            let head = rule.heads.first()?;
            if rule.kept == 0 && rule.heads.len() == 1 && head.args == [0, 1] {
                if producers[head.relation].is_some() { return None; }
                let body = posts(code, rule.body, cost)?;
                let ops: Vec<_> = body.into_iter().map(Op::from).collect();
                cost.opcodes_emitted += ops.len();
                producers[head.relation] = Some(Producer { rule: i, variables: rule.variables.len(), ops });
            } else if rule.kept == 1 && rule.heads.len() == 2
                && head.args == [0, 1] && rule.heads[1].args == [0, 2]
                && head.relation == rule.heads[1].relation && rule.variables.len() == 3
            {
                cost.instructions_read += 1;
                if !matches!(code.instructions()[rule.body], Instruction::Equal(1, 2) | Instruction::Equal(2, 1))
                    || contractions[head.relation].replace(i).is_some() { return None; }
            } else { return None; }
        }
        let mut next = vec![None; n];
        let mut incoming = vec![0; n];
        let mut passive = BTreeSet::new();
        let mut cells = BTreeSet::new();
        let mut count = 0;
        for (relation, producer) in producers.iter().enumerate() {
            let Some(p) = producer else { continue; };
            count += 1;
            if contractions[relation].is_some() { return None; }
            match p.ops.as_slice() {
                [end] if p.variables == 2 && end.arity == 2 && end.args == [1, 0] => {
                    if producers[end.relation].is_some() || contractions[end.relation].is_some()
                        || !passive.insert(end.relation) { return None; }
                }
                [cell, mark, control] if p.variables == 3
                    && cell.arity == 2 && cell.args == [0, 2]
                    && mark.arity == 2 && mark.args == [1, 2]
                    && control.arity == 2 && control.args == [2, 1] => {
                    if producers[cell.relation].is_some() || !cells.insert(cell.relation)
                        || producers[mark.relation].is_some() || contractions[mark.relation].is_some()
                        || !passive.insert(mark.relation) || producers[control.relation].is_none() { return None; }
                    next[relation] = Some(control.relation);
                    incoming[control.relation] += 1;
                }
                _ => return None,
            }
        }
        if count == 0 || cells.iter().any(|r| passive.contains(r)) { return None; }
        if contractions.iter().enumerate().any(|(r, c)| c.is_some() && !cells.contains(&r)) { return None; }
        let roots: Vec<_> = producers.iter().enumerate().filter_map(|(r,p)| (p.is_some() && incoming[r] == 0).then_some(r)).collect();
        if roots.len() != 1 || incoming.iter().any(|&n| n > 1) { return None; }
        let root = roots[0];
        let mut seen = BTreeSet::new();
        let mut cursor = Some(root);
        while let Some(r) = cursor {
            if !seen.insert(r) { return None; }
            cursor = next[r];
        }
        if seen.len() != count { return None; }
        let query = posts(code, code.query(), cost)?;
        for atom in &query {
            cost.query_atoms_checked += 1;
            // No arbitrary initial cells or intermediate controls: provenance is
            // initial keys -> one fresh layer at a time, never cross-family alias.
            if (producers[atom.relation].is_some() && atom.relation != root)
                || cells.contains(&atom.relation) { return None; }
        }
        let query: Vec<_> = query.into_iter().map(Op::from).collect();
        cost.opcodes_emitted += query.len();
        Some(Self { producers, contractions, query, preparation: *cost })
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Record {
    pub relation: usize,
    pub args: [usize; 2],
    pub arity: usize,
    pub live: bool,
    pub event: u64,
}
#[derive(Clone, Debug)]
pub struct Pending {
    pub event: u64,
    pub rule: Option<usize>,
    pub pc: usize,
    pub variables: Vec<usize>,
    // Equality is a body operation, after its consuming application boundary.
    pub equality: Option<(usize, usize)>,
}
#[derive(Clone, Default)]
struct State {
    records: Vec<Record>,
    parents: Vec<usize>,
    ranks: Vec<u8>,
    families: Vec<BTreeMap<usize, usize>>,
    queue: VecDeque<usize>,
    pending: Option<Pending>,
    applications: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Work {
    pub record_writes: usize,
    pub live_bit_writes: usize,
    pub fresh_identities: usize,
    pub parent_reads: usize,
    pub parent_writes: usize,
    pub family_lookups: usize,
    pub family_writes: usize,
    pub attachment_migrations: usize,
    pub body_frames: usize,
    pub body_opcodes: usize,
    pub queue_pushes: usize,
    pub queue_pops: usize,
    pub dead_queue_skips: usize,
    pub snapshot_pins: usize,
    pub cow_states: usize,
    pub cow_records: usize,
    pub cow_identities: usize,
    pub cow_family_entries: usize,
    pub cow_queue_entries: usize,
    pub cow_pending_slots: usize,
    pub cancel_calls: usize,
    pub canceled_queue_entries: usize,
    pub canceled_pending_slots: usize,
    pub released_source_record_slots: usize,
    pub released_source_identity_slots: usize,
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Memory {
    pub records: usize,
    pub live_records: usize,
    pub dead_records: usize,
    pub record_capacity: usize,
    pub identities: usize,
    pub identity_capacity: usize,
    pub family_entries: usize,
    pub queue: usize,
    pub queue_capacity: usize,
    pub pending_slots: usize,
    pub state_owners: usize,
    pub direct_capacity_bytes: usize,
}
fn memory(state: &Arc<State>) -> Memory {
    let live_records = state.records.iter().filter(|r| r.live).count();
    Memory {
        records: state.records.len(), live_records,
        dead_records: state.records.len() - live_records,
        record_capacity: state.records.capacity(), identities: state.parents.len(),
        identity_capacity: state.parents.capacity(),
        family_entries: state.families.iter().map(BTreeMap::len).sum(),
        queue: state.queue.len(), queue_capacity: state.queue.capacity(),
        pending_slots: state.pending.as_ref().map_or(0, |p| p.variables.len()),
        state_owners: Arc::strong_count(state),
        // BTreeMap nodes, Arc header and pending allocation overhead are covered
        // by the maintained allocator; this gauge is explicitly direct capacity.
        direct_capacity_bytes: state.records.capacity() * size_of::<Record>()
            + state.parents.capacity() * size_of::<usize>() + state.ranks.capacity()
            + state.families.capacity() * size_of::<BTreeMap<usize, usize>>()
            + state.queue.capacity() * size_of::<usize>()
            + state.pending.as_ref().map_or(0, |p| p.variables.capacity() * size_of::<usize>()),
    }
}
fn find(state: &State, mut id: usize, reads: &mut usize) -> usize {
    loop {
        *reads += 1;
        let parent = state.parents[id];
        if parent == id { return id; }
        id = parent;
    }
}
fn fresh(state: &mut State, work: &mut Work) -> usize {
    let id = state.parents.len();
    state.parents.push(id); state.ranks.push(0); state.families.push(BTreeMap::new());
    work.fresh_identities += 1; work.parent_writes += 1;
    id
}

pub struct Flat {
    code: Arc<Prepared>,
    certificate: Certificate,
    state: Arc<State>,
    pub work: Work,
    canceled: bool,
    paused: bool,
    stop_after: Option<u64>,
}
impl Flat {
    pub fn new(code: Arc<Prepared>, certificate: Certificate) -> Self {
        let mut state = State::default();
        let mut work = Work::default();
        let variables = (0..code.query_variables().len()).map(|_| fresh(&mut state, &mut work)).collect();
        state.pending = Some(Pending { event: 0, rule: None, pc: 0, variables, equality: None });
        work.body_frames += 1;
        Self { code, certificate, state: Arc::new(state), work, canceled: false, paused: false, stop_after: None }
    }
    pub fn applications(&self) -> u64 { self.state.applications }
    pub fn done(&self) -> bool { self.state.queue.is_empty() && self.state.pending.is_none() }
    pub fn memory(&self) -> Memory { memory(&self.state) }
    pub fn pending(&self) -> Option<&Pending> { self.state.pending.as_ref() }
    pub fn records(&self) -> &[Record] { &self.state.records }
    /// Late stepping freezes after the next consuming application, before body.
    pub fn request_step(&mut self) -> Result<(), &'static str> {
        if self.canceled { return Err("canceled"); }
        self.paused = false; self.stop_after = Some(self.applications() + 1); Ok(())
    }
    pub fn paused(&self) -> bool { self.paused }
    pub fn resume(&mut self) { self.paused = false; self.stop_after = None; }
    pub fn snapshot(&mut self) -> Snapshot {
        self.work.snapshot_pins += 1;
        Snapshot { state: self.state.clone(), code: self.code.clone() }
    }
    fn mutable(&mut self) -> &mut State {
        if Arc::strong_count(&self.state) > 1 {
            self.work.cow_states += 1;
            self.work.cow_records += self.state.records.len();
            self.work.cow_identities += self.state.parents.len();
            self.work.cow_family_entries += self.state.families.iter().map(BTreeMap::len).sum::<usize>();
            self.work.cow_queue_entries += self.state.queue.len();
            self.work.cow_pending_slots += self.state.pending.as_ref().map_or(0, |p| p.variables.len());
        }
        Arc::make_mut(&mut self.state)
    }
    pub fn advance(&mut self) {
        if self.canceled || self.paused || self.done() { return; }
        self.mutable();
        let state = Arc::get_mut(&mut self.state).unwrap();
        let work = &mut self.work;
        if let Some(mut pending) = state.pending.take() {
            if let Some((a, b)) = pending.equality.take() {
                work.body_opcodes += 1;
                let mut a = find(state, a, &mut work.parent_reads);
                let mut b = find(state, b, &mut work.parent_reads);
                if a != b {
                    if state.ranks[a] < state.ranks[b] { std::mem::swap(&mut a, &mut b); }
                    state.parents[b] = a; work.parent_writes += 1;
                    if state.ranks[a] == state.ranks[b] { state.ranks[a] += 1; }
                    // Remove stale losing-root entries. Reactivation on the
                    // winning representative discovers newly enabled matches.
                    for (_, id) in std::mem::take(&mut state.families[b]) {
                        state.queue.push_back(id); work.queue_pushes += 1;
                        work.attachment_migrations += 1;
                    }
                }
            } else {
                let ops = match pending.rule {
                    None => &self.certificate.query,
                    Some(rule) => &self.certificate.producers[self.code.rules()[rule].heads[0].relation].as_ref().unwrap().ops,
                };
                if let Some(op) = ops.get(pending.pc) {
                    let mut args = [0; 2];
                    for (port, arg) in args.iter_mut().enumerate().take(op.arity) { *arg = pending.variables[op.args[port]]; }
                    let id = state.records.len();
                    state.records.push(Record { relation: op.relation, args, arity: op.arity, live: true, event: pending.event });
                    work.record_writes += 1; work.body_opcodes += 1;
                    if self.certificate.producers[op.relation].is_some() || self.certificate.contractions[op.relation].is_some() {
                        state.queue.push_back(id); work.queue_pushes += 1;
                    }
                    pending.pc += 1;
                    if pending.pc < ops.len() { state.pending = Some(pending); }
                }
            }
            return;
        }
        let id = state.queue.pop_front().unwrap(); work.queue_pops += 1;
        let record = state.records[id];
        if !record.live { work.dead_queue_skips += 1; return; }
        if let Some(producer) = &self.certificate.producers[record.relation] {
            state.records[id].live = false; work.live_bit_writes += 1;
            state.applications += 1;
            let mut variables = record.args.to_vec();
            while variables.len() < producer.variables { variables.push(fresh(state, work)); }
            state.pending = Some(Pending { event: state.applications, rule: Some(producer.rule), pc: 0, variables, equality: None });
            work.body_frames += 1;
        } else {
            let key = find(state, record.args[0], &mut work.parent_reads);
            work.family_lookups += 1;
            match state.families[key].get(&record.relation).copied() {
                Some(other) if other != id => {
                    debug_assert!(state.records[other].live);
                    state.records[id].live = false; work.live_bit_writes += 1;
                    state.applications += 1;
                    state.pending = Some(Pending { event: state.applications, rule: self.certificate.contractions[record.relation], pc: 0,
                        variables: vec![record.args[0], state.records[other].args[1], record.args[1]],
                        equality: Some((state.records[other].args[1], record.args[1])) });
                    work.body_frames += 1;
                }
                Some(_) => (),
                None => { state.families[key].insert(record.relation, id); work.family_writes += 1; }
            }
        }
        if self.stop_after.is_some_and(|n| state.applications >= n) { self.paused = true; self.stop_after = None; }
    }
    pub fn cancel(&mut self) {
        if self.canceled { return; }
        self.work.cancel_calls += 1;
        self.work.canceled_queue_entries += self.state.queue.len();
        self.work.canceled_pending_slots += self.state.pending.as_ref().map_or(0, |p| p.variables.len());
        self.work.released_source_record_slots += self.state.records.len();
        self.work.released_source_identity_slots += self.state.parents.len();
        self.state = Arc::new(State::default());
        self.canceled = true;
    }
}

#[derive(Clone)]
pub struct Snapshot { state: Arc<State>, code: Arc<Prepared> }
impl Snapshot {
    pub fn memory(&self) -> Memory { memory(&self.state) }
    pub fn pending(&self) -> Option<&Pending> { self.state.pending.as_ref() }
    pub fn records(&self) -> &[Record] { &self.state.records }
    pub fn observe(&self) -> Projection {
        Projection { snapshot: self.clone(), phase: 0, slot: 0, record: 0, port: 0, work: Observation::default() }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Observation { pub record_scans: usize, pub parent_reads: usize, pub scalars: usize }
pub struct Projection { snapshot: Snapshot, phase: u8, slot: usize, record: usize, port: usize, pub work: Observation }
impl Iterator for Projection {
    type Item = Output;
    fn next(&mut self) -> Option<Output> {
        let state = &self.snapshot.state;
        let result = loop {
            match self.phase {
                0 => { self.phase = 1; break Output::Begin { completion: 0, alternative: 0 }; }
                1 if self.slot < self.snapshot.code.query_variables().len() => {
                    let slot = self.slot; self.slot += 1;
                    break Output::Variable { slot, variable: find(state, slot, &mut self.work.parent_reads) as u64 };
                }
                1 => self.phase = 2,
                2 if self.record < state.records.len() => {
                    self.work.record_scans += 1;
                    let r = state.records[self.record];
                    if !r.live { self.record += 1; continue; }
                    self.port = 0; self.phase = 3;
                    break Output::Fact { occurrence: self.record as u64, relation: r.relation };
                }
                2 => { self.phase = 4; break Output::End; }
                3 => {
                    let r = state.records[self.record];
                    if self.port < r.arity {
                        let variable = find(state, r.args[self.port], &mut self.work.parent_reads) as u64;
                        self.port += 1; break Output::Port { variable };
                    }
                    self.record += 1; self.phase = 2; break Output::EndFact;
                }
                _ => return None,
            }
        };
        self.work.scalars += 1;
        Some(result)
    }
}

/// Selection is from the original prepared source, before any admission. This
/// prototype supports committed snapshots/pending descriptors and no history or
/// choice-selection API. Callers requesting either use the incumbent outright.
pub enum Adapter { Flat(Flat), Incumbent(Box<crate::engine::Engine>) }
impl Adapter {
    pub fn new(code: Arc<Prepared>, history: bool, choice_observation: bool) -> (Self, Preparation) {
        if history || choice_observation {
            return (Self::Incumbent(Box::new(crate::engine::Engine::with_history(code, history))), Preparation::default());
        }
        match Certificate::recognize(&code) {
            Ok(c) => { let p = c.preparation; (Self::Flat(Flat::new(code, c)), p) }
            Err(p) => (Self::Incumbent(Box::new(crate::engine::Engine::new(code))), p),
        }
    }
}
