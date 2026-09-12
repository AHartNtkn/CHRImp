//! Persistent remaining expressions, independent of scheduler continuations.
use super::*;
use crate::identity::{Resolve, ResolveStatus};
use crate::observe::ExpressionKind;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Obligation {
    event: u64,
    instruction: usize,
    start: usize,
    scope: Condition,
    // The current split's guard stays separate until budgeted projection. A
    // scheduler transition never needs to finish an extra Boolean operation.
    guard: Condition,
}
/// One persistent task record owns both its completion certificate and syntax.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Pending {
    pub scope: Condition,
    body: Option<u64>,
}
struct Descriptor {
    variables: Arc<Vec<u64>>,
    parts: [Option<Obligation>; 2],
    capture_epoch: u64,
    marked: u64,
}
#[derive(Default)]
pub(super) struct Obligations {
    pub index: Store<Pending>,
    descriptors: BTreeMap<u64, Descriptor>,
    next_descriptor: u64,
    capture_epoch: u64,
    epoch: u64,
}
impl Obligations {
    pub fn empty(&self) -> Root {
        self.index.empty()
    }
    pub fn descriptor_count(&self) -> usize {
        self.descriptors.len()
    }
    pub fn freeze_syntax(&mut self) {
        self.index.assert_mutable();
        self.capture_epoch = self
            .capture_epoch
            .checked_add(1)
            .expect("syntax capture epoch exhausted");
    }
    fn update_descriptor(
        &mut self,
        previous: Option<u64>,
        parts: [Option<Obligation>; 2],
        variables: &Arc<Vec<u64>>,
    ) -> Option<u64> {
        self.index.assert_mutable();
        if let Some(id) = previous {
            let descriptor = self.descriptors.get_mut(&id).expect("live body descriptor");
            debug_assert!(
                Arc::ptr_eq(&descriptor.variables, variables),
                "body variables are fixed"
            );
            if descriptor.parts == parts {
                return previous;
            }
            // Certificate cursors read only Pending.scope. Only syntax captures
            // freeze descriptor contents, so ordinary progress owns this epoch.
            if descriptor.capture_epoch == self.capture_epoch {
                descriptor.parts = parts;
                return previous;
            }
        } else if parts == [None; 2] {
            return None;
        }
        Some(self.allocate_descriptor(parts, variables))
    }
    fn allocate_descriptor(
        &mut self,
        parts: [Option<Obligation>; 2],
        variables: &Arc<Vec<u64>>,
    ) -> u64 {
        let id = self.next_descriptor;
        self.next_descriptor = id
            .checked_add(1)
            .expect("body descriptor identity exhausted");
        self.descriptors.insert(
            id,
            Descriptor {
                variables: variables.clone(),
                parts,
                capture_epoch: self.capture_epoch,
                marked: 0,
            },
        );
        id
    }
    /// Restrict a frozen pending version. The caller owns publication and must
    /// finish the job under the frozen maintenance lane before physical GC.
    pub fn restrict(&self, root: Root, assignments: Arc<BTreeMap<u64, bool>>) -> Restriction {
        Restriction {
            input: root,
            filter: self.index.filter(root),
            assignments: Some(assignments),
            draining: BTreeMap::new(),
            condition: None,
            pending: None,
            parts: [None; 2],
            field: 0,
            result: None,
        }
    }
    pub fn collect(&mut self, roots: std::vec::IntoIter<Root>) -> Collector {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("obligation collection epoch exhausted");
        Collector {
            index: self.index.collect(roots),
            after_descriptor: None,
            done: false,
        }
    }
}
/// A staged cofactor of certificate and syntax supports. Changed descriptors
/// get fresh identities; input versions remain immutable even in this epoch.
pub(super) struct Restriction {
    input: Root,
    filter: crate::store::Filter<Pending>,
    assignments: Option<Arc<BTreeMap<u64, bool>>>,
    draining: BTreeMap<u64, bool>,
    condition: Option<crate::condition::Restriction>,
    pending: Option<Pending>,
    parts: [Option<Obligation>; 2],
    // Zero is the leaf scope; 1..=4 are the two part scope/guard pairs.
    field: usize,
    result: Option<Root>,
}
impl Restriction {
    #[cfg(test)]
    fn roots(&self) -> impl Iterator<Item = Root> + '_ {
        std::iter::once(self.input).chain(self.filter.roots())
    }
    fn cleanup_tick(&mut self) -> bool {
        if let Some(assignments) = self.assignments.take() {
            if let Some(assignments) = Arc::into_inner(assignments) {
                self.draining = assignments;
            }
            return false;
        }
        self.draining.pop_first();
        self.draining.is_empty()
    }
    /// One filter/Boolean/cleanup transition, or an atomic descriptor+leaf
    /// publication. Keep source work frozen and publish only the complete root.
    pub fn tick(&mut self, store: &mut Obligations, arena: &mut Arena) -> Option<Root> {
        store.index.assert_mutable();
        assert!(
            store.index.contains(self.input),
            "stale or foreign pending restriction root"
        );
        if let Some(root) = self.result {
            return self.cleanup_tick().then_some(root);
        }
        if let Some(condition) = &mut self.condition {
            if let Progress::Complete(scope) = condition.tick(arena) {
                if self.field == 0 {
                    let pending = self.pending.as_mut().expect("restricted pending leaf");
                    pending.scope = scope;
                    if scope != Condition::FALSE
                        && let Some(id) = pending.body
                    {
                        self.parts = store.descriptors[&id].parts;
                        self.field = 1;
                    } else {
                        self.field = 5;
                    }
                } else {
                    let part = self.parts[(self.field - 1) / 2]
                        .as_mut()
                        .expect("restricted part");
                    if self.field % 2 == 1 {
                        part.scope = scope;
                    } else {
                        part.guard = scope;
                    }
                    self.field += 1;
                }
                self.condition = None;
            }
        } else if let Some(mut pending) = self.pending {
            if self.field == 5 {
                if pending.scope != Condition::FALSE
                    && let Some(id) = pending.body
                {
                    let descriptor = &store.descriptors[&id];
                    if descriptor.parts != self.parts {
                        let variables = descriptor.variables.clone();
                        pending.body = Some(store.allocate_descriptor(self.parts, &variables));
                    }
                }
                // Consume replace immediately: a fresh descriptor always has a
                // leaf in filter.roots() before GC can intervene between ticks.
                self.filter
                    .replace((pending.scope != Condition::FALSE).then_some(pending));
                match self.filter.tick(&mut store.index) {
                    crate::store::FilterStatus::Pending => {}
                    crate::store::FilterStatus::Complete(root) => self.result = Some(root),
                    crate::store::FilterStatus::Leaf { .. } => {
                        unreachable!("filter replacement transition")
                    }
                }
                self.pending = None;
                self.parts = [None; 2];
            } else if let Some(part) = self.parts[(self.field - 1) / 2] {
                let scope = if self.field % 2 == 1 {
                    part.scope
                } else {
                    part.guard
                };
                self.condition = Some(
                    arena.restrict(
                        scope,
                        self.assignments
                            .as_ref()
                            .expect("restriction assignments")
                            .clone(),
                    ),
                );
            } else {
                self.field += 1;
            }
        } else {
            match self.filter.tick(&mut store.index) {
                crate::store::FilterStatus::Pending => {}
                crate::store::FilterStatus::Leaf { value, .. } => {
                    self.pending = Some(value);
                    self.field = 0;
                    self.condition = Some(
                        arena.restrict(
                            value.scope,
                            self.assignments
                                .as_ref()
                                .expect("restriction assignments")
                                .clone(),
                        ),
                    );
                }
                crate::store::FilterStatus::Complete(root) => self.result = Some(root),
            }
        }
        None
    }
}

