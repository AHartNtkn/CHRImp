//! Forget coordinates determined by older surviving choices wherever their birth occurs. The mutation
//! lane freezes publication while each supported index is rebuilt persistently.
use super::*;
use crate::condition::Transform;
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone, Copy)]
enum Phase {
    Inspections,
    Choices,
    Global,
    BirthSupport,
    Born,
    Positive,
    Negative,
    ProjectPositive,
    ProjectNegative,
    Disjoint,
    Reduce,
    Graph,
    History,
    Pending,
    Tasks,
    Parked,
    Births,
    Publish,
}
pub(super) struct Compact {
    phase: Phase,
    pin: Option<u64>,
    after: Option<u64>,
    choice: u64,
    image: Condition,
    positive_scope: Condition,
    bindings: Arc<BTreeMap<u64, Condition>>,
    active: Condition,
    born: Condition,
    boolean: Option<Job>,
    transform: Option<Transform>,
    index: Option<crate::store::Substitution>,
    pending: Option<obligations::Substitution>,
    state: StateRoot,
    pending_root: PendingRoot,
    task: usize,
    slot: usize,
}
impl Compact {
    pub(super) fn new(e: &Engine) -> Self {
        Self {
            phase: Phase::Inspections,
            // Stored snapshots are fresh captures with increasing IDs. Every
            // retained cutoff pins its birth prefix, so a later capture cannot
            // have a smaller cutoff (or None after Some). Historical inspection
            // clones live separately and are scanned below.
            pin: e
                .observer
                .as_ref()
                .and_then(Observe::last_choice)
                .max(e.step_coordinate_cutoff())
                .max(
                    e.snapshots
                        .last_key_value()
                        .and_then(|(_, snapshot)| snapshot.info.last_choice),
                ),
            after: None,
            choice: 0,
            image: Condition::FALSE,
            positive_scope: Condition::FALSE,
            bindings: Arc::new(BTreeMap::new()),
            active: e.active,
            born: Condition::FALSE,
            boolean: None,
            transform: None,
            index: None,
            pending: None,
            state: e.state.clone(),
            pending_root: e.pending_root.clone(),
            task: 0,
            slot: 0,
        }
    }
    pub(super) fn tick(&mut self, e: &mut Engine) -> bool {
        debug_assert!(e.lane == Some(Owner::Collection));
        match self.phase {
            Phase::Inspections => {
                let next = match self.after {
                    Some(id) => e.inspections.range((Excluded(id), Unbounded)).next(),
                    None => e.inspections.first_key_value(),
                };
                if let Some((&id, inspection)) = next {
                    if let Some(snapshot) = &inspection.snapshot {
                        self.pin = self.pin.max(snapshot.info.last_choice);
                    }
                    self.after = Some(id);
                } else {
                    self.phase = Phase::Choices;
                    self.after = self.pin;
                }
            }
            Phase::Choices => {
                if self.active == Condition::FALSE {
                    self.after = None;
                    self.phase = Phase::Births;
                    return false;
                }
                let next = match self.after {
                    Some(id) => e.births.range((Excluded(id), Unbounded)).next(),
                    None => e.births.first_key_value(),
                };
                if let Some((&id, birth)) = next {
                    #[cfg(feature = "diagnostics")]
                    {
                        e.diagnostics.shared.compaction.choices_examined += 1;
                    }
                    self.choice = id;
                    self.boolean = Some(e.arena.start(Operation::And(self.active, birth.decision)));
                    self.phase = Phase::Global;
                } else if self.bindings.is_empty() {
                    self.after = None;
                    self.phase = Phase::Births;
                } else {
                    self.index = Some(
                        e.graph
                            .index
                            .substitute(self.state.graph.clone(), self.bindings.clone()),
                    );
                    self.phase = Phase::Graph;
                }
            }
            Phase::Global => {
                if let Some(scope) = measured_poll!(
                    &mut self.boolean,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.boolean
                ) {
                    if scope == Condition::FALSE || scope == self.active {
                        self.image = if scope == self.active {
                            Condition::TRUE
                        } else {
                            Condition::FALSE
                        };
                        self.reduce(e);
                    } else if e.births[&self.choice].support == Condition::TRUE {
                        self.born = self.active;
                        self.positive_scope = scope;
                        self.boolean = Some(e.arena.start(Operation::And(
                            self.active,
                            e.births[&self.choice].decision.not(),
                        )));
                        self.phase = Phase::Negative;
                    } else {
                        self.transform = Some(
                            e.arena
                                .substitute(e.births[&self.choice].support, self.bindings.clone()),
                        );
                        self.phase = Phase::BirthSupport;
                    }
                }
            }
            Phase::BirthSupport => {
                if let Some(scope) = measured_poll!(
                    &mut self.transform,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.transform
                ) {
                    self.boolean = Some(e.arena.start(Operation::And(self.active, scope)));
                    self.phase = Phase::Born;
                }
            }
            Phase::Born => {
                if let Some(scope) = measured_poll!(
                    &mut self.boolean,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.boolean
                ) {
                    if scope == Condition::FALSE {
                        e.births.remove(&self.choice);
                        self.after = Some(self.choice);
                        self.phase = Phase::Choices;
                    } else {
                        self.born = scope;
                        self.boolean = Some(
                            e.arena
                                .start(Operation::And(scope, e.births[&self.choice].decision)),
                        );
                        self.phase = Phase::Positive;
                    }
                }
            }
            Phase::Positive => {
                if let Some(c) = measured_poll!(
                    &mut self.boolean,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.boolean
                ) {
                    if c == Condition::FALSE {
                        self.image = Condition::FALSE;
                        self.reduce(e);
                    } else {
                        self.positive_scope = c;
                        self.boolean = Some(e.arena.start(Operation::And(
                            self.born,
                            e.births[&self.choice].decision.not(),
                        )));
                        self.phase = Phase::Negative;
                    }
                }
            }
            Phase::Negative => {
                if let Some(c) = measured_poll!(
                    &mut self.boolean,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.boolean
                ) {
                    if c == Condition::FALSE {
                        self.image = Condition::TRUE;
                        self.reduce(e);
                    } else {
                        self.born = c;
                        self.transform =
                            Some(e.arena.project_before(self.positive_scope, self.choice));
                        self.phase = Phase::ProjectPositive;
                    }
                }
            }
            Phase::ProjectPositive => {
                if let Some(c) = measured_poll!(
                    &mut self.transform,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.transform
                ) {
                    self.image = c;
                    self.transform = Some(e.arena.project_before(self.born, self.choice));
                    self.phase = Phase::ProjectNegative;
                }
            }
            Phase::ProjectNegative => {
                if let Some(c) = measured_poll!(
                    &mut self.transform,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.transform
                ) {
                    self.boolean = Some(e.arena.start(Operation::And(self.image, c)));
                    self.phase = Phase::Disjoint;
                }
            }
            Phase::Disjoint => {
                if let Some(c) = measured_poll!(
                    &mut self.boolean,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.boolean
                ) {
                    if c == Condition::FALSE {
                        // No older assignment permits both born arms. The positive
                        // projection is the unique value there; outside the birth
                        // this coordinate does not denote a causal distinction.
                        self.reduce(e);
                    } else {
                        self.after = Some(self.choice);
                        self.phase = Phase::Choices;
                    }
                }
            }
            Phase::Reduce => {
                if let Some(c) = measured_poll!(
                    &mut self.transform,
                    &mut e.arena,
                    e.diagnostics.shared.compaction.transform
                ) {
                    self.active = c;
                    // Every temporary transform has released its assignment
                    // reference before extending the shared substitution map.
                    Arc::get_mut(&mut self.bindings)
                        .expect("exclusive discovery assignments")
                        .insert(self.choice, self.image);
                    self.after = Some(self.choice);
                    self.phase = Phase::Choices;
                }
            }
            Phase::Graph => {
                #[cfg(feature = "diagnostics")]
                {
                    e.diagnostics.shared.compaction.graph_index_steps += 1;
                }
                if let Some(root) = self
                    .index
                    .as_mut()
                    .unwrap()
                    .tick(&mut e.graph.index, &mut e.arena)
                {
                    self.state.graph = root;
                    self.index = Some(
                        e.history
                            .index
                            .substitute(self.state.history.clone(), self.bindings.clone()),
                    );
                    self.phase = Phase::History;
                }
            }
            Phase::History => {
                #[cfg(feature = "diagnostics")]
                {
                    e.diagnostics.shared.compaction.history_index_steps += 1;
                }
                if let Some(root) = self
                    .index
                    .as_mut()
                    .unwrap()
                    .tick(&mut e.history.index, &mut e.arena)
                {
                    self.state.history = root;
                    self.index = None;
                    self.pending = Some(
                        e.obligations
                            .substitute(self.pending_root.clone(), self.bindings.clone()),
                    );
                    self.phase = Phase::Pending;
                }
            }
            Phase::Pending => {
                #[cfg(feature = "diagnostics")]
                {
                    e.diagnostics.shared.compaction.pending_index_steps += 1;
                }
                let pending = self.pending.as_mut().unwrap();
                let result = pending.tick(&mut e.obligations, &mut e.arena);
                #[cfg(feature = "diagnostics")]
                {
                    let delta = std::mem::take(&mut pending.measured_transform);
                    e.diagnostics.shared.compaction.pending_transform.calls += delta.calls;
                    e.diagnostics.shared.compaction.pending_transform.work += delta.work;
                }
                if let Some(root) = result {
                    self.pending_root = root;
                    self.pending = None;
                    self.phase = Phase::Tasks;
                }
            }
            Phase::Tasks | Phase::Parked => {
                let scheduled = if matches!(self.phase, Phase::Tasks) {
                    e.queue.get_mut(self.task)
                } else {
                    match self.after {
                        Some(id) => e
                            .parked
                            .range_mut((Excluded(id), Unbounded))
                            .next()
                            .map(|(_, task)| task),
                        None => e.parked.values_mut().next(),
                    }
                };
                if let Some(scheduled) = scheduled {
                    if self.slot == 0 {
                        self.transform =
                            Some(e.arena.substitute(scheduled.scope, self.bindings.clone()));
                        self.slot = 1;
                    } else if let Some(c) = measured_poll!(
                        &mut self.transform,
                        &mut e.arena,
                        e.diagnostics.shared.compaction.transform
                    ) {
                        scheduled.scope = c;
                        if let Task::Body(body) = &mut scheduled.task {
                            debug_assert!(
                                body.job.is_none() && body.merge.is_none() && body.update.is_none()
                            );
                            // Pending bodies have not acquired the mutation lane,
                            // so their scope still equals the scheduling support.
                            body.scope = c;
                        }
                        self.task += 1;
                        self.after = Some(scheduled.id);
                        self.slot = 0;
                    }
                } else {
                    self.phase = if matches!(self.phase, Phase::Tasks) {
                        Phase::Parked
                    } else {
                        Phase::Births
                    };
                    self.after = None;
                    self.slot = 0;
                }
            }
            Phase::Births => {
                let next = match self.after {
                    Some(id) => e.births.range((Excluded(id), Unbounded)).next(),
                    None => e.births.first_key_value(),
                };
                if let Some((&id, birth)) = next {
                    if self.bindings.contains_key(&id) {
                        e.births.remove(&id);
                        self.after = Some(id);
                    } else if self.pin.is_some_and(|pin| id <= pin) {
                        self.after = Some(id);
                    } else if self.slot == 0 {
                        self.transform =
                            Some(e.arena.substitute(birth.support, self.bindings.clone()));
                        self.slot = 1;
                    } else if self.slot == 1 {
                        if let Some(c) = measured_poll!(
                            &mut self.transform,
                            &mut e.arena,
                            e.diagnostics.shared.compaction.transform
                        ) {
                            self.boolean = Some(e.arena.start(Operation::And(c, self.active)));
                            self.slot = 2;
                        }
                    } else if let Some(c) = measured_poll!(
                        &mut self.boolean,
                        &mut e.arena,
                        e.diagnostics.shared.compaction.boolean
                    ) {
                        if c == Condition::FALSE {
                            e.births.remove(&id);
                        } else {
                            e.births.get_mut(&id).unwrap().support = c;
                        }
                        self.after = Some(id);
                        self.slot = 0;
                    }
                } else {
                    self.phase = Phase::Publish;
                }
            }
            Phase::Publish => {
                e.state = self.state.clone();
                e.active = self.active;
                e.pending_root = self.pending_root.clone();
                if !self.bindings.is_empty() {
                    #[cfg(feature = "diagnostics")]
                    {
                        e.diagnostics.shared.coordinates.publications += 1;
                        e.diagnostics.shared.coordinates.assignments_published +=
                            self.bindings.len() as u64;
                    }
                    e.coordinates.publish(self.bindings.clone());
                }
                return true;
            }
        }
        false
    }
    fn reduce(&mut self, e: &Engine) {
        self.transform = Some(e.arena.substitute(
            self.active,
            Arc::new(BTreeMap::from([(self.choice, self.image)])),
        ));
        self.phase = Phase::Reduce;
    }
}
