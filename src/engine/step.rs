//! Logical rule stepping, with budgeted choice selection and commit checks.
use super::*;
use crate::trace::{Cursor as TraceCursor, Step as TraceStep, Trace};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StepStatus {
    /// Idle, or the current logical step has finished and source is paused.
    pub done: bool,
    pub event: Option<u64>,
    pub rule: Option<usize>,
    pub shared: Option<bool>,
}
impl Default for StepStatus {
    fn default() -> Self {
        Self {
            done: true,
            event: None,
            rule: None,
            shared: None,
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Select,
    Support,
    Decision,
    Run,
    Overlap,
    Shared,
    Cleanup,
    Done,
}
#[derive(Clone, Copy)]
enum ProbePhase {
    Scan,
    Active,
    Overlap,
}
struct Probe {
    // Every newly admitted obligation stays within its parent's scope. Thus
    // this frozen pending union conservatively covers future work too. An
    // empty intersection with selected active scope certifies no application
    // remains, independently of primary-output backpressure. This certificate
    // does not publish a completion or alter source scheduling.
    cursor: Cursor,
    blocked: Condition,
    job: Option<Job>,
    phase: ProbePhase,
}
pub(super) struct RuleStep {
    selections: VecDeque<(u64, bool)>,
    scope: Condition,
    decision: Condition,
    application: Option<(u64, usize, Condition)>,
    job: Option<Job>,
    probe: Option<Probe>,
    phase: Phase,
    source_turn: bool,
    resume: bool,
    status: StepStatus,
}
pub(super) enum Gate {
    Run,
    Yield,
    Stop,
}
impl RuleStep {
    pub(super) fn pending_root(&self) -> Option<PendingRoot> {
        self.probe.as_ref().map(|p| p.cursor.root())
    }
}
impl Trace for Probe {
    fn trace(&self, c: &mut TraceCursor) -> TraceStep {
        match c.phase {
            0 => c.fields(&[self.blocked]),
            1 => c.optional(self.job.as_ref()),
            _ => TraceStep::Done,
        }
    }
}
impl Trace for RuleStep {
    fn trace(&self, c: &mut TraceCursor) -> TraceStep {
        match c.phase {
            0 => c.fields(&[
                self.scope,
                self.decision,
                self.application.map_or(Condition::FALSE, |a| a.2),
            ]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.probe.as_ref()),
            _ => TraceStep::Done,
        }
    }
}
impl Engine {
    /// The controller owns a selection through execution and the paused Done
    /// state, when the UI may still inspect its choice IDs. Keep the prefix
    /// pinned until resume/cancellation drains the owner or a new step takes it.
    pub(super) fn step_coordinate_cutoff(&self) -> Option<u64> {
        self.rule_step
            .as_ref()
            .and_then(|_| self.births.last_key_value().map(|(&id, _)| id))
    }
    /// Select existing explicit choices and pause after one overlapping commit.
    /// The request does not execute source work or run a Boolean operation.
    /// A completed step remains paused until another request or `resume`.
    pub fn request_step(&mut self, choices: Vec<(u64, bool)>) -> Result<(), InspectionError> {
        if self.canceled() {
            return Err(InspectionError::Canceled);
        }
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        if self.rule_step.as_ref().is_some_and(|s| !s.status.done) {
            return Err(InspectionError::InProgress);
        }
        if choices.iter().any(|(id, _)| !self.births.contains_key(id)) {
            return Err(InspectionError::UnknownChoice);
        }
        self.rule_step = Some(RuleStep {
            selections: choices.into(),
            scope: self.active,
            decision: Condition::FALSE,
            application: None,
            job: None,
            probe: None,
            phase: Phase::Select,
            source_turn: false,
            resume: false,
            status: StepStatus {
                done: false,
                ..StepStatus::default()
            },
        });
        Ok(())
    }
    pub fn step_status(&self) -> StepStatus {
        self.rule_step
            .as_ref()
            .map_or_else(StepStatus::default, |s| s.status)
    }
    /// Cancel the stepping constraint and resume ordinary execution. Scratch is
    /// discarded incrementally by `advance` before source execution resumes.
    pub fn resume(&mut self) -> Result<(), InspectionError> {
        if self.canceled() {
            return Err(InspectionError::Canceled);
        }
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        if let Some(step) = &mut self.rule_step {
            step.resume = true;
            step.phase = Phase::Cleanup;
            step.status.done = false;
        }
        Ok(())
    }
    // Shared by every source cancellation entry point, including inspection
    // advancement. The controller stays traced until its jobs finish discard.
    pub(super) fn discard_step_tick(&mut self) -> bool {
        let Some(step) = &mut self.rule_step else {
            return true;
        };
        step.resume = true;
        step.phase = Phase::Cleanup;
        self.step_gate();
        self.rule_step.is_none()
    }
    pub(super) fn step_application(&mut self, event: u64, rule: usize, support: Condition) {
        if let Some(step) = &mut self.rule_step {
            debug_assert!(matches!(step.phase, Phase::Run));
            step.application = Some((event, rule, support));
            step.job = Some(self.arena.start(Operation::And(support, step.scope)));
            step.phase = Phase::Overlap;
        }
    }
    // Called before each source tick. Selection and post-commit checks freeze
    // source execution. The no-application certificate alternates with source
    // work, so neither an expensive BDD nor a diverging sibling starves it.
    pub(super) fn step_gate(&mut self) -> Gate {
        let Some(mut step) = self.rule_step.take() else {
            return Gate::Run;
        };
        let mut gate = Gate::Yield;
        match step.phase {
            Phase::Select => {
                if let Some((id, positive)) = step.selections.pop_front() {
                    let birth = &self.births[&id];
                    step.decision = if positive {
                        birth.decision
                    } else {
                        birth.decision.not()
                    };
                    step.job = Some(self.arena.start(Operation::And(step.scope, birth.support)));
                    step.phase = Phase::Support;
                } else {
                    step.selections = VecDeque::new();
                    step.phase = if step.scope == Condition::FALSE {
                        Phase::Cleanup
                    } else {
                        Phase::Run
                    };
                }
            }
            Phase::Support => {
                if let Some(c) = poll(&mut step.job, &mut self.arena) {
                    step.job = Some(self.arena.start(Operation::And(c, step.decision)));
                    step.phase = Phase::Decision;
                }
            }
            Phase::Decision => {
                if let Some(c) = poll(&mut step.job, &mut self.arena) {
                    step.scope = c;
                    step.decision = Condition::FALSE;
                    step.phase = Phase::Select;
                }
            }
            Phase::Run => {
                step.source_turn = !step.source_turn;
                if step.source_turn {
                    gate = Gate::Run;
                } else if let Some(probe) = &mut step.probe {
                    match probe.phase {
                        ProbePhase::Scan => {
                            if probe.job.is_some() {
                                if let Some(c) = poll(&mut probe.job, &mut self.arena) {
                                    probe.blocked = c;
                                }
                            } else if let Some((_, c)) = probe.cursor.next(&self.obligations.index)
                            {
                                probe.job =
                                    Some(self.arena.start(Operation::Or(probe.blocked, c.scope)));
                            } else {
                                probe.job =
                                    Some(self.arena.start(Operation::And(step.scope, self.active)));
                                probe.phase = ProbePhase::Active;
                            }
                        }
                        ProbePhase::Active => {
                            if let Some(c) = poll(&mut probe.job, &mut self.arena) {
                                probe.job =
                                    Some(self.arena.start(Operation::And(c, probe.blocked)));
                                probe.phase = ProbePhase::Overlap;
                            }
                        }
                        ProbePhase::Overlap => {
                            if let Some(c) = poll(&mut probe.job, &mut self.arena) {
                                step.probe = None;
                                if c == Condition::FALSE {
                                    step.phase = Phase::Cleanup;
                                }
                            }
                        }
                    }
                } else {
                    step.probe = Some(Probe {
                        cursor: self.obligations.index.range(
                            self.pending_root.clone(),
                            [0; 4],
                            [u64::MAX; 4],
                        ),
                        blocked: Condition::FALSE,
                        job: None,
                        phase: ProbePhase::Scan,
                    });
                }
            }
            Phase::Overlap => {
                if let Some(c) = poll(&mut step.job, &mut self.arena) {
                    if c == Condition::FALSE {
                        step.application = None;
                        step.phase = Phase::Run;
                    } else {
                        let (event, rule, support) = step.application.unwrap();
                        step.status.event = Some(event);
                        step.status.rule = Some(rule);
                        step.job =
                            Some(self.arena.start(Operation::Difference(support, step.scope)));
                        step.phase = Phase::Shared;
                    }
                }
            }
            Phase::Shared => {
                if let Some(c) = poll(&mut step.job, &mut self.arena) {
                    step.status.shared = Some(c != Condition::FALSE);
                    step.phase = Phase::Cleanup;
                }
            }
            Phase::Cleanup => {
                if let Some(job) = &mut step.job {
                    if job.discard_tick() {
                        step.job = None;
                    }
                } else if let Some(job) = step.probe.as_mut().and_then(|p| p.job.as_mut()) {
                    if job.discard_tick() {
                        step.probe.as_mut().unwrap().job = None;
                    }
                } else if step.selections.pop_front().is_none() {
                    step.probe = None;
                    step.scope = Condition::FALSE;
                    step.decision = Condition::FALSE;
                    step.application = None;
                    step.selections = VecDeque::new();
                    step.phase = Phase::Done;
                    step.status.done = true;
                    if step.resume {
                        return Gate::Yield;
                    }
                }
            }
            Phase::Done => gate = Gate::Stop,
        }
        self.rule_step = Some(step);
        gate
    }
}

#[cfg(test)]
mod coordinate_tests {
    use super::*;
    #[test]
    fn step_pins_coordinates_through_done_until_selection_ownership_ends() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        e.request_step(vec![]).unwrap();
        for _ in 0..2 {
            let (id, decision) = e.arena.fresh_choice();
            e.births.insert(
                id,
                Birth {
                    event: 0,
                    instruction: 0,
                    start: 0,
                    split: 1,
                    end: 2,
                    support: Condition::TRUE,
                    decision,
                },
            );
            assert_eq!(e.step_coordinate_cutoff(), Some(id));
        }
        e.rule_step.as_mut().unwrap().phase = Phase::Cleanup;
        for _ in 0..100 {
            e.step_gate();
            if e.rule_step.as_ref().unwrap().status.done {
                break;
            }
        }
        assert!(e.rule_step.as_ref().unwrap().status.done);
        assert_eq!(
            e.step_coordinate_cutoff(),
            e.births.last_key_value().map(|(&id, _)| id)
        );
        e.resume().unwrap();
        for _ in 0..100 {
            e.step_gate();
            if e.rule_step.is_none() {
                break;
            }
        }
        assert!(e.rule_step.is_none());
        assert_eq!(e.step_coordinate_cutoff(), None);
    }
}