impl Trace for Restriction {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.fields(&[
                self.pending
                    .map_or(Condition::FALSE, |pending| pending.scope),
                self.filter
                    .values()
                    .next()
                    .map_or(Condition::FALSE, |pending| pending.scope),
                self.parts[0].map_or(Condition::FALSE, |part| part.scope),
                self.parts[0].map_or(Condition::FALSE, |part| part.guard),
                self.parts[1].map_or(Condition::FALSE, |part| part.scope),
                self.parts[1].map_or(Condition::FALSE, |part| part.guard),
            ]),
            1 => cursor.optional(self.condition.as_ref()),
            _ => Step::Done,
        }
    }
}

pub(super) struct Collector {
    index: crate::store::Collector<std::vec::IntoIter<Root>>,
    after_descriptor: Option<u64>,
    done: bool,
}
impl Collector {
    pub fn done(&self) -> bool {
        self.done
    }
    pub fn tick(&mut self, store: &mut Obligations) -> Option<[Condition; 5]> {
        self.index.validate(&store.index);
        if !self.index.done() {
            if let Some((_, value)) = self.index.tick(&mut store.index) {
                let mut roots = [Condition::FALSE; 5];
                roots[0] = value.scope;
                if let Some(id) = value.body {
                    let descriptor = store
                        .descriptors
                        .get_mut(&id)
                        .expect("rooted body descriptor");
                    if descriptor.marked != store.epoch {
                        descriptor.marked = store.epoch;
                        for (part, body) in descriptor.parts.into_iter().enumerate() {
                            if let Some(body) = body {
                                roots[part * 2 + 1] = body.scope;
                                roots[part * 2 + 2] = body.guard;
                            }
                        }
                    }
                }
                return Some(roots);
            }
        } else if !self.done {
            let next = match self.after_descriptor {
                Some(id) => store.descriptors.range((Excluded(id), Unbounded)).next(),
                None => store.descriptors.first_key_value(),
            };
            if let Some((&id, descriptor)) = next {
                if descriptor.marked != store.epoch {
                    store.descriptors.remove(&id);
                }
                self.after_descriptor = Some(id);
            } else {
                self.done = true;
            }
        }
        None
    }
}
impl Engine {
    pub(super) fn finish_body_record(&mut self, id: u64) {
        // Only a history record publishes this intermediate point. Otherwise
        // the scheduler removes the completed task at this same tick's end.
        if self.record_history {
            let key = [id, 0, 0, 0];
            let mut pending = self
                .obligations
                .index
                .get(self.pending_root, &key)
                .expect("scheduled body");
            let previous = pending.body;
            let Some(id) = previous else {
                return;
            };
            let variables = self.obligations.descriptors[&id].variables.clone();
            pending.body = self
                .obligations
                .update_descriptor(previous, [None; 2], &variables);
            if pending.body == previous {
                return;
            }
            self.pending_root = self
                .obligations
                .index
                .insert(self.pending_root, key, pending);
        }
    }
    pub(super) fn sync_obligation(&mut self, id: u64, body: &Body) {
        let key = [id, 0, 0, 0];
        let mut pending = self
            .obligations
            .index
            .get(self.pending_root, &key)
            .expect("scheduled body");
        let parts = self.obligation_parts(body);
        let previous = pending.body;
        pending.body = self
            .obligations
            .update_descriptor(previous, parts, &body.variables);
        if pending.body == previous {
            return;
        }
        self.pending_root = self
            .obligations
            .index
            .insert(self.pending_root, key, pending);
    }
    pub(super) fn pending_task(&mut self, scope: Condition, task: &Task) -> Pending {
        let body = match task {
            Task::Body(body) => {
                let parts = self.obligation_parts(body);
                self.obligations
                    .update_descriptor(None, parts, &body.variables)
            }
            _ => None,
        };
        Pending { scope, body }
    }
    fn obligation_parts(&self, body: &Body) -> [Option<Obligation>; 2] {
        if body.scope == Condition::FALSE {
            return [None; 2];
        }
        let base = Obligation {
            event: body.event,
            instruction: body.instruction,
            start: 0,
            scope: body.scope,
            guard: Condition::TRUE,
        };
        let mut parts = [Some(base), None];
        match &self.code.instructions[body.instruction] {
            Instruction::And(items) => {
                parts[0] = (body.index < items.len()).then_some(Obligation {
                    start: body.index,
                    ..base
                });
            }
            Instruction::Or(items) => {
                if matches!(body.phase, BodyPhase::Left | BodyPhase::Right) {
                    // Left has not yet admitted its child. Right has admitted
                    // it, so only the complementary suffix remains here.
                    parts[0] = matches!(body.phase, BodyPhase::Left).then_some(Obligation {
                        instruction: items[body.index],
                        guard: body.decision,
                        ..base
                    });
                    parts[1] = (body.index + 1 < items.len()).then_some(Obligation {
                        start: body.index + 1,
                        guard: body.decision.not(),
                        ..base
                    });
                } else {
                    parts[0] = (body.index < items.len()).then_some(Obligation {
                        start: body.index,
                        ..base
                    });
                }
            }
            _ => {}
        }
        parts
    }
}

