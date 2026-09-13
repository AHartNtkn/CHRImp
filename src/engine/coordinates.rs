//! Coordinate ownership for immutable readers across causal compaction.
use super::*;
use crate::condition::Transform;
use crate::gc::discard_slot;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::sync::Weak;

#[derive(Clone)]
pub(super) struct Epoch {
    id: u64,
    lease: Arc<()>,
}
pub(super) struct Coordinates {
    current: Epoch,
    readers: BTreeMap<u64, Weak<()>>,
    changes: BTreeMap<u64, Arc<BTreeMap<u64, Condition>>>,
    draining: BTreeMap<u64, Condition>,
    assignment_count: usize,
}
impl Default for Coordinates {
    fn default() -> Self {
        let current = Epoch {
            id: 0,
            lease: Arc::new(()),
        };
        Self {
            readers: BTreeMap::from([(0, Arc::downgrade(&current.lease))]),
            current,
            changes: BTreeMap::new(),
            draining: BTreeMap::new(),
            assignment_count: 0,
        }
    }
}
impl Coordinates {
    pub(super) fn current(&self) -> Epoch {
        self.current.clone()
    }
    pub(super) fn publish(&mut self, bindings: Arc<BTreeMap<u64, Condition>>) -> Epoch {
        self.assignment_count = self
            .assignment_count
            .checked_add(bindings.len())
            .expect("coordinate accounting overflow");
        self.changes.insert(self.current.id, bindings);
        self.current = Epoch {
            id: self
                .current
                .id
                .checked_add(1)
                .expect("coordinate epoch exhausted"),
            lease: Arc::new(()),
        };
        self.readers
            .insert(self.current.id, Arc::downgrade(&self.current.lease));
        self.current()
    }
    pub(super) fn transport(&self, input: Condition, from: &Epoch) -> Transport {
        Transport {
            #[cfg(feature = "diagnostics")]
            measured_transform: Default::default(),
            _from: from.clone(),
            next: from.id,
            target: self.current(),
            value: input,
            job: None,
            discarding: false,
        }
    }
    pub(super) fn memory(&self) -> usize {
        self.changes
            .len()
            .saturating_add(self.readers.len())
            .saturating_add(self.assignment_count)
            .saturating_add(self.draining.len())
    }
    pub(super) fn cleanup_tick(&mut self) -> bool {
        if self.draining.pop_first().is_some() {
            return false;
        }
        let Some((&id, lease)) = self.readers.first_key_value() else {
            return true;
        };
        if lease.strong_count() != 0 {
            return self.changes.is_empty();
        }
        self.readers.pop_first();
        if let Some(bindings) = self.changes.remove(&id) {
            self.assignment_count -= bindings.len();
            if let Some(bindings) = Arc::into_inner(bindings) {
                self.draining = bindings;
            }
        }
        false
    }
}
// The epoch log owns images even when no transport has started using them.
// Visit one epoch boundary or one image per trace step, including drain state.
// Cleanup may retire maps during tracing; the child cursor belongs to one epoch.
struct Images<'a>(&'a BTreeMap<u64, Condition>);
impl Trace for Images<'_> {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => cursor.values(self.0),
            _ => Step::Done,
        }
    }
}
impl Trace for Coordinates {
    fn trace(&self, cursor: &mut TraceCursor) -> Step {
        match cursor.phase {
            0 => {
                if cursor.slot == 0 {
                    let next = match cursor.key {
                        Some(epoch) => self
                            .changes
                            .range((std::ops::Bound::Excluded(epoch), std::ops::Bound::Unbounded))
                            .next(),
                        None => self.changes.first_key_value(),
                    };
                    let Some((&epoch, _)) = next else {
                        return cursor.advance();
                    };
                    cursor.key = Some(epoch);
                    cursor.slot = 1;
                }
                let epoch = cursor.key.expect("coordinate trace epoch");
                let step = if let Some(images) = self.changes.get(&epoch) {
                    cursor.optional(Some(&Images(images)))
                } else {
                    // A removed map must never lend its child position to the
                    // next map, even when both maps contain the same choice IDs.
                    cursor.advance()
                };
                if cursor.phase == 1 {
                    cursor.phase = 0;
                    cursor.key = Some(epoch);
                    cursor.slot = 0;
                }
                step
            }
            1 => cursor.values(&self.draining),
            _ => Step::Done,
        }
    }
}

