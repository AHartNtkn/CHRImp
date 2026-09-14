//! Prove terminal failure before constructing a constructor choice's supports.
//! The mutation lane pins the context through admission. Each rejected arm has
//! a finite source witness: its original leading post then a terminal consumer.
//! No consumer is actually consumed on surviving support.
use super::*;
use crate::gc::discard_slot;
use crate::graph::Occurrences;
use crate::identity::Equal;
use crate::program::constructors::{ConstructorChoice, ConsumerValue};
use crate::trace::{Cursor as TraceCursor, Step, Trace};

#[derive(Clone, Copy)]
enum Phase {
    Arm,
    Guard,
    Scan,
    Hit,
    Test,
    Equal,
    Exclude,
    Accumulate,
    Total,
    Failed,
    Active,
    Overlap,
    Chosen,
    LeftOnly,
    Left,
    Right,
    Done,
}
pub(super) struct Split {
    root: Root,
    plan: Arc<ConstructorChoice>,
    code: Arc<Prepared>,
    variables: Arc<Vec<u64>>,
    instruction: usize,
    arm: usize,
    split: usize,
    end: usize,
    guard: usize,
    test: usize,
    occurrence: u64,
    cursor: Option<Occurrences>,
    equal: Option<Equal>,
    job: Option<Job>,
    phase: Phase,
    scope: Condition,
    active: Condition,
    viable: Condition,
    hit: Condition,
    left: Condition,
    right: Condition,
    total: Condition,
    failed: Condition,
    overlap: Condition,
    chosen: Condition,
    decision: Condition,
    birth: Option<u64>,
}
pub(super) struct Result {
    pub left: Condition,
    pub right: Condition,
    pub total: Condition,
    pub failed: Condition,
    pub active: Condition,
    pub birth: Option<(u64, Condition, Condition)>,
}
impl Split {
    pub fn new(
        root: Root,
        code: Arc<Prepared>,
        plan: Arc<ConstructorChoice>,
        b: &Body,
        active: Condition,
    ) -> Self {
        let Instruction::Or(items) = &code.instructions[b.instruction] else {
            unreachable!()
        };
        let items = code.operands(*items);
        let end = b.end.unwrap_or(items.len());
        Self {
            root,
            plan,
            code,
            variables: b.variables.clone(),
            instruction: b.instruction,
            arm: b.index,
            split: b.index + (end - b.index) / 2,
            end,
            guard: 0,
            test: 0,
            occurrence: 0,
            cursor: None,
            equal: None,
            job: None,
            phase: Phase::Arm,
            scope: b.scope,
            active,
            viable: Condition::FALSE,
            hit: Condition::FALSE,
            left: Condition::FALSE,
            right: Condition::FALSE,
            total: Condition::FALSE,
            failed: Condition::FALSE,
            overlap: Condition::FALSE,
            chosen: Condition::FALSE,
            decision: Condition::FALSE,
            birth: None,
        }
    }
    pub fn root(&self) -> Root {
        self.root.clone()
    }
    fn consumers(&self) -> &[crate::program::constructors::FailureConsumer] {
        let Instruction::Or(items) = &self.code.instructions[self.instruction] else {
            unreachable!()
        };
        let items = self.code.operands(*items);
        self.plan
            .rejection
            .get(&items[self.arm])
            .map_or(&[], Vec::as_slice)
    }
    pub fn tick(&mut self, g: &Graph, a: &mut Arena) -> Option<Result> {
        match self.phase {
            Phase::Arm => {
                if self.arm == self.end {
                    self.job = Some(a.start(Operation::Or(self.left, self.right)));
                    self.phase = Phase::Total;
                } else {
                    self.viable = self.scope;
                    self.guard = 0;
                    self.phase = Phase::Guard;
                }
            }
            Phase::Guard => {
                if self.viable == Condition::FALSE || self.guard == self.consumers().len() {
                    let old = if self.arm < self.split {
                        self.left
                    } else {
                        self.right
                    };
                    self.job = Some(a.start(Operation::Or(old, self.viable)));
                    self.phase = Phase::Accumulate;
                } else {
                    let consumer = &self.consumers()[self.guard];
                    debug_assert!(matches!(
                        self.code.instructions[self.code.rules[consumer.rule].body],
                        Instruction::Fail
                    ));
                    self.cursor = Some(
                        g.relation(self.root.clone(), consumer.relation)
                            .expect("prepared consumer"),
                    );
                    self.phase = Phase::Scan;
                }
            }
            Phase::Scan => {
                if let Some((occurrence, support)) = self.cursor.as_mut().unwrap().next(g) {
                    self.occurrence = occurrence;
                    self.job = Some(a.start(Operation::And(self.viable, support)));
                    self.phase = Phase::Hit;
                } else {
                    self.cursor = None;
                    self.guard += 1;
                    self.phase = Phase::Guard;
                }
            }
            Phase::Hit => {
                if let Some(hit) = poll(&mut self.job, a) {
                    self.hit = hit;
                    self.test = 0;
                    self.phase = Phase::Test;
                }
            }
            Phase::Test => {
                if self.hit == Condition::FALSE {
                    self.phase = Phase::Scan;
                } else if self.test == self.consumers()[self.guard].tests.len() {
                    self.job = Some(a.start(Operation::Difference(self.viable, self.hit)));
                    self.phase = Phase::Exclude;
                } else {
                    let (port, value) = &self.consumers()[self.guard].tests[self.test];
                    let fact = g
                        .fact(self.root.clone(), self.occurrence)
                        .expect("pinned consumer");
                    let x = match value {
                        ConsumerValue::Body(slot) => self.variables[*slot],
                        ConsumerValue::Port(p) => fact.args[*p],
                    };
                    self.equal = Some(Equal::new(
                        g,
                        self.root.clone(),
                        x,
                        fact.args[*port],
                        self.hit,
                    ));
                    self.phase = Phase::Equal;
                }
            }
            Phase::Equal => {
                if let Some(hit) = self.equal.as_mut().unwrap().tick(g, a) {
                    self.hit = hit;
                    self.equal = None;
                    self.test += 1;
                    self.phase = Phase::Test;
                }
            }
            Phase::Exclude => {
                if let Some(viable) = poll(&mut self.job, a) {
                    self.viable = viable;
                    if viable == Condition::FALSE {
                        self.cursor = None;
                        self.phase = Phase::Guard;
                    } else {
                        self.phase = Phase::Scan;
                    }
                }
            }
            Phase::Accumulate => {
                if let Some(support) = poll(&mut self.job, a) {
                    if self.arm < self.split {
                        self.left = support;
                    } else {
                        self.right = support;
                    }
                    self.arm += 1;
                    self.phase = Phase::Arm;
                }
            }
            Phase::Total => {
                if let Some(total) = poll(&mut self.job, a) {
                    self.total = total;
                    self.job = Some(a.start(Operation::Difference(self.scope, total)));
                    self.phase = Phase::Failed;
                }
            }
            Phase::Failed => {
                if let Some(failed) = poll(&mut self.job, a) {
                    self.failed = failed;
                    self.job = Some(a.start(Operation::Difference(self.active, failed)));
                    self.phase = Phase::Active;
                }
            }
            Phase::Active => {
                if let Some(active) = poll(&mut self.job, a) {
                    self.active = active;
                    self.job = Some(a.start(Operation::And(self.left, self.right)));
                    self.phase = Phase::Overlap;
                }
            }
            // L and R describe viability, not additional language choices.
            // Only L∩R needs a fresh source decision. Route L\R directly;
            // the right scope is (L∪R) minus the admitted left scope. This
            // preserves multiplicity without inventing choices on single-arm
            // support, including support where neither arm survives.
            Phase::Overlap => {
                if let Some(overlap) = poll(&mut self.job, a) {
                    self.overlap = overlap;
                    if overlap != Condition::FALSE {
                        let (id, decision) = a.fresh_scoped_choice(overlap);
                        self.birth = Some(id);
                        self.decision = decision;
                    }
                    self.job = Some(a.start(Operation::And(overlap, self.decision)));
                    self.phase = Phase::Chosen;
                }
            }
            Phase::Chosen => {
                if let Some(chosen) = poll(&mut self.job, a) {
                    self.chosen = chosen;
                    self.job = Some(a.start(Operation::Difference(self.left, self.right)));
                    self.phase = Phase::LeftOnly;
                }
            }
            Phase::LeftOnly => {
                if let Some(only) = poll(&mut self.job, a) {
                    self.job = Some(a.start(Operation::Or(only, self.chosen)));
                    self.phase = Phase::Left;
                }
            }
            Phase::Left => {
                if let Some(left) = poll(&mut self.job, a) {
                    self.left = left;
                    self.job = Some(a.start(Operation::Difference(self.total, left)));
                    self.phase = Phase::Right;
                }
            }
            Phase::Right => {
                if let Some(right) = poll(&mut self.job, a) {
                    self.right = right;
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => {
                return Some(Result {
                    left: self.left,
                    right: self.right,
                    total: self.total,
                    failed: self.failed,
                    active: self.active,
                    birth: self.birth.map(|id| (id, self.decision, self.overlap)),
                });
            }
        }
        None
    }
    pub fn discard_tick(&mut self) -> bool {
        if discard_slot(&mut self.job, |child| child.discard_tick()) {
            return false;
        }
        if discard_slot(&mut self.equal, |child| child.discard_tick()) {
            return false;
        }
        true
    }
}
impl Trace for Split {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[
                self.scope,
                self.active,
                self.viable,
                self.hit,
                self.left,
                self.right,
                self.total,
                self.failed,
                self.overlap,
                self.chosen,
                self.decision,
            ]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.equal.as_ref()),
            _ => Step::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn engine() -> Engine {
        let program = crate::syntax::parse_program("copper(X,Y) \\ copper(X,Z) <=> Y=Z. tide(X) \\ tide(X) <=> true. copper(X,Y),tide(X) <=> fail. copper(X,Y),crane(X,Y,Z,Z) <=> fail. route(X,Y) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).").unwrap();
        let query = crate::syntax::parse_query("crane(A,B,C,D),(C=D;true),route(A,B)").unwrap();
        Engine::new(Arc::new(crate::program::prepare(&program, &query).unwrap()))
    }
    #[test]
    fn every_rejection_phase_survives_collection_and_bounded_cancellation() {
        for phase in 0..=Phase::Done as u8 {
            for cancel in [false, true] {
                let mut e = engine();
                let mut outputs = 0;
                let mut found = false;
                for _ in 0..200_000 {
                    if e.queue.iter().any(|s| matches!(&s.task, Task::Body(b) if b.rejection.as_ref().is_some_and(|r| r.phase as u8 == phase))) {
                        found = true; break;
                    }
                    e.advance(1);
                    if matches!(e.take_output(), Some(Output::End)) {
                        outputs += 1;
                    }
                }
                assert!(found, "unexercised rejection phase {phase}");
                e.request_collection();
                e.maintain(1);
                assert!(e.collecting());
                let applications = e.applications();
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
                for _ in 0..200_000 {
                    e.advance(1);
                    if matches!(e.take_output(), Some(Output::End)) {
                        outputs += 1;
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
                    assert_eq!(outputs, 3, "phase {phase}");
                }
            }
        }
    }
    fn project(e: &mut Engine, snapshot: ViewId, selection: Vec<(u64, bool)>) -> Vec<Output> {
        let id = e.start_inspection(Some(snapshot), selection).unwrap();
        let mut out = vec![];
        for _ in 0..200_000 {
            e.advance_inspection(id, 1).unwrap();
            if let Some(mut event) = e.take_inspection_output(id).unwrap() {
                if let Output::Begin { completion, .. } = &mut event {
                    *completion = 0;
                }
                out.push(event);
            }
            if e.inspection_status(id).unwrap().done {
                e.release_inspection(id).unwrap();
                return out;
            }
        }
        panic!("snapshot projection did not complete")
    }
    #[test]
    fn split_snapshots_preserve_exact_pending_arms_after_progress_and_cancel() {
        for phase in [BodyPhase::GuardLeft, BodyPhase::GuardRight] {
            let mut e = engine();
            let mut found = false;
            for _ in 0..200_000 {
                if e.queue
                    .iter()
                    .any(|s| matches!(&s.task, Task::Body(b) if b.phase as u8 == phase as u8))
                {
                    found = true;
                    break;
                }
                e.advance(1);
            }
            assert!(found);
            e.maintain(200_000);
            let snapshot = e.capture_snapshot().unwrap();
            let expected = project(&mut e, snapshot, vec![]);
            let names: Vec<_> = expected
                .iter()
                .filter_map(|o| match o {
                    Output::ExpressionRelation { relation } => {
                        Some(e.program().signatures()[*relation].name.as_str())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(names.iter().filter(|&&n| n == "copper").count(), 1);
            assert_eq!(names.iter().filter(|&&n| n == "tide").count(), 2);
            assert_eq!(names.iter().filter(|&&n| n == "velvet").count(), 1);
            assert_eq!(names.iter().filter(|&&n| n == "orbit").count(), 2);
            assert_eq!(
                expected.iter().filter(|o| matches!(o, Output::End)).count(),
                3
            );
            let birth = *e.choices().last().unwrap().0;
            let selected = project(&mut e, snapshot, vec![(birth, true)]);
            assert_eq!(
                selected.iter().filter(|o| matches!(o, Output::End)).count(),
                1
            );
            for _ in 0..200_000 {
                e.advance(1);
                e.take_output();
                if e.delivery_done() {
                    break;
                }
            }
            assert!(e.delivery_done());
            e.cancel();
            for _ in 0..200_000 {
                e.advance(1);
                if e.cancel_done() {
                    break;
                }
            }
            assert!(e.cancel_done());
            assert_eq!(project(&mut e, snapshot, vec![]), expected);
            assert_eq!(project(&mut e, snapshot, vec![(birth, true)]), selected);
            e.release_snapshot(snapshot).unwrap();
            e.maintain(200_000);
            assert_eq!(e.memory().pending_nodes, 0);
            assert_eq!(e.memory().obligation_descriptors, 0);
        }
    }
}
