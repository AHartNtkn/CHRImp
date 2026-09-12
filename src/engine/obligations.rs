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
        Some(id)
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
}
