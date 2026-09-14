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
    pub obligations: PendingRoot,
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
    FindPrefix,
    Select,
    RestrictScope,
    Observe,
    Project,
    Pending,
    Discard,
    Done,
}
pub(super) struct Inspection {
    pub snapshot: Option<Snapshot>,
    info: SnapshotInfo,
    prefixes: Vec<u64>,
    selection: usize,
    endpoint: Option<u64>,
    scope: Condition,
    job: Option<Job>,
    observer: Option<Observe>,
    pending: Option<obligations::Projection>,
    alternative_scope: Condition,
    output: Option<Output>,
    phase: Phase,
    canceled: bool,
    error: Option<InspectionError>,
    id: ViewId,
    #[cfg(test)]
    selection_jobs: usize,
}
impl Inspection {
    fn new(
        id: ViewId,
        snapshot: Snapshot,
        prefixes: Vec<u64>,
        error: Option<InspectionError>,
    ) -> Self {
        Self {
            info: snapshot.info,
            scope: if prefixes.is_empty() {
                snapshot.scope
            } else {
                Condition::TRUE
            },
            snapshot: Some(snapshot),
            selection: prefixes.len(),
            endpoint: prefixes.last().copied(),
            prefixes,
            job: None,
            observer: None,
            pending: None,
            alternative_scope: Condition::FALSE,
            output: None,
            phase: if error.is_some() {
                Phase::Discard
            } else {
                Phase::FindPrefix
            },
            canceled: false,
            error,
            id,
            #[cfg(test)]
            selection_jobs: 0,
        }
    }
    fn finish(&mut self) {
        self.snapshot = None;
        self.scope = Condition::FALSE;
        self.alternative_scope = Condition::FALSE;
        self.prefixes = Vec::new();
        self.phase = Phase::Done;
    }
    fn request_cancel(&mut self) {
        self.canceled = true;
        self.output = None;
        if !matches!(self.phase, Phase::Done) {
            self.phase = Phase::Discard;
        }
    }
    fn drain_tick(&mut self, restrictions: &mut restriction::Restrictions) -> bool {
        if matches!(self.phase, Phase::Done) {
            return true;
        }
        if let Some(id) = self.endpoint.take() {
            restrictions.release_endpoint(id);
        } else if let Some(job) = &mut self.job {
            if job.discard_tick() {
                self.job = None;
            }
        } else if let Some(pending) = &mut self.pending {
            if pending.discard_tick() {
                self.pending = None;
            }
        } else if let Some(observer) = &mut self.observer {
            if observer.discard_tick() {
                self.observer = None;
            }
        } else if let Some(id) = self.prefixes.pop() {
            self.job = restrictions.release(id);
        } else {
            self.finish();
            return true;
        }
        false
    }
    pub(super) fn discard_tick(&mut self, restrictions: &mut restriction::Restrictions) -> bool {
        self.request_cancel();
        self.drain_tick(restrictions)
    }
    fn tick(
        &mut self,
        graph: &Graph,
        arena: &mut Arena,
        births: &BTreeMap<u64, Birth>,
        code: &Arc<Prepared>,
        obligations: &obligations::Obligations,
        restrictions: &mut restriction::Restrictions,
    ) {
        if self.output.is_some() {
            return;
        }
        match self.phase {
            Phase::FindPrefix => {
                // One metadata lookup per budget tick; admission does no search.
                if self.selection == 0 {
                    self.phase = Phase::Select;
                } else {
                    let id = self.prefixes[self.selection - 1];
                    if let Some(scope) = restrictions.result(id) {
                        self.scope = scope;
                        self.phase = Phase::Select;
                    } else {
                        self.selection -= 1;
                    }
                }
            }
            Phase::Select => {
                if let Some(&id) = self.prefixes.get(self.selection) {
                    if let Some(scope) = restrictions.tick(id, self.scope, arena) {
                        self.scope = scope;
                        self.selection += 1;
                    }
                } else if self.prefixes.is_empty() {
                    self.phase = Phase::Observe;
                } else {
                    // Restriction computation is shared across snapshot scopes.
                    // Apply the reader's own scope exactly once at the boundary.
                    self.job = Some(arena.start(Operation::And(
                        self.scope,
                        self.snapshot.as_ref().unwrap().scope,
                    )));
                    #[cfg(test)]
                    {
                        self.selection_jobs += 1;
                    }
                    self.phase = Phase::RestrictScope;
                }
            }
            Phase::RestrictScope => {
                if let Some(scope) = poll(&mut self.job, arena) {
                    self.scope = scope;
                    self.phase = Phase::Observe;
                }
            }
            Phase::Observe => {
                let snapshot = self.snapshot.as_ref().unwrap();
                self.observer = Some(Observe::new(
                    Completion {
                        id: self.id.0,
                        support: self.scope,
                        state: StateRoot {
                            graph: snapshot.graph.clone(),
                            history: graph.empty(),
                        },
                        last_choice: snapshot.info.last_choice,
                    },
                    code.clone(),
                    snapshot.variables.clone(),
                ));
                self.phase = Phase::Project;
            }
            Phase::Project => match self.observer.as_mut().unwrap().tick(graph, arena, births) {
                ObserveStatus::Pending => {}
                ObserveStatus::Event(Output::End) => {
                    self.pending = Some(obligations::Projection::new(
                        obligations,
                        self.snapshot.as_ref().unwrap().obligations.clone(),
                        self.alternative_scope,
                    ));
                    self.phase = Phase::Pending;
                }
                ObserveStatus::Event(output) => {
                    if matches!(output, Output::Begin { .. }) {
                        self.alternative_scope = self.observer.as_ref().unwrap().current_scope();
                    }
                    self.output = Some(output);
                }
                ObserveStatus::Done => {
                    self.observer = None;
                    self.phase = Phase::Discard;
                }
            },
            Phase::Pending => {
                match self.pending.as_mut().unwrap().tick(
                    obligations,
                    graph,
                    self.snapshot.as_ref().unwrap().graph.clone(),
                    arena,
                    code,
                ) {
                    ObserveStatus::Pending => {}
                    ObserveStatus::Event(output) => self.output = Some(output),
                    ObserveStatus::Done => {
                        self.pending = None;
                        self.alternative_scope = Condition::FALSE;
                        self.output = Some(Output::End);
                        self.phase = Phase::Project;
                    }
                }
            }
            Phase::Discard => {
                self.drain_tick(restrictions);
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
                self.alternative_scope,
                self.snapshot.as_ref().map_or(Condition::FALSE, |s| s.scope),
            ]),
            1 => c.optional(self.job.as_ref()),
            2 => c.optional(self.observer.as_ref()),
            3 => c.optional(self.pending.as_ref()),
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
    fn snapshot(&mut self, kind: SnapshotKind, scope: Condition) -> Snapshot {
        // External captures run between service turns, with no collector or
        // task temporarily outside its registry. Recording starts promoted.
        // First capture visits each queued/parked task and inserts its body
        // descriptor; later captures retain the existing root/epoch publication.
        while !self.promote_body_syntax_tick() {}
        // Every new syntax view freezes the current descriptor epoch. Reusing
        // an existing Snapshot only shares its already-frozen ownership root.
        self.obligations.freeze_syntax();
        Snapshot {
            info: SnapshotInfo {
                id: next_id(),
                applications: self.applications,
                kind,
                last_choice: self.births.last_key_value().map(|(&id, _)| id),
            },
            graph: self.state.graph.clone(),
            obligations: self.pending_root.clone(),
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
    pub fn snapshots_before(&self, before: ViewId) -> impl Iterator<Item = SnapshotInfo> + '_ {
        self.snapshots.range(..before.0).rev().map(|(_, s)| s.info)
    }
    pub fn choices_before(
        &self,
        before: u64,
        through: Option<u64>,
    ) -> impl Iterator<Item = (&u64, &Birth)> {
        use std::ops::Bound::{Excluded, Included};
        let end = match through {
            Some(last) if last < before => Included(last),
            Some(_) => Excluded(before),
            None => Excluded(0),
        };
        self.births.range((Included(0), end)).rev()
    }
    pub fn release_snapshot(&mut self, id: ViewId) -> Result<(), InspectionError> {
        if self.collector.is_some() {
            return Err(InspectionError::Busy);
        }
        self.snapshots
            .remove(&id.0)
            .ok_or(InspectionError::UnknownSnapshot)?;
        self.archive.invalidate();
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
        // Register the complete requested path before execution. Thus readers
        // admitted together share even when drained sequentially. Registration
        // performs O(N log C) map work for N supplied choices and C live
        // prefixes, with no Boolean evaluation or graph traversal.
        let mut prefixes = Vec::with_capacity(choices.len());
        let mut error = None;
        for (choice, positive) in choices {
            let Some(birth) = self
                .births
                .get(&choice)
                .filter(|_| snapshot.info.last_choice.is_some_and(|last| choice <= last))
            else {
                error = Some(InspectionError::UnknownChoice);
                break;
            };
            let decision = if positive {
                birth.decision
            } else {
                birth.decision.not()
            };
            let prefix =
                self.restrictions
                    .acquire(prefixes.last().copied(), birth.support, decision);
            prefixes.push(prefix);
        }
        if let Some(&endpoint) = prefixes.last() {
            self.restrictions.endpoint(endpoint);
        }
        self.inspections
            .insert(id.0, Inspection::new(id, snapshot, prefixes, error));
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
            if view.discard_tick(&mut self.restrictions) {
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
            &self.obligations,
            &mut self.restrictions,
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

#[cfg(test)]
mod cutoff_tests {
    use super::*;
    fn check(e: &Engine) {
        let cutoffs = e
            .snapshots
            .values()
            .map(|s| s.info.last_choice)
            .collect::<Vec<_>>();
        assert!(cutoffs.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(
            e.snapshots
                .last_key_value()
                .and_then(|(_, s)| s.info.last_choice),
            cutoffs.into_iter().flatten().max()
        );
    }
    #[test]
    fn capture_order_bounds_cutoffs_through_collection_release_and_old_inspection_clones() {
        for history in [false, true] {
            let code = crate::program::prepare(
                &crate::syntax::parse_program("").unwrap(),
                &crate::syntax::parse_query("(a();b()),(c();d())").unwrap(),
            )
            .unwrap();
            let mut e = Engine::with_history(Arc::new(code), history);
            let initial = e.capture_snapshot().unwrap();
            assert_eq!(e.snapshot_info(initial).unwrap().last_choice, None);
            let mut choices = 0;
            for _ in 0..100_000 {
                e.advance(1);
                e.take_output();
                if !e.collecting() && e.births.len() > choices {
                    choices = e.births.len();
                    e.capture_snapshot().unwrap();
                    e.capture_snapshot().unwrap();
                    check(&e);
                }
                if e.delivery_done() {
                    break;
                }
            }
            assert!(e.delivery_done());
            check(&e);
            e.cancel();
            for _ in 0..100_000 {
                e.advance(1);
                if e.cancel_done() {
                    break;
                }
            }
            assert!(e.cancel_done());
            e.capture_snapshot().unwrap();
            check(&e);
            let old = e
                .snapshots
                .values()
                .find(|s| s.info.last_choice == Some(0))
                .unwrap()
                .info
                .id;
            let clone = e.start_inspection(Some(old), vec![]).unwrap();
            let mut ids = e.snapshots.keys().copied().collect::<Vec<_>>();
            while let Some(id) = ids.pop() {
                e.release_snapshot(ViewId(id)).unwrap();
                e.maintain(100_000);
                check(&e);
                ids.reverse();
            }
            assert!(e.snapshots.is_empty());
            assert_eq!(e.births.last_key_value().map(|(&id, _)| id), Some(0));
            e.cancel_inspection(clone).unwrap();
            e.advance_inspection(clone, 100_000).unwrap();
            assert!(e.inspection_status(clone).unwrap().done);
            e.release_inspection(clone).unwrap();
            e.maintain(100_000);
            assert!(e.births.is_empty());
        }
    }
}

#[cfg(test)]
mod restriction_tests {
    use super::*;

    pub(super) fn source() -> Engine {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("hold() <=> hold().").unwrap(),
            &crate::syntax::parse_query("hold(),(a();b()),(c();d()),(e();f())").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        e.advance(10_000);
        while e.collecting() {
            e.maintain(1);
        }
        assert_eq!(e.births.len(), 3);
        e
    }
    pub(super) fn drain(e: &mut Engine, id: ViewId) -> usize {
        let mut answers = 0;
        for _ in 0..100_000 {
            e.advance_inspection(id, 1).unwrap();
            if matches!(e.take_inspection_output(id).unwrap(), Some(Output::End)) {
                answers += 1;
            }
            let status = e.inspection_status(id).unwrap();
            assert_eq!(status.error, None);
            if status.done {
                return answers;
            }
        }
        panic!("inspection must finish");
    }
    fn jobs(e: &Engine) -> usize {
        e.restrictions.jobs_started
            + e.inspections
                .values()
                .map(|i| i.selection_jobs)
                .sum::<usize>()
    }
    // Execute the original baseline's two Boolean operations per selection.
    // This is a direct work control, not a second production execution mode.
    fn baseline_restrict(e: &mut Engine, choices: &[(u64, bool)]) -> (Condition, usize) {
        let mut scope = Condition::TRUE;
        let mut started = 0;
        for &(id, positive) in choices {
            let birth = &e.births[&id];
            let operands = [
                birth.support,
                if positive {
                    birth.decision
                } else {
                    birth.decision.not()
                },
            ];
            for operand in operands {
                let mut job = Some(e.arena.start(Operation::And(scope, operand)));
                started += 1;
                loop {
                    if let Some(result) = poll(&mut job, &mut e.arena) {
                        scope = result;
                        break;
                    }
                }
            }
        }
        (scope, started)
    }
    #[test]
    fn live_successive_prefixes_execute_fewer_restriction_jobs() {
        let mut shared = source();
        let choices: Vec<_> = shared.births.keys().map(|&id| (id, true)).collect();
        let first_snapshot = shared.capture_snapshot().unwrap();
        let first = shared
            .start_inspection(Some(first_snapshot), choices[..2].to_vec())
            .unwrap();
        // A distinct graph snapshot must still share the selection prefix.
        shared.advance(1000);
        while shared.collecting() {
            shared.maintain(1);
        }
        let second = shared.start_inspection(None, choices.clone()).unwrap();
        shared.release_snapshot(first_snapshot).unwrap();
        shared.maintain(100_000);
        assert_eq!(drain(&mut shared, first), 2);
        assert_eq!(shared.memory().restriction_nodes, 3);
        shared.request_collection();
        shared.maintain(100_000);
        // First reader is complete before the second starts executing.
        assert_eq!(drain(&mut shared, second), 1);
        let shared_jobs = jobs(&shared);
        assert_eq!(shared.memory().restriction_nodes, 0);

        let mut isolated = source();
        let first = isolated
            .start_inspection(None, choices[..2].to_vec())
            .unwrap();
        assert_eq!(drain(&mut isolated, first), 2);
        let second = isolated.start_inspection(None, choices.clone()).unwrap();
        assert_eq!(drain(&mut isolated, second), 1);
        let isolated_jobs = jobs(&isolated);
        assert_eq!(isolated.memory().restriction_nodes, 0);
        let mut baseline = source();
        let (_, short_jobs) = baseline_restrict(&mut baseline, &choices[..2]);
        let (_, long_jobs) = baseline_restrict(&mut baseline, &choices);
        assert_eq!(
            (shared_jobs, short_jobs + long_jobs, isolated_jobs),
            (8, 10, 12)
        );
        assert!(shared_jobs < short_jobs + long_jobs);
        assert!(
            shared_jobs < isolated_jobs,
            "shared={shared_jobs}, isolated={isolated_jobs}"
        );
    }
}

#[cfg(test)]
mod shared_restriction_lifetime_tests {
    use super::restriction_tests::{drain, source};
    use super::*;

    #[test]
    fn shared_selection_respects_each_snapshot_scope_and_future_cutoff() {
        let mut e = source();
        let (&choice, birth) = e.births.first_key_value().unwrap();
        let decision = birth.decision;
        let yes = e.capture_snapshot().unwrap();
        let no = e.capture_snapshot().unwrap();
        // Valid restrictions of the same immutable graph; selection results
        // must not incorporate whichever snapshot happened to request first.
        e.snapshots.get_mut(&yes.0).unwrap().scope = decision;
        e.snapshots.get_mut(&no.0).unwrap().scope = decision.not();
        let a = e.start_inspection(Some(yes), vec![(choice, true)]).unwrap();
        let b = e.start_inspection(Some(no), vec![(choice, true)]).unwrap();
        assert_eq!(e.memory().restriction_nodes, 1);
        assert_eq!(drain(&mut e, a), 4);
        assert_eq!(drain(&mut e, b), 0);
        assert_eq!(e.restrictions.jobs_started, 2);
        assert_eq!(e.memory().restriction_nodes, 0);

        e.snapshots.get_mut(&no.0).unwrap().info.last_choice = None;
        let good = e.start_inspection(Some(yes), vec![(choice, true)]).unwrap();
        let bad = e.start_inspection(Some(no), vec![(choice, true)]).unwrap();
        e.advance_inspection(bad, 1000).unwrap();
        assert_eq!(
            e.inspection_status(bad).unwrap().error,
            Some(InspectionError::UnknownChoice)
        );
        assert!(e.take_inspection_output(bad).unwrap().is_none());
        assert_eq!(drain(&mut e, good), 4);
        assert_eq!(e.memory().restriction_nodes, 0);
    }

    #[test]
    fn invalid_and_canceled_partial_paths_release_without_executing_selection_jobs() {
        let mut e = source();
        let choices: Vec<_> = e.births.keys().map(|&id| (id, true)).collect();
        let keep = e.start_inspection(None, choices[..2].to_vec()).unwrap();
        let mut invalid = choices.clone();
        invalid.push((u64::MAX, true));
        let bad = e.start_inspection(None, invalid).unwrap();
        let canceled = e.start_inspection(None, choices).unwrap();
        assert_eq!(
            e.restrictions.jobs_started, 0,
            "admission evaluates no restrictions"
        );
        assert_eq!(e.memory().restriction_nodes, 3);
        e.cancel_inspection(canceled).unwrap();
        for id in [bad, canceled] {
            e.advance_inspection(id, 1000).unwrap();
            assert!(e.inspection_status(id).unwrap().done);
            assert!(e.take_inspection_output(id).unwrap().is_none());
        }
        assert_eq!(
            e.inspection_status(bad).unwrap().error,
            Some(InspectionError::UnknownChoice)
        );
        assert_eq!(e.restrictions.jobs_started, 0);
        assert_eq!(e.memory().restriction_nodes, 2);
        e.release_inspection(bad).unwrap();
        e.release_inspection(canceled).unwrap();
        e.request_collection();
        e.maintain(100_000);
        assert_eq!(drain(&mut e, keep), 2);
        assert_eq!(e.memory().restriction_nodes, 0);
        e.release_inspection(keep).unwrap();
        e.maintain(100_000);
    }

    #[test]
    fn cancellation_transfers_inflight_work_and_releases_all_cache_ownership() {
        let mut e = source();
        let choices: Vec<_> = e.births.keys().map(|&id| (id, true)).collect();
        let a = e.start_inspection(None, choices.clone()).unwrap();
        let b = e.start_inspection(None, choices.clone()).unwrap();
        // Start a shared Boolean job, then cancel its original reader.
        for _ in 0..100 {
            e.inspection_tick(a.0);
            if e.restrictions.jobs_started > 0 {
                break;
            }
        }
        assert_eq!(e.restrictions.jobs_started, 1);
        e.cancel_inspection(a).unwrap();
        for _ in 0..1000 {
            if e.discard_inspection(a, 1).unwrap() {
                break;
            }
        }
        assert!(e.inspection_status(a).unwrap().done);
        assert_eq!(e.memory().restriction_nodes, 3);
        e.request_collection();
        e.maintain(100_000);
        assert_eq!(drain(&mut e, b), 1);
        assert_eq!(e.restrictions.jobs_started, 6);
        assert_eq!(e.memory().restriction_nodes, 0);
        e.release_inspection(a).unwrap();
        e.release_inspection(b).unwrap();
        e.maintain(100_000);

        let c = e.start_inspection(None, choices.clone()).unwrap();
        let before = e.restrictions.jobs_started;
        for _ in 0..100 {
            e.inspection_tick(c.0);
            if e.restrictions.jobs_started > before {
                break;
            }
        }
        assert_eq!(e.restrictions.jobs_started, before + 1);
        e.cancel_inspection(c).unwrap();
        // Release the unstarted suffix and detach the first job for disposal.
        for _ in 0..100 {
            e.discard_inspection(c, 1).unwrap();
            if e.memory().restriction_nodes == 0 {
                break;
            }
        }
        assert_eq!(e.memory().restriction_nodes, 0);
        let d = e.start_inspection(None, choices).unwrap();
        e.request_collection();
        e.maintain(100_000);
        assert_eq!(drain(&mut e, d), 1);
        for _ in 0..1000 {
            if e.discard_inspection(c, 1).unwrap() {
                break;
            }
        }
        assert!(e.inspection_status(c).unwrap().done);
        assert_eq!(e.memory().restriction_nodes, 0);

        let all: Vec<_> = e.births.keys().map(|&id| (id, false)).collect();
        let last = e.start_inspection(None, all).unwrap();
        e.inspection_tick(last.0);
        e.cancel();
        for _ in 0..100_000 {
            if e.cancel_done() {
                break;
            }
            e.advance(1);
        }
        assert!(e.cancel_done());
        assert!(e.inspection_status(last).unwrap().canceled);
        assert_eq!(e.memory().restriction_nodes, 0);
        assert_eq!(e.memory().conditions, 0);
    }
    fn collect(e: &mut Engine) {
        e.request_collection();
        for _ in 0..100_000 {
            e.maintain(1);
            if !e.collecting() {
                return;
            }
        }
        panic!("collection must complete");
    }
    fn hold(e: &mut Engine, id: ViewId) {
        for _ in 0..100_000 {
            e.advance_inspection(id, 1).unwrap();
            if e.inspections[&id.0].output.is_some() {
                return;
            }
        }
        panic!("inspection must deliver");
    }
    #[test]
    fn late_shorter_longer_and_identical_readers_survive_pruned_prefixes_and_gc() {
        for stopped in [false, true] {
            let mut e = source();
            let snapshot = e.capture_snapshot().unwrap();
            if stopped {
                e.cancel();
                for _ in 0..100_000 {
                    e.advance(1);
                    if e.cancel_done() {
                        break;
                    }
                }
                assert!(e.cancel_done());
            }
            let choices: Vec<_> = e.births.keys().map(|&id| (id, true)).collect();
            let first = e.start_inspection(Some(snapshot), choices.clone()).unwrap();
            hold(&mut e, first);
            collect(&mut e);
            let before = e.restrictions.jobs_started;
            let same = e.start_inspection(Some(snapshot), choices.clone()).unwrap();
            assert_eq!(drain(&mut e, same), 1);
            assert_eq!(
                e.restrictions.jobs_started, before,
                "later identical reader reuses final computation"
            );
            e.release_inspection(same).unwrap();
            collect(&mut e);
            let short = e
                .start_inspection(Some(snapshot), choices[..1].to_vec())
                .unwrap();
            hold(&mut e, short);
            collect(&mut e);
            let before_middle = e.restrictions.jobs_started;
            let middle = e
                .start_inspection(Some(snapshot), choices[..2].to_vec())
                .unwrap();
            assert_eq!(drain(&mut e, middle), 2);
            assert_eq!(
                e.restrictions.jobs_started - before_middle,
                2,
                "longer reader uses the live shorter endpoint and computes only its extension"
            );
            assert_eq!(drain(&mut e, short), 4);
            assert_eq!(drain(&mut e, first), 1);
            for id in [first, short, middle] {
                e.release_inspection(id).unwrap();
            }
            collect(&mut e);
            assert_eq!(e.restrictions.len(), 0);
            let before_late = e.restrictions.jobs_started;
            let late = e
                .start_inspection(Some(snapshot), choices[..1].to_vec())
                .unwrap();
            assert_eq!(drain(&mut e, late), 4);
            assert_eq!(
                e.restrictions.jobs_started - before_late,
                2,
                "shorter demand after the longer reader fully finishes reconstructs safely"
            );
            e.release_inspection(late).unwrap();
            e.release_snapshot(snapshot).unwrap();
            collect(&mut e);
            assert_eq!(e.restrictions.len(), 0);
            e.cancel();
            for _ in 0..100_000 {
                e.advance(1);
                if e.cancel_done() {
                    break;
                }
            }
            collect(&mut e);
            assert_eq!(e.memory().conditions, 0);
        }
    }
    #[test]
    fn repeated_cancellation_at_every_selection_step_preserves_waiting_reader() {
        for stopped in [false, true] {
            for steps in 0..40 {
                let mut e = source();
                let snapshot = e.capture_snapshot().unwrap();
                if stopped {
                    e.cancel();
                    for _ in 0..100_000 {
                        e.advance(1);
                        if e.cancel_done() {
                            break;
                        }
                    }
                    assert!(e.cancel_done());
                }
                let choices: Vec<_> = e.births.keys().map(|&id| (id, true)).collect();
                let survivor = e.start_inspection(Some(snapshot), choices.clone()).unwrap();
                for _ in 0..3 {
                    let transient = e.start_inspection(Some(snapshot), choices.clone()).unwrap();
                    for _ in 0..steps {
                        e.advance_inspection(transient, 1).unwrap();
                        collect(&mut e);
                    }
                    e.cancel_inspection(transient).unwrap();
                    for _ in 0..1000 {
                        let done = e.discard_inspection(transient, 1).unwrap();
                        collect(&mut e);
                        if done {
                            break;
                        }
                    }
                    assert!(e.inspection_status(transient).unwrap().done);
                    e.release_inspection(transient).unwrap();
                    collect(&mut e);
                }
                assert_eq!(drain(&mut e, survivor), 1);
                e.release_inspection(survivor).unwrap();
                e.release_snapshot(snapshot).unwrap();
                collect(&mut e);
                assert_eq!(e.restrictions.len(), 0);
                e.cancel();
                for _ in 0..100_000 {
                    e.advance(1);
                    if e.cancel_done() {
                        break;
                    }
                }
                collect(&mut e);
                assert_eq!(e.memory().conditions, 0);
            }
        }
    }
}