pub(super) struct Transport {
    #[cfg(feature = "diagnostics")]
    measured_transform: diagnostics::ConditionalWork,
    _from: Epoch,
    next: u64,
    target: Epoch,
    value: Condition,
    job: Option<Transform>,
    discarding: bool,
}
impl Transport {
    pub(super) fn tick(&mut self, arena: &mut Arena, coordinates: &Coordinates) -> Progress {
        assert!(!self.discarding, "discarded coordinate transport");
        if self.next == self.target.id {
            return Progress::Complete(self.value);
        }
        if let Some(job) = &mut self.job {
            if let Progress::Complete(value) = measured_tick!(job, arena, self.measured_transform) {
                self.value = value;
                self.job = None;
                self.next += 1;
            }
        } else {
            self.job = Some(
                arena.substitute(
                    self.value,
                    coordinates
                        .changes
                        .get(&self.next)
                        .expect("leased coordinate transition")
                        .clone(),
                ),
            );
        }
        Progress::Pending
    }
    pub(super) fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.value = Condition::FALSE;
        if discard_slot(&mut self.job, |child| child.discard_tick()) {
            return false;
        }
        true
    }
}

impl Trace for Transport {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.value]),
            1 => c.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(t: &mut Transport, a: &mut Arena, c: &Coordinates) -> Condition {
        for _ in 0..10000 {
            if let Progress::Complete(v) = t.tick(a, c) {
                return v;
            }
        }
        panic!("transport did not finish");
    }
    #[test]
    fn old_reader_crosses_multiple_publications_without_changing_target() {
        let mut a = Arena::default();
        let (x_id, x) = a.fresh_choice();
        let (y_id, y) = a.fresh_choice();
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(x_id, Condition::FALSE)])));
        let mut first = c.transport(y, &old);
        c.publish(Arc::new(BTreeMap::from([(y_id, Condition::TRUE)])));
        assert_eq!(finish(&mut first, &mut a, &c), y);
        assert_eq!(
            finish(&mut c.transport(x, &old), &mut a, &c),
            Condition::FALSE
        );
        assert_eq!(
            finish(&mut c.transport(y, &old), &mut a, &c),
            Condition::TRUE
        );
    }
    #[test]
    fn log_retention_follows_reader_leases() {
        let mut a = Arena::default();
        let (id, _) = a.fresh_choice();
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        for _ in 0..8 {
            c.cleanup_tick();
        }
        assert_eq!(c.changes.len(), 1);
        drop(old);
        for _ in 0..8 {
            c.cleanup_tick();
        }
        assert!(c.changes.is_empty());
        assert!(c.draining.is_empty());
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    use crate::syntax::{parse_program, parse_query};
    #[test]
    fn immutable_matcher_candidates_use_current_coordinates_before_commit() {
        for positive in [false, true] {
            let code = crate::program::prepare(
                &parse_program("p() ==> result().").unwrap(),
                &parse_query("p()").unwrap(),
            )
            .unwrap();
            let mut e = Engine::new(Arc::new(code));
            e.queue.clear();
            e.pending_root = e.obligations.empty();
            let mut post = e
                .graph
                .post(e.state.graph, 0, vec![], Condition::TRUE)
                .unwrap();
            e.state.graph = loop {
                if let UpdateStatus::Complete(root) = post.tick(&mut e.graph) {
                    break root;
                }
            };
            let (choice, x) = e.arena.fresh_choice();
            let scope = if positive { x } else { x.not() };
            let matches = Matches::new(
                &e.graph,
                e.state.graph.clone(),
                e.code.clone(),
                0,
                scope,
                None,
            )
            .unwrap();
            e.spawn(
                Condition::TRUE,
                Task::Search(Box::new(Search {
                    rule: 0,
                    matches: super::Discovery::Indexed(Box::new(matches)),
                    candidate: None,
                    commit: None,
                    transport: None,
                })),
            );
            e.coordinates
                .publish(Arc::new(BTreeMap::from([(choice, Condition::FALSE)])));
            for _ in 0..10000 {
                e.advance(1);
                e.take_output();
                if e.delivery_done() && e.pending_tasks() == 0 {
                    break;
                }
            }
            assert_eq!(e.applications(), u64::from(!positive));
            assert!(e.delivery_done());
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn collect_transport(a: &mut Arena, transport: &Transport) {
        let mut cursor = TraceCursor::default();
        let mut roots = vec![];
        for _ in 0..10000 {
            match transport.trace(&mut cursor) {
                Step::Root(root) => roots.push(root),
                Step::Pending => {}
                Step::Done => {
                    let mut gc = a.collect(roots.into_iter());
                    for _ in 0..10000 {
                        if gc.tick(a) {
                            return;
                        }
                    }
                    panic!("collector did not finish");
                }
            }
        }
        panic!("trace did not finish");
    }
    #[test]
    fn transport_survives_collection_at_every_restriction_tick() {
        let mut a = Arena::default();
        let (id, x) = a.fresh_choice();
        let (_, y) = a.fresh_choice();
        let mut and = a.start(Operation::And(x, y));
        let input = loop {
            if let Progress::Complete(c) = and.tick(&mut a) {
                break c;
            }
        };
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        let mut t = c.transport(input, &old);
        for _ in 0..10000 {
            collect_transport(&mut a, &t);
            if let Progress::Complete(result) = t.tick(&mut a, &c) {
                assert_eq!(result, y);
                return;
            }
        }
        panic!("transport did not finish");
    }
    #[test]
    fn certificate_is_transported_before_current_active_intersection() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let (id, x) = e.arena.fresh_choice();
        e.ready = Some(Ready {
            epoch: e.coordinates.current(),
            transport: None,
            cursor: e
                .obligations
                .index
                .range(e.pending_root.clone(), [0; 4], [u64::MAX; 4]),
            blocked: Condition::FALSE,
            scope: x,
            job: None,
            phase: ReadyPhase::Acquire,
        });
        e.coordinates
            .publish(Arc::new(BTreeMap::from([(id, Condition::FALSE)])));
        for _ in 0..10000 {
            e.completion();
            if e.ready.is_none() {
                assert!(e.observer.is_none());
                assert!(e.lane.is_none());
                assert_eq!(e.active, Condition::TRUE);
                return;
            }
        }
        panic!("certificate did not finish");
    }
    #[test]
    fn cancellation_drains_an_inflight_transport_and_coordinate_log() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let (id, x) = e.arena.fresh_choice();
        let epoch = e.coordinates.current();
        e.coordinates
            .publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        let mut transport = e.coordinates.transport(x, &epoch);
        assert!(matches!(
            transport.tick(&mut e.arena, &e.coordinates),
            Progress::Pending
        ));
        e.ready = Some(Ready {
            epoch,
            transport: Some(transport),
            cursor: e
                .obligations
                .index
                .range(e.pending_root.clone(), [0; 4], [u64::MAX; 4]),
            blocked: Condition::FALSE,
            scope: x,
            job: None,
            phase: ReadyPhase::Transport,
        });
        e.cancel();
        for _ in 0..10000 {
            e.advance(1);
            if e.cancel_done() {
                assert_eq!(e.coordinates.memory(), 1);
                return;
            }
        }
        panic!("cancellation did not drain transport");
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn old_finite_join_releases_broad_pending_scope_beside_divergence() {
        let program =
            "start() <=> (fail;true). finite(),p(X),q(Y) ==> pair(X,Y). loop() <=> loop().";
        let mut query = String::from("start(),(finite();loop())");
        for i in 0..4 {
            query.push_str(&format!(",p(P{i})"));
        }
        for i in 0..8 {
            query.push_str(&format!(",q(Q{i})"));
        }
        let code = crate::program::prepare(
            &crate::syntax::parse_program(program).unwrap(),
            &crate::syntax::parse_query(&query).unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let pair = e
            .code
            .signatures
            .iter()
            .position(|s| s.name == "pair")
            .unwrap();
        let mut pairs = 0;
        let mut answers = 0;
        let mut old_reader = false;
        for tick in 0..2_000_000 {
            if tick % 3001 == 0 && !e.collecting() {
                e.request_collection();
            }
            e.advance(1);
            old_reader |= e.queue.iter().chain(e.parked.values()).any(|s| {
                s.epoch
                    .as_ref()
                    .is_some_and(|epoch| epoch.id < e.coordinates.current.id)
            });
            match e.take_output() {
                Some(Output::Fact { relation, .. }) if relation == pair => pairs += 1,
                Some(Output::End) => {
                    answers += 1;
                    break;
                }
                _ => {}
            }
        }
        assert!(
            old_reader,
            "exercise a reader across compaction publication"
        );
        assert_eq!(
            answers, 1,
            "finite answer must survive continuing source work"
        );
        assert_eq!(pairs, 32, "every finite join tuple is preserved");
        assert!(!e.exhausted());
    }
}

#[cfg(test)]
mod maintenance_tests {
    use super::*;
    #[test]
    fn maintenance_spends_its_budget_draining_unowned_coordinate_records() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut assignments = BTreeMap::new();
        for _ in 0..64 {
            let (id, _) = e.arena.fresh_choice();
            assignments.insert(id, Condition::TRUE);
        }
        e.coordinates.publish(Arc::new(assignments));
        assert_eq!(e.coordinates.memory(), 67);
        e.maintain(1);
        assert_eq!(e.coordinates.memory(), 65);
        e.maintain(128);
        assert_eq!(e.coordinates.memory(), 1);
        assert_eq!(e.applications(), 0);
    }
}

#[cfg(test)]
mod rejected_reader_tests {
    use super::*;
    #[test]
    fn false_pending_scope_discards_a_large_old_join_without_enumerating_it() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("p(X),q(Y) ==> result(X,Y).").unwrap(),
            &crate::syntax::parse_query("p(A),q(B)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        e.queue.clear();
        e.pending_root = e.obligations.empty();
        for relation in 0..2 {
            for _ in 0..64 {
                let mut post = e
                    .graph
                    .post(
                        e.state.graph,
                        relation,
                        vec![e.ids.variable()],
                        Condition::TRUE,
                    )
                    .unwrap();
                e.state.graph = loop {
                    if let UpdateStatus::Complete(root) = post.tick(&mut e.graph) {
                        break root;
                    }
                };
            }
        }
        let (choice, x) = e.arena.fresh_choice();
        let matches =
            Matches::new(&e.graph, e.state.graph.clone(), e.code.clone(), 0, x, None).unwrap();
        e.spawn(
            x,
            Task::Search(Box::new(Search {
                rule: 0,
                matches: super::Discovery::Indexed(Box::new(matches)),
                candidate: None,
                commit: None,
                transport: None,
            })),
        );
        e.coordinates
            .publish(Arc::new(BTreeMap::from([(choice, Condition::FALSE)])));
        e.queue.front_mut().unwrap().scope = Condition::FALSE;
        // Exercise task service directly so unrelated physical pressure and
        // projection do not count against the reader's discard budget.
        for _ in 0..128 {
            if let Some(mut task) = e.queue.pop_front() {
                if e.task(&mut task) {
                    e.pending_root = e
                        .obligations
                        .index
                        .remove(e.pending_root, &[task.id, 0, 0, 0]);
                } else {
                    e.queue.push_back(task);
                }
            }
            e.coordinates.cleanup_tick();
        }
        assert_eq!(
            e.pending_tasks(),
            0,
            "dead reader must discard instead of enumerating 4096 tuples"
        );
        assert_eq!(e.coordinates.memory(), 1);
        assert_eq!(e.applications(), 0);
    }
}

#[cfg(test)]
mod functional_image_tests {
    use super::*;

    fn boolean(arena: &mut Arena, operation: Operation) -> Condition {
        let mut job = arena.start(operation);
        loop {
            if let Progress::Complete(value) = job.tick(arena) {
                return value;
            }
        }
    }
    fn collect(arena: &mut Arena, coordinates: &Coordinates, transports: &[&Transport]) {
        let mut roots = vec![];
        for traced in std::iter::once(coordinates as &dyn Trace)
            .chain(transports.iter().map(|t| *t as &dyn Trace))
        {
            let mut cursor = TraceCursor::default();
            for _ in 0..10000 {
                match traced.trace(&mut cursor) {
                    Step::Root(root) => roots.push(root),
                    Step::Pending => {}
                    Step::Done => break,
                }
            }
        }
        let mut gc = arena.collect(roots.into_iter());
        while !gc.tick(arena) {}
    }
    #[test]
    fn functional_images_compose_across_epochs_with_gc_at_every_transport_tick() {
        let mut arena = Arena::default();
        let (_, z) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let (xi, x) = arena.fresh_choice();
        let image = boolean(&mut arena, Operation::Or(y, z));
        let mut coordinates = Coordinates::default();
        let origin = coordinates.current();
        coordinates.publish(Arc::new(BTreeMap::from([(xi, image)])));
        let mut first = coordinates.transport(x, &origin);
        coordinates.publish(Arc::new(BTreeMap::from([(yi, z.not())])));
        let mut second = coordinates.transport(x, &origin);
        // First transport has a frozen target and must not apply the later map.
        let first_result = (0..10000)
            .find_map(|_| {
                collect(&mut arena, &coordinates, &[&first, &second]);
                match first.tick(&mut arena, &coordinates) {
                    Progress::Complete(value) => Some(value),
                    Progress::Pending => None,
                }
            })
            .expect("first epoch transport");
        assert_eq!(first_result, image);
        let result = (0..10000)
            .find_map(|_| {
                collect(&mut arena, &coordinates, &[&second]);
                match second.tick(&mut arena, &coordinates) {
                    Progress::Complete(value) => Some(value),
                    Progress::Pending => None,
                }
            })
            .expect("composed transport");
        assert_eq!(result, Condition::TRUE);
    }
    #[test]
    fn unstarted_transport_images_and_incremental_log_drain_are_roots() {
        let mut arena = Arena::default();
        let (_, y) = arena.fresh_choice();
        let (_, z) = arena.fresh_choice();
        let (xi, _) = arena.fresh_choice();
        let image = boolean(&mut arena, Operation::And(y, z));
        let mut coordinates = Coordinates::default();
        let origin = coordinates.current();
        coordinates.publish(Arc::new(BTreeMap::from([(xi, image)])));
        collect(&mut arena, &coordinates, &[]);
        assert!(arena.contains(image));
        drop(origin);
        coordinates.cleanup_tick();
        assert_eq!(coordinates.draining.len(), 1);
        collect(&mut arena, &coordinates, &[]);
        assert!(arena.contains(image));
        coordinates.cleanup_tick();
        collect(&mut arena, &coordinates, &[]);
        assert!(!arena.contains(image));
    }
    #[test]
    fn engine_collection_roots_functional_images_during_log_cleanup() {
        let code = crate::program::prepare(
            &crate::syntax::parse_program("").unwrap(),
            &crate::syntax::parse_query("true").unwrap(),
        )
        .unwrap();
        let mut engine = Engine::new(Arc::new(code));
        engine.queue.clear();
        engine.pending_root = engine.obligations.empty();
        let (_, y) = engine.arena.fresh_choice();
        let (_, z) = engine.arena.fresh_choice();
        let (xi, _) = engine.arena.fresh_choice();
        let image = boolean(&mut engine.arena, Operation::And(y, z));
        engine
            .coordinates
            .publish(Arc::new(BTreeMap::from([(xi, Condition::TRUE)])));
        let reader = engine.coordinates.current();
        engine
            .coordinates
            .publish(Arc::new(BTreeMap::from([(xi, image)])));
        // Start the actual physical collector before ordinary cleanup. The
        // first map is unleased; the second map alone owns the image.
        engine.request_collection();
        assert!(engine.collect_heap_mode(false));
        assert!(engine.collector.is_some());
        let before = engine.collections();
        for tick in 0..10000 {
            engine.coordinates.cleanup_tick();
            if tick % 2 == 0 {
                engine.advance(1);
            } else {
                engine.maintain(1);
            }
            if engine.collector.is_none() {
                break;
            }
        }
        assert!(engine.collector.is_none());
        assert_eq!(engine.collections(), before + 1);
        assert!(
            engine.arena.contains(image),
            "only the coordinate log owns this image"
        );
        engine.maintain(1);
        assert_eq!(engine.coordinates.changes.len(), 1);
        drop(reader);
        engine.maintain(100);
        engine.request_collection();
        engine.maintain(10000);
        assert!(!engine.collecting());
        assert!(!engine.arena.contains(image));
    }
    #[test]
    fn retiring_a_partly_traced_epoch_resets_the_next_maps_child_cursor() {
        let mut arena = Arena::default();
        let (_, y) = arena.fresh_choice();
        let (_, z) = arena.fresh_choice();
        let (xi, _) = arena.fresh_choice();
        let (wi, _) = arena.fresh_choice();
        let image = boolean(&mut arena, Operation::And(y, z));
        let mut coordinates = Coordinates::default();
        let oldest = coordinates.current();
        coordinates.publish(Arc::new(BTreeMap::from([
            (xi, Condition::TRUE),
            (wi, Condition::FALSE),
        ])));
        let reader = coordinates.current();
        coordinates.publish(Arc::new(BTreeMap::from([(xi, image), (wi, image.not())])));
        let mut cursor = TraceCursor::default();
        let mut roots = vec![];
        loop {
            if let Step::Root(root) = coordinates.trace(&mut cursor) {
                roots.push(root);
                break;
            }
        }
        assert_eq!(roots, vec![Condition::TRUE]);
        drop(oldest);
        coordinates.cleanup_tick();
        assert!(!coordinates.changes.contains_key(&0));
        let mut done = false;
        for _ in 0..100 {
            coordinates.cleanup_tick();
            match coordinates.trace(&mut cursor) {
                Step::Root(root) => roots.push(root),
                Step::Pending => {}
                Step::Done => {
                    done = true;
                    break;
                }
            }
        }
        assert!(done);
        assert!(roots.contains(&image));
        assert!(roots.contains(&image.not()));
        let mut gc = arena.collect(roots.into_iter());
        while !gc.tick(&mut arena) {}
        assert!(arena.contains(image));
        drop(reader);
    }
}

impl Engine {
    pub(super) fn cleanup_coordinates(&mut self) -> bool {
        #[cfg(feature = "diagnostics")]
        let before = (
            self.coordinates.readers.len(),
            self.coordinates.changes.len(),
            !self.coordinates.draining.is_empty(),
        );
        let done = self.coordinates.cleanup_tick();
        #[cfg(feature = "diagnostics")]
        {
            let d = &mut self.diagnostics.shared.coordinates;
            d.cleanup_probes += 1;
            d.epochs_retired += (before.0 - self.coordinates.readers.len()) as u64;
            d.maps_retired += (before.1 - self.coordinates.changes.len()) as u64;
            d.assignments_drained += u64::from(before.2);
        }
        done
    }
    pub(super) fn transport_tick(&mut self, transport: &mut Transport, search: bool) -> Progress {
        #[cfg(not(feature = "diagnostics"))]
        let _ = search;
        #[cfg(feature = "diagnostics")]
        let before = transport.next;
        let result = transport.tick(&mut self.arena, &self.coordinates);
        #[cfg(feature = "diagnostics")]
        {
            let d = if search {
                &mut self.diagnostics.shared.coordinates.search
            } else {
                &mut self.diagnostics.shared.coordinates.completion
            };
            d.calls += 1;
            d.epochs_crossed += transport.next - before;
            let delta = std::mem::take(&mut transport.measured_transform);
            d.transform.calls += delta.calls;
            d.transform.work += delta.work;
        }
        result
    }
}
