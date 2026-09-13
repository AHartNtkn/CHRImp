//! Select original constructor-led arms on already described support. No field
//! substitution or post fusion: each selected arm still executes its source body.
use super::*;
use crate::gc::discard_slot;
use crate::identity::{Resolve, ResolveStatus};
use crate::program::constructors::ConstructorChoice;
use crate::store;
use crate::trace::{Cursor as TraceCursor, Step, Trace};

#[derive(Clone, Copy)]
enum Phase {
    Resolve,
    Scan,
    Hit,
    Uncovered,
    Accumulate,
    Failure,
    FailureRemaining,
    Admit,
    Retire,
    Generative,
    Done,
}
pub(super) struct Dispatch {
    root: Root,
    plan: Arc<ConstructorChoice>,
    resolve: Option<Resolve>,
    cursor: Option<store::Cursor>,
    partition: Condition,
    uncovered: Condition,
    remaining: Condition,
    active: Condition,
    failed: Condition,
    hit: Condition,
    occurrence: u64,
    arm: Option<usize>,
    selected: BTreeMap<u64, Condition>,
    job: Option<Job>,
    phase: Phase,
}
enum Status {
    Pending,
    Active(Condition, Condition),
    Arm(usize, Condition),
    Generative(Condition),
    Done,
}
impl Dispatch {
    pub fn new(
        g: &Graph,
        root: Root,
        key: u64,
        scope: Condition,
        plan: Arc<ConstructorChoice>,
    ) -> Self {
        Self {
            resolve: Some(Resolve::new(g, root.clone(), key, scope)),
            root,
            plan,
            cursor: None,
            partition: Condition::FALSE,
            uncovered: scope,
            remaining: scope,
            active: Condition::FALSE,
            failed: Condition::FALSE,
            hit: Condition::FALSE,
            occurrence: 0,
            arm: None,
            selected: BTreeMap::new(),
            job: None,
            phase: Phase::Resolve,
        }
    }
    pub fn pending_scope(&self) -> Condition {
        self.remaining
    }
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    fn tick(&mut self, g: &Graph, a: &mut Arena, active: Condition) -> Status {
        match self.phase {
            Phase::Resolve => match self.resolve.as_mut().unwrap().tick(g, a) {
                ResolveStatus::Pending => {}
                ResolveStatus::Found { variable, support } => {
                    self.partition = support;
                    self.cursor = Some(g.constructor_attachments(
                        self.root.clone(),
                        variable,
                        self.plan.family as u64,
                    ));
                    self.phase = Phase::Scan;
                }
                ResolveStatus::Done => {
                    self.resolve = None;
                    self.job = Some(a.start(Operation::Difference(active, self.failed)));
                    self.phase = Phase::Failure;
                }
            },
            Phase::Scan => {
                if let Some((key, support)) = self.cursor.as_mut().unwrap().next(&g.index) {
                    self.occurrence = key[3];
                    self.job = Some(a.start(Operation::And(self.partition, support)));
                    self.phase = Phase::Hit;
                } else {
                    self.cursor = None;
                    self.phase = Phase::Resolve;
                }
            }
            Phase::Hit => {
                if let Some(hit) = poll(&mut self.job, a) {
                    self.hit = hit;
                    if hit == Condition::FALSE {
                        self.phase = Phase::Scan;
                    } else {
                        let relation = g
                            .fact(self.root.clone(), self.occurrence)
                            .expect("live normalized attachment")
                            .relation;
                        self.arm = self.plan.arms.get(&relation).copied();
                        self.job = Some(a.start(Operation::Difference(self.uncovered, hit)));
                        self.phase = Phase::Uncovered;
                    }
                }
            }
            Phase::Uncovered => {
                if let Some(rest) = poll(&mut self.job, a) {
                    self.uncovered = rest;
                    let old = self.arm.map_or(self.failed, |arm| {
                        self.selected
                            .get(&(arm as u64))
                            .copied()
                            .unwrap_or(Condition::FALSE)
                    });
                    self.job = Some(a.start(Operation::Or(old, self.hit)));
                    self.phase = Phase::Accumulate;
                }
            }
            Phase::Accumulate => {
                if let Some(support) = poll(&mut self.job, a) {
                    if let Some(arm) = self.arm {
                        self.selected.insert(arm as u64, support);
                    } else {
                        self.failed = support;
                    }
                    self.phase = Phase::Scan;
                }
            }
            Phase::Failure => {
                if let Some(active) = poll(&mut self.job, a) {
                    self.active = active;
                    self.job = Some(a.start(Operation::Difference(self.remaining, self.failed)));
                    self.phase = Phase::FailureRemaining;
                }
            }
            Phase::FailureRemaining => {
                if let Some(scope) = poll(&mut self.job, a) {
                    self.remaining = scope;
                    self.phase = Phase::Admit;
                    return Status::Active(self.active, self.failed);
                }
            }
            Phase::Admit => {
                if let Some((arm, support)) = self.selected.pop_first() {
                    self.arm = Some(arm as usize);
                    self.hit = support;
                    self.job = Some(a.start(Operation::Difference(self.remaining, support)));
                    self.phase = Phase::Retire;
                } else {
                    self.phase = Phase::Generative;
                }
            }
            Phase::Retire => {
                if let Some(scope) = poll(&mut self.job, a) {
                    self.remaining = scope;
                    self.phase = Phase::Admit;
                    return Status::Arm(self.arm.unwrap(), self.hit);
                }
            }
            Phase::Generative => {
                self.phase = Phase::Done;
                self.remaining = Condition::FALSE;
                if self.uncovered != Condition::FALSE {
                    return Status::Generative(self.uncovered);
                }
            }
            Phase::Done => return Status::Done,
        }
        Status::Pending
    }
    pub fn discard_tick(&mut self) -> bool {
        if discard_slot(&mut self.job, |child| child.discard_tick()) {
            return false;
        }
        if discard_slot(&mut self.resolve, |child| child.discard_tick()) {
            return false;
        }
        self.selected.pop_first().is_none()
    }
}
impl Trace for Dispatch {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[
                self.partition,
                self.uncovered,
                self.failed,
                self.hit,
                self.remaining,
                self.active,
            ]),
            1 => c.optional(self.resolve.as_ref()),
            2 => c.optional(self.job.as_ref()),
            3 => c.values(&self.selected),
            _ => Step::Done,
        }
    }
}
impl Engine {
    pub(super) fn dispatch_tick(&mut self, id: u64, b: &mut Body) -> bool {
        // The original body obligation and mutation lane remain owned until all
        // supported children are admitted. No partial completion escapes here.
        match b
            .dispatch
            .as_mut()
            .unwrap()
            .tick(&self.graph, &mut self.arena, self.active)
        {
            Status::Pending => {}
            Status::Active(active, failed) => {
                self.semantic_regions |= self.active != active;
                self.active = active;
                if failed != Condition::FALSE {
                    self.sync_obligation(id, b);
                    self.record(SnapshotKind::Failure, failed);
                }
            }
            Status::Arm(instruction, scope) => {
                self.normalization_stats.known_arm_admissions += 1;
                self.body(b.event, instruction, b.variables.clone(), scope);
            }
            Status::Generative(scope) => {
                self.normalization_stats.generative_dispatches += 1;
                let mut child = Body::new(b.event, b.instruction, b.variables.clone(), scope);
                child.dispatch_checked = true;
                self.spawn(scope, Task::Body(Box::new(child)));
            }
            Status::Done => {
                b.dispatch = None;
                self.finish_body_record(id);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_dispatch_phase_keeps_its_owner_through_collection_and_cancel() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../../examples/behavior-synthesis.chrnb")).unwrap();
        let mut p: crate::syntax::Program = serde_json::from_value(doc["program"].clone()).unwrap();
        p.rules.extend(crate::syntax::parse_program("start(R) <=> (app(R,X,Y),route(R);k(R),route(R);nil(R),route(R);mark(R),route(R)). route(R) <=> (app(R,A,B),seen(A,B);k(R),leaf(R)).").unwrap().rules);
        let code = Arc::new(
            crate::program::prepare(&p, &crate::syntax::parse_query("start(R)").unwrap()).unwrap(),
        );
        for phase in 0..11 {
            for cancel in [false, true] {
                let mut e = Engine::new(code.clone());
                let mut answers = 0;
                let mut found = false;
                for _ in 0..200_000 {
                    if e.queue.iter().any(|s| matches!(&s.task,Task::Body(b) if b.dispatch.as_ref().is_some_and(|d| d.phase as u8==phase))) {
                        found=true;break;
                    }
                    e.advance(1);
                    if matches!(e.take_output(), Some(Output::End)) {
                        answers += 1;
                    }
                }
                assert!(found, "phase {phase}");
                let applications = e.applications();
                let dispatched = e.normalization_stats().conditional_dispatches;
                e.request_collection();
                e.maintain(1);
                assert!(e.collecting());
                if cancel {
                    e.cancel();
                }
                for _ in 0..200_000 {
                    if !e.collecting() {
                        break;
                    }
                    e.maintain(1);
                }
                assert!(!e.collecting());
                assert_eq!(e.applications(), applications);
                assert_eq!(e.normalization_stats().conditional_dispatches, dispatched);
                for _ in 0..200_000 {
                    e.advance(1);
                    if matches!(e.take_output(), Some(Output::End)) {
                        answers += 1;
                    }
                    if if cancel {
                        e.cancel_done()
                    } else {
                        e.delivery_done()
                    } {
                        break;
                    }
                }
                if cancel {
                    assert!(e.cancel_done(), "phase {phase}");
                    assert_eq!(e.applications(), applications);
                    assert_eq!(e.normalization_stats().conditional_dispatches, dispatched);
                    let m = e.memory();
                    assert_eq!(
                        (
                            m.graph_nodes,
                            m.conditions,
                            m.pending_nodes,
                            m.occurrences,
                            m.choices
                        ),
                        (0, 0, 0, 0, 0)
                    );
                } else {
                    assert!(e.delivery_done(), "phase {phase}");
                    assert_eq!(answers, 4, "phase {phase}");
                }
            }
        }
    }
}