struct Frame {
    instruction: usize,
    index: usize,
    opened: bool,
}
enum Phase {
    Next,
    Support,
    Guard,
    Expression,
    End,
    Done,
}
pub(super) struct Projection {
    // The owning Inspection retains the whole snapshot root (including this
    // cursor's leaves and descriptors) until projection/discard completes.
    cursor: Cursor,
    alternative: Condition,
    scope: Condition,
    current: Option<Obligation>,
    descriptor: Option<u64>,
    remaining: [Option<Obligation>; 2],
    job: Option<Job>,
    resolve: Option<Resolve>,
    representative: Option<u64>,
    frames: Vec<Frame>,
    phase: Phase,
}
impl Projection {
    pub fn new(store: &Obligations, root: Root, alternative: Condition) -> Self {
        Self {
            cursor: store.index.range(root, [0; 4], [u64::MAX; 4]),
            alternative,
            scope: Condition::FALSE,
            current: None,
            descriptor: None,
            remaining: [None; 2],
            job: None,
            resolve: None,
            representative: None,
            frames: vec![],
            phase: Phase::Next,
        }
    }
    pub fn discard_tick(&mut self) -> bool {
        self.frames = Vec::new();
        if let Some(job) = &mut self.job {
            if job.discard_tick() {
                self.job = None;
            }
        } else if let Some(resolve) = &mut self.resolve {
            if resolve.discard_tick() {
                self.resolve = None;
            }
        } else {
            return true;
        }
        false
    }
    pub fn tick(
        &mut self,
        store: &Obligations,
        graph: &Graph,
        root: Root,
        arena: &mut Arena,
        code: &Prepared,
    ) -> ObserveStatus {
        match self.phase {
            Phase::Next => {
                let value = self.remaining[0]
                    .take()
                    .or_else(|| self.remaining[1].take());
                if let Some(value) = value {
                    self.current = Some(value);
                    self.job = Some(arena.start(Operation::And(self.alternative, value.scope)));
                    self.phase = Phase::Support;
                } else if let Some((_, value)) = self.cursor.next(&store.index) {
                    self.descriptor = value.body;
                    self.remaining = value
                        .body
                        .map_or([None; 2], |id| store.descriptors[&id].parts);
                } else {
                    self.phase = Phase::Done;
                }
            }
            Phase::Support => {
                if let Some(scope) = poll(&mut self.job, arena) {
                    self.job =
                        Some(arena.start(Operation::And(scope, self.current.unwrap().guard)));
                    self.phase = Phase::Guard;
                }
            }
            Phase::Guard => {
                if let Some(scope) = poll(&mut self.job, arena) {
                    self.scope = scope;
                    if scope == Condition::FALSE {
                        self.phase = Phase::Next;
                    } else {
                        let value = self.current.unwrap();
                        self.frames.push(Frame {
                            instruction: value.instruction,
                            index: value.start,
                            opened: false,
                        });
                        self.phase = Phase::Expression;
                        return ObserveStatus::Event(Output::PendingBegin { event: value.event });
                    }
                }
            }
            Phase::Expression => {
                if let Some(resolve) = &mut self.resolve {
                    match resolve.tick(graph, arena) {
                        ResolveStatus::Pending => {}
                        ResolveStatus::Found { variable, .. } => {
                            assert!(
                                self.representative.replace(variable).is_none(),
                                "multiple body representatives in one causal history"
                            );
                        }
                        ResolveStatus::Done => {
                            self.resolve = None;
                            return ObserveStatus::Event(Output::ExpressionVariable {
                                variable: self
                                    .representative
                                    .take()
                                    .expect("body variable representative"),
                            });
                        }
                    }
                    return ObserveStatus::Pending;
                }
                let Some(frame) = self.frames.last_mut() else {
                    self.phase = Phase::End;
                    return ObserveStatus::Pending;
                };
                let instruction = &code.instructions[frame.instruction];
                if !frame.opened {
                    frame.opened = true;
                    return ObserveStatus::Event(match instruction {
                        Instruction::Post(atom) => Output::ExpressionRelation {
                            relation: atom.relation,
                        },
                        _ => Output::Expression {
                            operator: match instruction {
                                Instruction::And(_) => ExpressionKind::And,
                                Instruction::Or(_) => ExpressionKind::Or,
                                Instruction::Equal(..) => ExpressionKind::Equal,
                                Instruction::True => ExpressionKind::True,
                                Instruction::Fail => ExpressionKind::Fail,
                                Instruction::Post(_) => unreachable!(),
                            },
                        },
                    });
                }
                let slot = match instruction {
                    Instruction::Post(atom) => atom.args.get(frame.index).copied(),
                    Instruction::Equal(x, y) => [*x, *y].get(frame.index).copied(),
                    Instruction::And(items) | Instruction::Or(items) => {
                        if let Some(&instruction) = items.get(frame.index) {
                            frame.index += 1;
                            self.frames.push(Frame {
                                instruction,
                                index: 0,
                                opened: false,
                            });
                            return ObserveStatus::Pending;
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(slot) = slot {
                    frame.index += 1;
                    let variables = &store.descriptors
                        [&self.descriptor.expect("projected body descriptor")]
                        .variables;
                    self.resolve = Some(Resolve::new(graph, root, variables[slot], self.scope));
                } else {
                    self.frames.pop();
                    return ObserveStatus::Event(Output::ExpressionEnd);
                }
            }
            Phase::End => {
                self.frames = Vec::new();
                self.current = None;
                self.scope = Condition::FALSE;
                self.phase = Phase::Next;
                return ObserveStatus::Event(Output::PendingEnd);
            }
            Phase::Done => return ObserveStatus::Done,
        }
        ObserveStatus::Pending
    }
}
impl Trace for Projection {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.alternative, self.scope]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.resolve.as_ref()),
            _ => Step::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliased_unposted_local_is_retained_by_an_inspection_alone() {
        let ports = vec!["Y"; 128].join(",");
        let code = crate::program::prepare(
            &crate::syntax::parse_program(&format!("p(X) <=> X=Y,q({ports}).")).unwrap(),
            &crate::syntax::parse_query("p(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut found = false;
        for _ in 0..10000 {
            e.advance(1);
            if e.collector.is_some() || e.lane.is_some() {
                continue;
            }
            found = e.queue.iter().any(|s| match &s.task {
                Task::Body(b)
                    if matches!(b.phase, BodyPhase::Arguments) && b.variables.len() == 2 =>
                {
                    e.graph.index.get(
                        e.state.graph,
                        &[crate::identity::PARENT, b.variables[1], b.variables[0], 0],
                    ) == Some(Condition::TRUE)
                }
                _ => false,
            });
            if found {
                break;
            }
        }
        assert!(found);
        assert_eq!(e.snapshots().count(), 0);
        let snapshot = e.capture_snapshot().unwrap();
        e.cancel();
        for _ in 0..100000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        let view = e.start_inspection(Some(snapshot), vec![]).unwrap();
        e.release_snapshot(snapshot).unwrap();
        let mut variables = 0;
        for _ in 0..10000 {
            e.advance_inspection(view, 1).unwrap();
            if let Some(Output::ExpressionVariable { variable }) =
                e.take_inspection_output(view).unwrap()
            {
                assert_eq!(variable, 0);
                variables += 1;
            }
            if e.inspection_status(view).unwrap().done {
                break;
            }
            e.request_collection();
            e.maintain(100000);
        }
        assert!(e.inspection_status(view).unwrap().done);
        assert_eq!(variables, 128);
        e.release_inspection(view).unwrap();
        e.maintain(100000);
        assert_eq!(e.memory().pending_nodes, 0);
        assert_eq!(e.memory().graph_nodes, 0);
    }
    #[test]
    fn captures_freeze_syntax_but_certificate_roots_do_not_version_progress() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("a(A),b(A),c(A),d(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        while !e.queue.iter().any(|s| matches!(s.task, Task::Body(_))) {
            e.advance(1);
        }
        let index = e
            .queue
            .iter()
            .position(|s| matches!(s.task, Task::Body(_)))
            .unwrap();
        let mut parent = e.queue.remove(index).unwrap();
        let key = [parent.id, 0, 0, 0];
        let certificate = e.pending_root;
        let original = e
            .obligations
            .index
            .get(certificate, &key)
            .unwrap()
            .body
            .unwrap();
        // A completion certificate owns an old index version, not old syntax.
        e.ready = Some(Ready {
            epoch: e.coordinates.current(),
            transport: None,
            cursor: e
                .obligations
                .index
                .range(certificate, [0; 4], [u64::MAX; 4]),
            blocked: Condition::FALSE,
            scope: Condition::TRUE,
            job: None,
            phase: ReadyPhase::Scan,
        });
        assert!(!e.task(&mut parent)); // Dispatch.
        assert!(!e.task(&mut parent)); // Admit a, retain b,c,d.
        assert_eq!(
            e.obligations.index.get(e.pending_root, &key).unwrap().body,
            Some(original)
        );
        assert_eq!(
            e.obligations.descriptors[&original].parts[0].unwrap().start,
            1
        );
        assert_eq!(e.obligations.next_descriptor, 2); // Parent and admitted a only.
        let first = e.capture_snapshot().unwrap();
        let duplicate = e.capture_snapshot().unwrap();
        assert_eq!(
            e.snapshots[&first.0].obligations,
            e.snapshots[&duplicate.0].obligations
        );
        assert!(!e.task(&mut parent)); // Admit b; first update after capture.
        let second_id = e
            .obligations
            .index
            .get(e.pending_root, &key)
            .unwrap()
            .body
            .unwrap();
        assert_ne!(second_id, original);
        assert_eq!(
            e.obligations.descriptors[&original].parts[0].unwrap().start,
            1
        );
        assert_eq!(
            e.obligations.descriptors[&second_id].parts[0]
                .unwrap()
                .start,
            2
        );
        let second = e.capture_snapshot().unwrap();
        let epoch = e.obligations.capture_epoch;
        let clone = e.start_inspection(Some(first), vec![]).unwrap();
        assert_eq!(e.obligations.capture_epoch, epoch);
        let ephemeral = e.start_inspection(None, vec![]).unwrap();
        assert_eq!(e.obligations.capture_epoch, epoch + 1);
        assert!(!e.task(&mut parent)); // Admit c; both newer views retain c,d.
        let third_id = e
            .obligations
            .index
            .get(e.pending_root, &key)
            .unwrap()
            .body
            .unwrap();
        assert_ne!(third_id, second_id);
        assert_eq!(
            e.obligations.descriptors[&second_id].parts[0]
                .unwrap()
                .start,
            2
        );
        assert!(!e.task(&mut parent)); // Admit d in the same epoch.
        assert_eq!(
            e.obligations.index.get(e.pending_root, &key).unwrap().body,
            Some(third_id)
        );
        assert!(
            e.obligations.descriptors[&third_id]
                .parts
                .iter()
                .all(Option::is_none)
        );
        e.queue.push_back(parent);
        e.request_collection();
        e.maintain(100000);
        assert!(e.obligations.descriptors.contains_key(&original));
        assert!(e.obligations.descriptors.contains_key(&second_id));
        for id in [clone, ephemeral] {
            e.cancel_inspection(id).unwrap();
            for _ in 0..1000 {
                e.advance_inspection(id, 1).unwrap();
                e.request_collection();
                e.maintain(100000);
                if e.inspection_status(id).unwrap().done {
                    break;
                }
            }
            assert!(e.inspection_status(id).unwrap().done);
            e.release_inspection(id).unwrap();
        }
        for id in [first, duplicate, second] {
            e.release_snapshot(id).unwrap();
        }
        e.cancel();
        for _ in 0..100000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert_eq!(e.memory().obligation_descriptors, 0);
    }

    #[test]
    fn ordinary_alias_churn_keeps_descriptor_storage_bounded() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("loop(X) <=> X=Y,loop(Y).").unwrap(),
            &crate::syntax::parse_query("loop(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut peak = 0;
        for _ in 0..100 {
            e.advance(10000);
            e.request_collection();
            e.maintain(100000);
            peak = peak.max(e.memory().obligation_descriptors);
            assert_eq!(e.obligations.capture_epoch, 0);
        }
        assert!(e.applications() > 100);
        assert!(
            peak < 16,
            "retained {peak} body descriptors after maintenance"
        );
        e.cancel();
        for _ in 0..100000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert_eq!(e.memory().obligation_descriptors, 0);
    }
    fn boolean(arena: &mut Arena, operation: Operation) -> Condition {
        let mut job = arena.start(operation);
        loop {
            if let Progress::Complete(value) = job.tick(arena) {
                return value;
            }
        }
    }
    fn collect_restriction(store: &mut Obligations, arena: &mut Arena, job: &Restriction) {
        let mut conditions = vec![];
        let mut cursor = TraceCursor::default();
        loop {
            match job.trace(&mut cursor) {
                Step::Root(root) => conditions.push(root),
                Step::Pending => {}
                Step::Done => break,
            }
        }
        let mut gc = store.collect(job.roots().collect::<Vec<_>>().into_iter());
        while !gc.done() {
            if let Some(roots) = gc.tick(store) {
                conditions.extend(roots);
            }
        }
        drop(gc);
        let mut gc = arena.collect(conditions.into_iter());
        while !gc.tick(arena) {}
    }
    #[test]
    fn pending_restriction_preserves_old_roots_and_descriptors_through_every_gc_phase() {
        let mut arena = Arena::default();
        let (_, x) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let (_, z) = arena.fresh_choice();
        let xy = boolean(&mut arena, Operation::And(x, y));
        let input = boolean(&mut arena, Operation::Or(xy, z));
        let mut store = Obligations::default();
        let variables = Arc::new(vec![7, 9]);
        let parts = [
            Some(Obligation {
                event: 3,
                instruction: 2,
                start: 1,
                scope: x,
                guard: z,
            }),
            Some(Obligation {
                event: 3,
                instruction: 4,
                start: 0,
                scope: y,
                guard: input,
            }),
        ];
        let kept = store.update_descriptor(None, parts, &variables);
        let removed = store.update_descriptor(None, parts, &variables);
        let unchanged_parts = [parts[0], None];
        let unchanged = store.update_descriptor(None, unchanged_parts, &variables);
        let leaves = [
            Pending {
                scope: input,
                body: kept,
            },
            Pending {
                scope: y,
                body: unchanged,
            },
            Pending {
                scope: y.not(),
                body: removed,
            },
            Pending {
                scope: Condition::TRUE,
                body: None,
            },
            Pending {
                scope: Condition::FALSE,
                body: None,
            },
        ];
        let mut root = store.empty();
        for (i, value) in leaves.into_iter().enumerate() {
            root = store.index.insert(root, [i as u64, 0, 0, 0], value);
        }
        let epoch = store.capture_epoch;
        let mut job = store.restrict(root, Arc::new(BTreeMap::from([(yi, true)])));
        let result = (0..10000)
            .find_map(|_| {
                collect_restriction(&mut store, &mut arena, &job);
                let result = job.tick(&mut store, &mut arena);
                collect_restriction(&mut store, &mut arena, &job);
                result
            })
            .expect("finite cofactor");
        let expected = boolean(&mut arena, Operation::Or(x, z));
        assert_eq!(store.index.get(result, &[0; 4]).unwrap().scope, expected);
        let changed = store.index.get(result, &[0; 4]).unwrap().body.unwrap();
        assert_ne!(Some(changed), kept);
        let mut expected_parts = parts;
        expected_parts[1].as_mut().unwrap().scope = Condition::TRUE;
        expected_parts[1].as_mut().unwrap().guard = expected;
        assert!(store.descriptors[&changed].parts == expected_parts);
        assert_eq!(
            store.index.get(result, &[1, 0, 0, 0]).unwrap().body,
            unchanged
        );
        assert_eq!(
            store.index.get(result, &[1, 0, 0, 0]).unwrap().scope,
            Condition::TRUE
        );
        assert!(store.index.get(result, &[2, 0, 0, 0]).is_none());
        assert!(store.index.get(result, &[4, 0, 0, 0]).is_none());
        for (i, value) in leaves.into_iter().enumerate() {
            let old = store.index.get(root, &[i as u64, 0, 0, 0]).unwrap();
            assert!(old == value, "historical/certificate leaf changed");
        }
        assert_eq!(store.capture_epoch, epoch);
        assert_eq!(store.descriptor_count(), 4);
        assert!(store.descriptors[&kept.unwrap()].parts == parts);
        assert!(store.descriptors[&removed.unwrap()].parts == parts);
        assert!(store.descriptors[&unchanged.unwrap()].parts == unchanged_parts);
        for descriptor in store.descriptors.values() {
            assert!(Arc::ptr_eq(&descriptor.variables, &variables));
        }
        assert!(job.roots().any(|r| r == root));
        assert!(job.roots().any(|r| r == result));
        drop(job);
        let mut gc = store.collect(vec![result].into_iter());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        assert!(!store.descriptors.contains_key(&kept.unwrap()));
        assert!(!store.descriptors.contains_key(&removed.unwrap()));
        assert!(store.descriptors.contains_key(&unchanged.unwrap()));
        assert!(store.descriptors.contains_key(&changed));
        assert_eq!(store.descriptor_count(), 2);
    }

    #[test]
    fn pending_restriction_reuses_unaffected_root() {
        let mut store = Obligations::default();
        let mut arena = Arena::default();
        let (_, scope) = arena.fresh_choice();
        let root = store
            .index
            .insert(store.empty(), [0; 4], Pending { scope, body: None });
        let before = store.index.node_count();
        let mut job = store.restrict(root, Arc::new(BTreeMap::new()));
        let result = (0..100)
            .find_map(|_| job.tick(&mut store, &mut arena))
            .unwrap();
        assert_eq!(result, root);
        assert_eq!(store.index.node_count(), before);
    }

    #[test]
    fn pending_restriction_projects_current_and_historical_alternatives() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("left(A),right(A)").unwrap(),
        )
        .unwrap();
        let posts: Vec<_> = code
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(i, instruction)| {
                if let Instruction::Post(atom) = instruction {
                    Some((i, atom.relation))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(posts.len(), 2);
        let mut arena = Arena::default();
        let (choice, guard) = arena.fresh_choice();
        let bindings = Arc::new(BTreeMap::from([(choice, true)]));
        let mut graph = Graph::new(&code.signatures);
        let mut post = graph
            .post(graph.empty(), posts[0].1, vec![7], guard)
            .unwrap();
        let old_graph = (0..100)
            .find_map(|_| match post.tick(&mut graph) {
                crate::graph::UpdateStatus::Complete(root) => Some(root),
                _ => None,
            })
            .unwrap();
        let mut graph_job = graph.index.restrict(old_graph, bindings.clone());
        let new_graph = (0..1000)
            .find_map(|_| graph_job.tick(&mut graph.index, &mut arena))
            .unwrap();
        let mut store = Obligations::default();
        // Exercise both scope and guard: either unchanged field would wrongly
        // render the rejected expression in the current unconditional view.
        let parts = [
            Some(Obligation {
                event: 4,
                instruction: posts[0].0,
                start: 0,
                scope: guard,
                guard: Condition::TRUE,
            }),
            Some(Obligation {
                event: 4,
                instruction: posts[1].0,
                start: 0,
                scope: Condition::TRUE,
                guard: guard.not(),
            }),
        ];
        let body = store.update_descriptor(None, parts, &Arc::new(vec![7]));
        let old = store.index.insert(
            store.empty(),
            [0; 4],
            Pending {
                scope: Condition::TRUE,
                body,
            },
        );
        let mut job = store.restrict(old, bindings);
        let current = (0..1000)
            .find_map(|_| job.tick(&mut store, &mut arena))
            .unwrap();
        let reverse_bindings = Arc::new(BTreeMap::from([(choice, false)]));
        let mut reverse_graph_job = graph.index.restrict(old_graph, reverse_bindings.clone());
        let reverse_graph = (0..1000)
            .find_map(|_| reverse_graph_job.tick(&mut graph.index, &mut arena))
            .unwrap();
        let mut reverse_job = store.restrict(old, reverse_bindings);
        let reverse = (0..1000)
            .find_map(|_| reverse_job.tick(&mut store, &mut arena))
            .unwrap();
        // Retain all published versions through descriptor collection.
        let mut gc = store.collect(vec![old, current, reverse].into_iter());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        for (pending, root, alternative, relation) in [
            (current, new_graph, Condition::TRUE, posts[0].1),
            (reverse, reverse_graph, Condition::TRUE, posts[1].1),
            (old, old_graph, guard, posts[0].1),
            (old, old_graph, guard.not(), posts[1].1),
        ] {
            let mut projection = Projection::new(&store, pending, alternative);
            let mut events = vec![];
            let mut done = false;
            for _ in 0..1000 {
                match projection.tick(&store, &graph, root, &mut arena, &code) {
                    ObserveStatus::Event(event) => events.push(event),
                    ObserveStatus::Done => {
                        done = true;
                        break;
                    }
                    ObserveStatus::Pending => {}
                }
            }
            assert!(done);
            assert_eq!(
                events,
                vec![
                    Output::PendingBegin { event: 4 },
                    Output::ExpressionRelation { relation },
                    Output::ExpressionVariable { variable: 7 },
                    Output::ExpressionEnd,
                    Output::PendingEnd,
                ]
            );
        }
    }

    #[test]
    fn pending_restriction_drains_last_assignment_owner_incrementally() {
        let mut arena = Arena::default();
        let assignments = Arc::new(
            (0..2048)
                .map(|_| {
                    let (id, _) = arena.fresh_choice();
                    (id, true)
                })
                .collect(),
        );
        let mut store = Obligations::default();
        let root = store.index.insert(
            store.empty(),
            [0; 4],
            Pending {
                scope: Condition::TRUE,
                body: None,
            },
        );
        let mut job = store.restrict(root, assignments);
        for _ in 0..2048 {
            assert!(job.tick(&mut store, &mut arena).is_none());
        }
        assert_eq!(
            (0..100).find_map(|_| job.tick(&mut store, &mut arena)),
            Some(root)
        );
    }
}
