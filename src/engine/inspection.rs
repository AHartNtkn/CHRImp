//! Explicit immutable views and independently budgeted graph projection.
use super::*;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_VIEW: AtomicU64 = AtomicU64::new(1);
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ViewId(pub u64);
fn next_id() -> ViewId {
    ViewId(
        NEXT_VIEW
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("view identity exhausted"),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotKind {
    Requested,
    Initial,
    Application { rule: usize, event: u64 },
    Post { occurrence: u64 },
    Merge,
    Choice,
    Failure,
    NormalForm,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct SnapshotInfo {
    pub id: ViewId,
    pub applications: u64,
    pub kind: SnapshotKind,
    pub last_choice: Option<u64>,
}
#[derive(Clone)]
pub(super) struct Snapshot {
    pub info: SnapshotInfo,
    pub graph: Root,
    pub scope: Condition,
    variables: Arc<Vec<u64>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionError {
    Canceled,
    Busy,
    Initializing,
    UnknownSnapshot,
    UnknownInspection,
    UnknownChoice,
    InProgress,
}
impl std::fmt::Display for InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Canceled => "source execution has been canceled",
            Self::Busy => "collection is in progress; retry after advancing the engine",
            Self::Initializing => "query variables are still being initialized",
            Self::UnknownSnapshot => "snapshot does not belong to this run or has been released",
            Self::UnknownInspection => {
                "inspection does not belong to this run or has been released"
            }
            Self::UnknownChoice => "choice does not exist in this snapshot",
            Self::InProgress => "finish or cancel and drain the inspection before releasing it",
        })
    }
}
impl std::error::Error for InspectionError {}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct InspectionStatus {
    pub done: bool,
    pub canceled: bool,
    pub error: Option<InspectionError>,
}
#[derive(Clone, Copy)]
enum Phase {
    Select,
    Support,
    Decision,
    Project,
    Discard,
    Done,
}
pub(super) struct Inspection {
    pub snapshot: Option<Snapshot>,
    info: SnapshotInfo,
    selections: VecDeque<(u64, bool)>,
    scope: Condition,
    decision: Condition,
    job: Option<Job>,
    observer: Option<Observe>,
    output: Option<Output>,
    phase: Phase,
    canceled: bool,
    error: Option<InspectionError>,
    id: ViewId,
}
impl Inspection {
    fn new(id: ViewId, snapshot: Snapshot, selections: Vec<(u64, bool)>) -> Self {
        Self {
            info: snapshot.info,
            scope: snapshot.scope,
            snapshot: Some(snapshot),
            selections: selections.into(),
            decision: Condition::FALSE,
            job: None,
            observer: None,
            output: None,
            phase: Phase::Select,
            canceled: false,
            error: None,
            id,
        }
    }
    fn finish(&mut self) {
        self.snapshot = None;
        self.scope = Condition::FALSE;
        self.decision = Condition::FALSE;
        self.selections = VecDeque::new();
        self.phase = Phase::Done;
    }
    fn request_cancel(&mut self) {
        self.canceled = true;
        self.output = None;
        if !matches!(self.phase, Phase::Done) {
            self.phase = Phase::Discard;
        }
    }
    fn drain_tick(&mut self) -> bool {
        if matches!(self.phase, Phase::Done) {
            return true;
        }
        if let Some(job) = &mut self.job {
            if job.discard_tick() {
                self.job = None;
            }
        } else if let Some(observer) = &mut self.observer {
            if observer.discard_tick() {
                self.observer = None;
            }
        } else if self.selections.pop_front().is_none() {
            self.finish();
            return true;
        }
        false
    }
    pub(super) fn discard_tick(&mut self) -> bool {
        self.request_cancel();
        self.drain_tick()
    }
    fn tick(
        &mut self,
        graph: &Graph,
        arena: &mut Arena,
        births: &BTreeMap<u64, Birth>,
        code: &Arc<Prepared>,
    ) {
        if self.output.is_some() {
            return;
        }
        match self.phase {
            Phase::Select => {
                if let Some((id, positive)) = self.selections.pop_front() {
                    let snapshot = self.snapshot.as_ref().unwrap();
                    if let Some(birth) = births
                        .get(&id)
                        .filter(|_| snapshot.info.last_choice.is_some_and(|last| id <= last))
                    {
                        self.decision = if positive {
                            birth.decision
                        } else {
                            birth.decision.not()
                        };
                        self.job = Some(arena.start(Operation::And(self.scope, birth.support)));
                        self.phase = Phase::Support;
                    } else {
                        self.error = Some(InspectionError::UnknownChoice);
                        self.phase = Phase::Discard;
                    }
                } else {
                    self.selections = VecDeque::new();
                    let snapshot = self.snapshot.as_ref().unwrap();
                    self.observer = Some(Observe::new(
                        Completion {
                            id: self.id.0,
                            support: self.scope,
                            state: StateRoot {
                                graph: snapshot.graph,
                                history: graph.empty(),
                            },
                            last_choice: snapshot.info.last_choice,
                        },
                        code.clone(),
                        snapshot.variables.clone(),
                    ));
                    self.phase = Phase::Project;
                }
            }
            Phase::Support => {
                if let Some(scope) = poll(&mut self.job, arena) {
                    self.job = Some(arena.start(Operation::And(scope, self.decision)));
                    self.phase = Phase::Decision;
                }
            }
            Phase::Decision => {
                if let Some(scope) = poll(&mut self.job, arena) {
                    self.scope = scope;
                    self.decision = Condition::FALSE;
                    self.phase = Phase::Select;
                }
            }
            Phase::Project => match self.observer.as_mut().unwrap().tick(graph, arena, births) {
                ObserveStatus::Pending => {}
                ObserveStatus::Event(output) => self.output = Some(output),
                ObserveStatus::Done => {
                    self.observer = None;
                    self.finish();
                }
            },
            Phase::Discard => {
                self.drain_tick();
            }
            Phase::Done => {}
        }
    }
    fn status(&self) -> InspectionStatus {
        InspectionStatus {
            done: matches!(self.phase, Phase::Done) && self.output.is_none(),
            canceled: self.canceled,
            error: self.error,
        }
    }
}
impl Trace for Inspection {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[
                self.scope,
                self.decision,
                self.snapshot.as_ref().map_or(Condition::FALSE, |s| s.scope),
            ]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.observer.as_ref()),
            _ => Step::Done,
        }
    }
}
impl Engine {
    fn view_boundary(&self) -> Result<(), InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        if !self.canceled() && self.variables.len() != self.code.query_variables.len() {
            return Err(InspectionError::Initializing);
        }
        Ok(())
    }
    fn snapshot(&self, kind: SnapshotKind, scope: Condition) -> Snapshot {
        Snapshot {
            info: SnapshotInfo {
                id: next_id(),
                applications: self.applications,
                kind,
                last_choice: self.births.last_key_value().map(|(&id, _)| id),
            },
            graph: self.state.graph,
            scope,
            variables: self.variables.clone(),
        }
    }
    pub(super) fn record(&mut self, kind: SnapshotKind, scope: Condition) {
        if self.record_history {
            let snapshot = self.snapshot(kind, scope);
            self.snapshots.insert(snapshot.info.id.0, snapshot);
        }
    }
    /// A requested snapshot owns its immutable graph until explicitly released.
    pub fn capture_snapshot(&mut self) -> Result<ViewId, InspectionError> {
        self.view_boundary()?;
        let snapshot = self.snapshot(SnapshotKind::Requested, self.active);
        let id = snapshot.info.id;
        self.snapshots.insert(id.0, snapshot);
        Ok(id)
    }
    pub fn snapshots(&self) -> impl Iterator<Item = SnapshotInfo> + '_ {
        self.snapshots.values().map(|s| s.info)
    }
    pub fn snapshot_info(&self, id: ViewId) -> Result<SnapshotInfo, InspectionError> {
        self.snapshots
            .get(&id.0)
            .map(|s| s.info)
            .ok_or(InspectionError::UnknownSnapshot)
    }
    pub fn snapshots_after(
        &self,
        after: Option<ViewId>,
    ) -> impl Iterator<Item = SnapshotInfo> + '_ {
        use std::ops::Bound::{Excluded, Unbounded};
        self.snapshots
            .range((after.map_or(Unbounded, |id| Excluded(id.0)), Unbounded))
            .map(|(_, s)| s.info)
    }
    pub fn choices_after(
        &self,
        after: Option<u64>,
        through: Option<u64>,
    ) -> impl Iterator<Item = (&u64, &Birth)> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let bounds = match through {
            Some(last) if after.is_none_or(|id| id < last) => {
                (after.map_or(Unbounded, Excluded), Included(last))
            }
            _ => (Included(0), Excluded(0)),
        };
        self.births.range(bounds)
    }
    pub fn release_snapshot(&mut self, id: ViewId) -> Result<(), InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        self.snapshots
            .remove(&id.0)
            .ok_or(InspectionError::UnknownSnapshot)?;
        self.request_collection();
        Ok(())
    }
    /// None captures the current view only for this projection, without recording history.
    /// Selection constrains both a choice's birth region and its requested arm.
    pub fn start_inspection(
        &mut self,
        snapshot: Option<ViewId>,
        choices: Vec<(u64, bool)>,
    ) -> Result<ViewId, InspectionError> {
        self.view_boundary()?;
        let snapshot = match snapshot {
            Some(id) => self
                .snapshots
                .get(&id.0)
                .cloned()
                .ok_or(InspectionError::UnknownSnapshot)?,
            None => self.snapshot(SnapshotKind::Requested, self.active),
        };
        let id = next_id();
        self.inspections
            .insert(id.0, Inspection::new(id, snapshot, choices));
        self.latest_inspection = Some(id.0);
        Ok(id)
    }
    /// Metadata refers to the captured view, including after its graph is released.
    /// An ephemeral view's metadata does not register a persistent snapshot handle.
    pub fn inspection_snapshot_info(&self, id: ViewId) -> Result<SnapshotInfo, InspectionError> {
        self.inspections
            .get(&id.0)
            .map(|i| i.info)
            .ok_or(InspectionError::UnknownInspection)
    }
    pub fn inspections(&self) -> impl Iterator<Item = ViewId> + '_ {
        self.inspections.keys().map(|&id| ViewId(id))
    }
    pub fn inspection_status(&self, id: ViewId) -> Result<InspectionStatus, InspectionError> {
        self.inspections
            .get(&id.0)
            .map(Inspection::status)
            .ok_or(InspectionError::UnknownInspection)
    }
    pub fn take_inspection_output(
        &mut self,
        id: ViewId,
    ) -> Result<Option<Output>, InspectionError> {
        self.inspections
            .get_mut(&id.0)
            .map(|i| i.output.take())
            .ok_or(InspectionError::UnknownInspection)
    }
    pub fn cancel_inspection(&mut self, id: ViewId) -> Result<(), InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        let inspection = self
            .inspections
            .get_mut(&id.0)
            .ok_or(InspectionError::UnknownInspection)?;
        inspection.request_cancel();
        Ok(())
    }
    /// Discard a view without starting a collection between batched root releases.
    pub fn discard_inspection(
        &mut self,
        id: ViewId,
        budget: usize,
    ) -> Result<bool, InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        let view = self
            .inspections
            .get_mut(&id.0)
            .ok_or(InspectionError::UnknownInspection)?;
        for _ in 0..budget {
            if view.discard_tick() {
                return Ok(true);
            }
        }
        Ok(view.status().done)
    }
    pub fn release_inspection(&mut self, id: ViewId) -> Result<(), InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        if !self.inspection_status(id)?.done {
            return Err(InspectionError::InProgress);
        }
        self.inspections.remove(&id.0);
        self.request_collection();
        Ok(())
    }
    /// Inspection can advance while source execution is paused. Each call is budgeted,
    /// and a single unread scalar event applies backpressure to its projection.
    pub fn advance_inspection(&mut self, id: ViewId, budget: usize) -> Result<(), InspectionError> {
        if !self.inspections.contains_key(&id.0) {
            return Err(InspectionError::UnknownInspection);
        }
        for _ in 0..budget {
            if self.canceled()
                && self
                    .cancellation
                    .inspection_limit
                    .is_some_and(|last| id.0 <= last)
            {
                self.inspections.get_mut(&id.0).unwrap().request_cancel();
            }
            let inspection = self.inspections.get(&id.0).unwrap();
            if inspection.output.is_some() || matches!(inspection.phase, Phase::Done) {
                break;
            }
            if !self.collect_heap_mode(false) {
                if self.canceled() && !self.cancellation.finished {
                    self.cancel_tick();
                } else {
                    self.inspection_tick(id.0);
                }
            }
        }
        Ok(())
    }
    fn inspection_tick(&mut self, id: u64) {
        if self.canceled()
            && self
                .cancellation
                .inspection_limit
                .is_some_and(|last| id <= last)
        {
            self.inspections.get_mut(&id).unwrap().request_cancel();
        }
        self.inspections.get_mut(&id).unwrap().tick(
            &self.graph,
            &mut self.arena,
            &self.births,
            &self.code,
        );
    }
    pub(super) fn service_inspection(&mut self) {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        // Freeze the admission boundary for each round. New IDs cannot keep
        // moving its end and postpone an already waiting view or cancellation.
        if self.inspection_round.is_none() {
            self.inspection_round = self.inspections.last_key_value().map(|(&id, _)| id);
        }
        let Some(last) = self.inspection_round else {
            return;
        };
        let next = self
            .inspections
            .range((
                self.last_inspection.map_or(Unbounded, Excluded),
                Included(last),
            ))
            .next()
            .map(|(&id, _)| id);
        if let Some(id) = next {
            self.last_inspection = Some(id);
            self.inspection_tick(id);
        } else {
            self.last_inspection = None;
            self.inspection_round = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuing_admissions_cannot_overtake_a_waiting_cancellation() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let waiting = e.start_inspection(None, vec![]).unwrap();
        e.service_inspection();
        e.cancel_inspection(waiting).unwrap();
        for _ in 0..50 {
            let newer = e.start_inspection(None, vec![]).unwrap();
            e.service_inspection();
            e.cancel_inspection(newer).unwrap();
            e.advance_inspection(newer, 100).unwrap();
            e.release_inspection(newer).unwrap();
            if e.inspection_status(waiting).unwrap().done {
                break;
            }
        }
        assert!(
            e.inspection_status(waiting).unwrap().done,
            "later arrivals must not indefinitely postpone the existing discard"
        );
    }
}
