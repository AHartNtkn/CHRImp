//! Source cancellation keeps each task in its traced owner until discard ends.
use super::*;
use std::ops::Bound::{Excluded, Included, Unbounded};

#[derive(Default)]
enum Phase {
    #[default]
    Queue,
    Parked,
    Ready,
    Observe,
    Inspections,
    Waiters,
    Roots,
    Collect,
    Done,
}
#[derive(Default)]
pub(super) struct Cancellation {
    pub requested: bool,
    pub finished: bool,
    pub roots_released: bool,
    pub inspection_limit: Option<u64>,
    after_inspection: Option<u64>,
    phase: Phase,
}
impl Engine {
    /// O(1) request. No source task or primary projection advances after this.
    /// Views admitted later remain independently inspectable.
    pub fn cancel(&mut self) {
        if !self.cancellation.requested {
            self.cancellation.requested = true;
            self.cancellation.inspection_limit = self.latest_inspection;
        }
    }
    pub fn canceled(&self) -> bool {
        self.cancellation.requested
    }
    /// Source discard and its physical collection are complete. Held views may
    /// still own data; releasing them requests another maintenance collection.
    pub fn cancel_done(&self) -> bool {
        self.cancellation.finished && !self.collecting() && !self.release_pending()
    }
    /// Budgeted collection without source work or projection. An already
    /// running semantic collection may finish; new collections are physical.
    pub fn maintain(&mut self, budget: usize) {
        for _ in 0..budget {
            if self.release_store_tick() {
                continue;
            }
            let coordinates_done = self.cleanup_coordinates();
            if !self.collect_heap_mode(false) && coordinates_done {
                break;
            }
        }
    }
    pub(super) fn cancel_tick(&mut self) {
        if !self.discard_step_tick() {
            return;
        }
        match self.cancellation.phase {
            Phase::Queue => {
                if let Some(task) = self.queue.front_mut() {
                    if task.task.discard_tick() {
                        self.queue.pop_front();
                    }
                } else {
                    self.queue = VecDeque::new();
                    self.cancellation.phase = Phase::Parked;
                }
            }
            Phase::Parked => {
                if let Some(mut entry) = self.parked.first_entry() {
                    if entry.get_mut().task.discard_tick() {
                        entry.remove_entry();
                    }
                } else {
                    self.cancellation.phase = Phase::Ready;
                }
            }
            Phase::Ready => {
                if let Some(ready) = &mut self.ready {
                    if let Some(job) = &mut ready.job {
                        if job.discard_tick() {
                            ready.job = None;
                        }
                    } else if let Some(transport) = &mut ready.transport {
                        if transport.discard_tick() {
                            ready.transport = None;
                        }
                    } else {
                        self.ready = None;
                    }
                } else {
                    self.cancellation.phase = Phase::Observe;
                }
            }
            Phase::Observe => {
                if let Some(observer) = &mut self.observer {
                    if observer.discard_tick() {
                        self.observer = None;
                    }
                } else {
                    self.cancellation.phase = Phase::Inspections;
                }
            }
            Phase::Inspections => {
                let next = self.cancellation.inspection_limit.and_then(|last| {
                    self.inspections
                        .range((
                            self.cancellation
                                .after_inspection
                                .map_or(Unbounded, Excluded),
                            Included(last),
                        ))
                        .next()
                        .map(|(&id, _)| id)
                });
                if let Some(id) = next {
                    if self
                        .inspections
                        .get_mut(&id)
                        .unwrap()
                        .discard_tick(&mut self.restrictions)
                    {
                        self.cancellation.after_inspection = Some(id);
                    }
                } else {
                    self.cancellation.phase = Phase::Waiters;
                }
            }
            Phase::Waiters => {
                self.waiting = VecDeque::new(); // Scalar owner IDs only.
                if self.requested.pop_first().is_none() {
                    self.lane = None;
                    self.cancellation.phase = Phase::Roots;
                }
            }
            Phase::Roots => {
                self.state = StateRoot {
                    graph: self.graph.empty(),
                    history: self.history.empty(),
                };
                self.pending_root = self.obligations.empty();
                self.active = Condition::FALSE;
                self.variables = Arc::new(Vec::new());
                self.cancellation.roots_released = true;
                self.request_collection();
                self.cancellation.phase = Phase::Collect;
            }
            Phase::Collect => {
                // advance services every outstanding collection before reaching
                // here, including its leases and final root walk.
                debug_assert!(!self.collecting());
                if !self.cleanup_coordinates() {
                    return;
                }
                self.cancellation.finished = true;
                self.cancellation.phase = Phase::Done;
            }
            Phase::Done => self.service_inspection(),
        }
    }
}
impl Task {
    pub(super) fn discard_tick(&mut self) -> bool {
        match self {
            Task::Init(vars) => {
                *vars = Vec::new();
                true
            }
            Task::Activate { .. } => true,
            Task::Wake(wake) => wake.discard_tick(),
            Task::Body(body) => {
                if let Some(rejection) = &mut body.rejection {
                    if rejection.discard_tick() {
                        body.rejection = None;
                    }
                    return false;
                }
                if let Some(dispatch) = &mut body.dispatch {
                    if dispatch.discard_tick() {
                        body.dispatch = None;
                    }
                    return false;
                }
                if let Some(n) = &mut body.normalizer {
                    if n.discard_tick() {
                        body.normalizer = None;
                    }
                    return false;
                }
                if let Some(job) = &mut body.job {
                    if job.discard_tick() {
                        body.job = None;
                    }
                    return false;
                }
                if let Some(merge) = &mut body.merge {
                    if merge.discard_tick() {
                        body.merge = None;
                    }
                    return false;
                }
                body.args = Vec::new();
                body.variables = Arc::new(Vec::new());
                body.update = None;
                true
            }
            Task::Search(search) => {
                if let Some(commit) = &mut search.commit {
                    if commit.discard_tick() {
                        search.commit = None;
                    }
                    return false;
                }
                if let Some(transport) = &mut search.transport {
                    if transport.discard_tick() {
                        search.transport = None;
                    }
                    return false;
                }
                if !search.matches.discard_tick() {
                    return false;
                }
                search.candidate = None; // Match contains scalar vectors only.
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_does_not_wait_for_a_reserved_semantic_lane() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        // A collector has been handed the FIFO lane but has not started yet.
        e.lane = Some(Owner::Collection);
        e.cancel();
        for _ in 0..1000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert!(e.lane.is_none() && e.waiting.is_empty() && e.requested.is_empty());
    }
}
