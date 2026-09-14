//! Coordinate ownership for immutable readers across causal compaction.
use super::*;
use crate::condition::Transform;
use crate::gc::discard_slot;
use crate::trace::{Cursor as TraceCursor, Step, Trace};
use std::cell::{Cell, RefCell};
use std::sync::Weak;

type Map = Arc<BTreeMap<u64, Condition>>;
const SEGMENT_EPOCHS: usize = 8;

// A single publication needs no run allocation. Runs append immutable maps;
// retirement empties slots without changing any remaining epoch's identity.
enum Segment {
    Single(Map),
    Run(Box<Run>),
}
struct Run {
    deltas: Vec<Option<Map>>,
}
struct Composition {
    from: u64,
    to: u64,
    images: Map,
}
// One completed exact prefix across all segments. It never owns an epoch lease:
// the first retired map in its interval invalidates it. Trace both Boolean roots
// until then, even when the original transport has been dropped.
#[derive(Clone, Copy)]
struct Prefix {
    from: u64,
    to: u64,
    input: Condition,
    output: Condition,
}
#[derive(Default)]
struct Changes {
    segments: BTreeMap<u64, Segment>,
    len: usize,
    runs: usize,
    composition_records: Cell<usize>,
    // One immutable recent image, bounded independently of reader age/count.
    composition: RefCell<Option<Composition>>,
    #[cfg(feature = "diagnostics")]
    diagnostics: RefCell<diagnostics::SegmentDiagnostics>,
}
impl Segment {
    fn span(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Run(run) => run.deltas.len(),
        }
    }
    fn get(&self, offset: usize) -> Option<&Map> {
        match self {
            Self::Single(map) => (offset == 0).then_some(map),
            Self::Run(run) => run.deltas.get(offset)?.as_ref(),
        }
    }
}
impl Changes {
    fn len(&self) -> usize {
        self.len
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn insert(&mut self, epoch: u64, map: Map) {
        if let Some((&start, segment)) = self.segments.last_key_value()
            && start + segment.span() as u64 == epoch
            && segment.span() < SEGMENT_EPOCHS
        {
            let segment = self.segments.get_mut(&start).unwrap();
            match segment {
                Segment::Single(first) => {
                    *segment = Segment::Run(Box::new(Run {
                        deltas: vec![Some(first.clone()), Some(map)],
                    }));
                    self.runs += 1;
                }
                Segment::Run(run) => run.deltas.push(Some(map)),
            }
        } else {
            self.segments.insert(epoch, Segment::Single(map));
            #[cfg(feature = "diagnostics")]
            {
                self.diagnostics.borrow_mut().segments_created += 1;
            }
        }
        self.len += 1;
    }
    fn get(&self, epoch: &u64) -> Option<&Map> {
        let (&start, segment) = self.segments.range(..=epoch).next_back()?;
        segment.get((*epoch - start) as usize)
    }
    fn next(&self, after: Option<u64>) -> Option<(u64, &Map)> {
        let begin = after.map_or(0, |epoch| epoch + 1);
        // At most one segment can begin before the requested epoch.
        let start = self
            .segments
            .range(..=begin)
            .next_back()
            .map_or(begin, |(&id, _)| id);
        for (&id, segment) in self.segments.range(start..) {
            for offset in begin.saturating_sub(id) as usize..segment.span() {
                if let Some(map) = segment.get(offset) {
                    return Some((id + offset as u64, map));
                }
            }
        }
        None
    }
    fn remove(&mut self, epoch: &u64) -> Option<Map> {
        self.get(epoch)?;
        if self
            .composition
            .get_mut()
            .as_ref()
            .is_some_and(|c| c.from <= *epoch && *epoch < c.to)
        {
            let c = self.composition.get_mut().take().unwrap();
            self.composition_records.set(0);
            #[cfg(not(feature = "diagnostics"))]
            let _ = c;
            #[cfg(feature = "diagnostics")]
            {
                let d = &mut *self.diagnostics.borrow_mut();
                d.invalidations += 1;
                d.retained_compositions -= 1;
                d.retained_composition_assignments -= c.images.len();
            }
        }
        let (&start, _) = self.segments.range(..=epoch).next_back()?;
        let segment = self.segments.get_mut(&start).unwrap();
        let (map, empty) = match segment {
            Segment::Single(_) => {
                if *epoch != start {
                    return None;
                }
                let Segment::Single(map) = self.segments.remove(&start).unwrap() else {
                    unreachable!()
                };
                self.len -= 1;
                #[cfg(feature = "diagnostics")]
                {
                    self.diagnostics.borrow_mut().segments_retired += 1;
                }
                return Some(map);
            }
            Segment::Run(run) => {
                let map = run.deltas.get_mut((*epoch - start) as usize)?.take()?;
                (map, run.deltas.iter().all(Option::is_none))
            }
        };
        if empty {
            self.segments.remove(&start);
            self.runs -= 1;
            #[cfg(feature = "diagnostics")]
            {
                self.diagnostics.borrow_mut().segments_retired += 1;
            }
        }
        self.len -= 1;
        Some(map)
    }
    fn images(&self, from: u64, target: u64) -> (Map, u64) {
        let (&start, segment) = self
            .segments
            .range(..=from)
            .next_back()
            .expect("leased coordinate segment");
        let fallback = || {
            (
                self.get(&from)
                    .expect("leased coordinate transition")
                    .clone(),
                from + 1,
            )
        };
        let Segment::Run(run) = segment else {
            return fallback();
        };
        let to = target.min(start + run.deltas.len() as u64);
        if to <= from + 1 {
            return fallback();
        }
        if let Some(c) = self.composition.borrow().as_ref()
            && c.from == from
            && c.to == to
        {
            #[cfg(feature = "diagnostics")]
            {
                self.diagnostics.borrow_mut().segment_hits += 1;
            }
            return (c.images.clone(), to);
        }
        // A bounded exact prefix. Constants survive subsequent substitution;
        // the first assignment to a repeated key therefore wins. Functional
        // images and nonsingleton maps retain the per-epoch transport path.
        let mut entries = [(0, Condition::FALSE); SEGMENT_EPOCHS];
        for (slot, epoch) in (from..to).enumerate() {
            #[cfg(feature = "diagnostics")]
            {
                self.diagnostics.borrow_mut().composition_probes += 1;
            }
            let map = segment
                .get((epoch - start) as usize)
                .expect("pinned interior cutoff");
            if map.len() != 1 {
                return fallback();
            }
            let (&id, &image) = map.first_key_value().unwrap();
            if !image.is_terminal() {
                return fallback();
            }
            entries[slot] = (id, image);
        }
        let mut images = BTreeMap::new();
        for &(id, image) in &entries[..(to - from) as usize] {
            images.entry(id).or_insert(image);
        }
        let images = Arc::new(images);
        let previous = self.composition.borrow_mut().replace(Composition {
            from,
            to,
            images: images.clone(),
        });
        let old_records = previous.as_ref().map_or(0, |c| 1 + c.images.len());
        self.composition_records
            .set(self.composition_records.get() + 1 + images.len() - old_records);
        #[cfg(feature = "diagnostics")]
        {
            let d = &mut *self.diagnostics.borrow_mut();
            d.compositions_built += 1;
            d.composition_entries += to - from;
            d.invalidations += u64::from(previous.is_some());
            d.retained_compositions += usize::from(previous.is_none());
            d.retained_composition_assignments += images.len();
            d.retained_composition_assignments -= previous.as_ref().map_or(0, |c| c.images.len());
        }
        (images, to)
    }
}

#[derive(Clone)]
pub(super) struct Epoch {
    id: u64,
    lease: Arc<()>,
}
pub(super) struct Coordinates {
    current: Epoch,
    readers: BTreeMap<u64, Weak<()>>,
    changes: Changes,
    draining: BTreeMap<u64, Condition>,
    assignment_count: usize,
    prefix: Cell<Option<Prefix>>,
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
            changes: Changes::default(),
            draining: BTreeMap::new(),
            assignment_count: 0,
            prefix: Cell::new(None),
        }
    }
}
impl Coordinates {
    fn reuse(&self, from: u64, target: u64, input: Condition) -> Option<(Condition, u64)> {
        #[cfg(feature = "diagnostics")]
        {
            self.changes.diagnostics.borrow_mut().prefix_probes += 1;
        }
        let prefix = self.prefix.get()?;
        if prefix.from != from || prefix.to > target || prefix.input != input {
            return None;
        }
        #[cfg(feature = "diagnostics")]
        {
            self.changes.diagnostics.borrow_mut().prefix_hits += 1;
        }
        Some((prefix.output, prefix.to))
    }
    fn remember(&self, from: u64, to: u64, input: Condition, output: Condition) {
        let previous = self.prefix.replace(Some(Prefix {
            from,
            to,
            input,
            output,
        }));
        #[cfg(not(feature = "diagnostics"))]
        let _ = previous;
        #[cfg(feature = "diagnostics")]
        {
            let d = &mut *self.changes.diagnostics.borrow_mut();
            d.prefix_publications += 1;
            d.prefix_invalidations += u64::from(previous.is_some());
        }
    }
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
            job_target: from.id,
            target: self.current(),
            value: input,
            job: None,
            discarding: false,
        }
    }
    pub(super) fn memory(&self) -> usize {
        // Packing does not remove the retained maps or their assignments.
        // Preserve their counts and charge the additional run/cache records.
        self.changes
            .len()
            .saturating_add(self.changes.runs)
            .saturating_add(self.readers.len())
            .saturating_add(self.assignment_count)
            .saturating_add(self.draining.len())
            .saturating_add(self.changes.composition_records.get())
            .saturating_add(usize::from(self.prefix.get().is_some()))
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
        if self.prefix.get().is_some_and(|p| p.from <= id && id < p.to) {
            self.prefix.set(None);
            #[cfg(feature = "diagnostics")]
            {
                self.changes.diagnostics.borrow_mut().prefix_invalidations += 1;
            }
        }
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
// Composition images are exclusively immortal Boolean constants. General
// functional images remain owned/traced by their original exact epoch maps.
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
                    let Some((epoch, _)) = self.changes.next(cursor.key) else {
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
            1 => match self.prefix.get() {
                Some(prefix) => cursor.fields(&[prefix.input, prefix.output]),
                None => cursor.advance(),
            },
            2 => cursor.values(&self.draining),
            _ => Step::Done,
        }
    }
}

pub(super) struct Transport {
    #[cfg(feature = "diagnostics")]
    measured_transform: diagnostics::ConditionalWork,
    _from: Epoch,
    next: u64,
    job_target: u64,
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
                coordinates.remember(self.next, self.job_target, self.value, value);
                self.value = value;
                self.job = None;
                self.next = self.job_target;
            }
        } else {
            if let Some((value, to)) = coordinates.reuse(self.next, self.target.id, self.value) {
                self.value = value;
                self.next = to;
                return Progress::Pending;
            }
            let (images, to) = coordinates.changes.images(self.next, self.target.id);
            self.job_target = to;
            self.job = Some(arena.substitute(self.value, images));
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

    #[test]
    fn sparse_publications_pack_exact_deltas_in_one_segment() {
        let mut a = Arena::default();
        let mut c = Coordinates::default();
        let old = c.current();
        for _ in 0..8 {
            let (id, _) = a.fresh_choice();
            c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        }
        assert_eq!(c.changes.segments.len(), 1);
        assert_eq!(c.changes.len(), 8, "all exact source maps remain retained");
        assert_eq!(
            c.memory(),
            26,
            "maps, assignments, epoch leases, and one run"
        );
        drop(old);
        for _ in 0..100 {
            c.cleanup_tick();
        }
        assert_eq!(c.memory(), 1);
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn adjacent_constant_deltas_traverse_an_unaffected_boolean_once() {
        let mut a = Arena::default();
        let (_, x) = a.fresh_choice();
        let (_, y) = a.fresh_choice();
        let mut conjunction = a.start(Operation::And(x, y));
        let input = loop {
            if let Progress::Complete(v) = conjunction.tick(&mut a) {
                break v;
            }
        };
        let mut c = Coordinates::default();
        let old = c.current();
        let mut images = BTreeMap::new();
        for _ in 0..8 {
            let (id, _) = a.fresh_choice();
            images.insert(id, Condition::TRUE);
            c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        }
        let mut reference = a.substitute(input, Arc::new(images));
        while reference.tick(&mut a) != Progress::Complete(input) {}
        for _ in 0..2 {
            let mut transport = c.transport(input, &old);
            assert_eq!(finish(&mut transport, &mut a, &c), input);
            assert!(
                transport.measured_transform.work <= reference.work(),
                "at most one Boolean traversal per exact composition"
            );
        }
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn repeated_exact_single_epoch_transport_reuses_boolean_work() {
        let mut a = Arena::default();
        let (xi, x) = a.fresh_choice();
        let (_, y) = a.fresh_choice();
        let mut job = a.start(Operation::And(x, y));
        let input = loop {
            if let Progress::Complete(value) = job.tick(&mut a) {
                break value;
            }
        };
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(xi, Condition::TRUE)])));
        let mut first = c.transport(input, &old);
        assert_eq!(finish(&mut first, &mut a, &c), y);
        assert!(first.measured_transform.work > 0);
        let mut second = c.transport(input, &old);
        assert_eq!(finish(&mut second, &mut a, &c), y);
        assert_eq!(
            second.measured_transform.work, 0,
            "an exact retained prefix result is reusable"
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn retained_composition_images_are_bounded_across_many_segments() {
        let mut a = Arena::default();
        let (_, x) = a.fresh_choice();
        let mut c = Coordinates::default();
        let old = c.current();
        for _ in 0..64 {
            let (id, _) = a.fresh_choice();
            c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
        }
        assert_eq!(finish(&mut c.transport(x, &old), &mut a, &c), x);
        let d = c.changes.diagnostics.borrow();
        assert_eq!(d.compositions_built, 8);
        assert!(
            d.retained_compositions <= 1,
            "one recent composition, not one per retained segment"
        );
        assert!(d.retained_composition_assignments <= 8);
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn segmented_transport_work_scales_at_equal_publications_and_reader_cutoffs() {
        for n in [8, 64, 512] {
            let code = crate::program::prepare(
                &crate::syntax::parse_program("").unwrap(),
                &crate::syntax::parse_query("true").unwrap(),
            )
            .unwrap();
            let mut e = Engine::new(Arc::new(code));
            let (_, x) = e.arena.fresh_choice();
            let (_, y) = e.arena.fresh_choice();
            let mut job = e.arena.start(Operation::And(x, y));
            let input = loop {
                if let Progress::Complete(value) = job.tick(&mut e.arena) {
                    break value;
                }
            };
            let old = e.coordinates.current();
            let mut maps = vec![];
            for _ in 0..n {
                let (id, _) = e.arena.fresh_choice();
                let map = Arc::new(BTreeMap::from([(id, Condition::TRUE)]));
                maps.push(map.clone());
                e.coordinates.publish(map);
            }
            let prepared_records = e.coordinates.memory();
            let mut reference_work = 0;
            let mut reference_calls = 0;
            for value in [input, input.not(), input] {
                // Execute the accepted algorithm's exact per-epoch operations,
                // with the same immutable map owners and reader endpoints.
                for map in &maps {
                    let mut transform = e.arena.substitute(value, map.clone());
                    loop {
                        reference_calls += 1;
                        if let Progress::Complete(result) = transform.tick(&mut e.arena) {
                            assert_eq!(result, value);
                            reference_work += transform.work();
                            break;
                        }
                    }
                }
                let mut transport = e.coordinates.transport(value, &old);
                loop {
                    if let Progress::Complete(result) = e.transport_tick(&mut transport, true) {
                        assert_eq!(result, value);
                        break;
                    }
                }
            }
            let retained_records = e.coordinates.memory();
            let work = e.diagnostics.shared.coordinates.clone();
            assert_eq!(work.search.epochs_crossed, 3 * n as u64);
            assert_eq!(work.search.transform_starts, 3 * n as u64 / 8);
            assert_eq!(work.search.transform.work * 8, reference_work);
            assert_eq!(work.segments.retained_compositions, 1);
            assert_eq!(work.segments.retained_composition_assignments, 8);
            if n == 8 {
                assert_eq!(work.segments.segment_hits, 2);
            }
            drop(maps);
            drop(old);
            let mut release_calls = 0;
            while !e.cleanup_coordinates() {
                release_calls += 1;
            }
            assert_eq!(e.coordinates.memory(), 1);
            assert_eq!(
                e.diagnostics.shared.coordinates.assignments_drained,
                n as u64
            );
            println!(
                "coordinate_probe={}",
                serde_json::json!({
                    "publications": n, "assignments": n, "readers": 3,
                    "validated_results": 3, "source_cutoff": 0, "target_cutoff": n,
                    "reference_transform_starts": 3*n, "reference_transform_calls": reference_calls,
                    "reference_transform_work": reference_work, "candidate": work,
                    "candidate_prepared_records": prepared_records,
                    "candidate_retained_records": retained_records,
                    "accepted_log_records_formula": 3*n+1,
                    "release_calls": release_calls, "final_records": e.coordinates.memory(),
                })
            );
        }
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
    fn prefix_alone_roots_boolean_dag_until_its_source_map_retires() {
        let mut arena = Arena::default();
        let (_, x) = arena.fresh_choice();
        let (_, y) = arena.fresh_choice();
        let (zi, _) = arena.fresh_choice();
        let input = boolean(&mut arena, Operation::And(x, y));
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(zi, Condition::FALSE)])));
        let mut transport = c.transport(input, &old);
        while transport.tick(&mut arena, &c) != Progress::Complete(input) {}
        drop(transport);
        collect(&mut arena, &c, &[]);
        assert!(
            arena.contains(input),
            "completed prefix owns both Boolean roots"
        );
        let mut repeat = c.transport(input, &old);
        assert_eq!(repeat.tick(&mut arena, &c), Progress::Pending);
        assert_eq!(repeat.tick(&mut arena, &c), Progress::Complete(input));
        drop(repeat);
        drop(old);
        for _ in 0..100 {
            c.cleanup_tick();
        }
        assert_eq!(c.memory(), 1);
        collect(&mut arena, &c, &[]);
        assert!(!arena.contains(input));
    }

    #[test]
    fn later_prefix_cannot_satisfy_an_older_frozen_target() {
        let mut arena = Arena::default();
        let (xi, _) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let mut c = Coordinates::default();
        let old = c.current();
        c.publish(Arc::new(BTreeMap::from([(xi, Condition::TRUE)])));
        let mut frozen = c.transport(y, &old);
        c.publish(Arc::new(BTreeMap::from([(yi, Condition::FALSE)])));
        let mut later = c.transport(y, &old);
        while later.tick(&mut arena, &c) != Progress::Complete(Condition::FALSE) {}
        drop(later);
        loop {
            collect(&mut arena, &c, &[&frozen]);
            if let Progress::Complete(value) = frozen.tick(&mut arena, &c) {
                assert_eq!(value, y);
                break;
            }
        }
    }
    #[test]
    fn every_segment_cutoff_and_repeated_key_matches_a_truth_table() {
        let mut arena = Arena::default();
        let (xi, x) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let (zi, z) = arena.fresh_choice();
        let xy = boolean(&mut arena, Operation::And(x, y));
        let input = boolean(&mut arena, Operation::Or(xy, z));
        let mut c = Coordinates::default();
        let mut epochs = vec![c.current()];
        let ids = [xi, yi, zi];
        for i in 0..19 {
            c.publish(Arc::new(BTreeMap::from([(
                ids[i % 3],
                if i % 2 == 0 {
                    Condition::TRUE
                } else {
                    Condition::FALSE
                },
            )])));
            epochs.push(c.current());
        }
        for to in 0..=19 {
            for from in 0..=to {
                for _ in 0..2 {
                    let mut transport = c.transport(input, &epochs[from]);
                    // The same frozen target a transport created at `to` owns.
                    transport.target = epochs[to].clone();
                    let result = loop {
                        if let Progress::Complete(value) = transport.tick(&mut arena, &c) {
                            break value;
                        }
                    };
                    for bits in 0..8 {
                        let mut values = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
                        let mut assigned = [false; 3];
                        for i in from..to {
                            if !assigned[i % 3] {
                                values[i % 3] = i % 2 == 0;
                                assigned[i % 3] = true;
                            }
                        }
                        assert_eq!(
                            arena.evaluate(result, |id| bits & (1 << id) != 0),
                            (values[0] && values[1]) || values[2],
                            "{from}..{to}, bits={bits}"
                        );
                    }
                }
            }
        }
        drop(epochs);
        for _ in 0..200 {
            c.cleanup_tick();
        }
        assert_eq!(c.memory(), 1);
    }

    #[test]
    fn cached_prefix_survives_append_and_interior_retirement() {
        let mut arena = Arena::default();
        let (xi, x) = arena.fresh_choice();
        let (yi, y) = arena.fresh_choice();
        let mut c = Coordinates::default();
        let origin = c.current();
        c.publish(Arc::new(BTreeMap::from([(xi, Condition::TRUE)])));
        let interior = c.current();
        c.publish(Arc::new(BTreeMap::from([(yi, Condition::FALSE)])));
        let mut frozen = c.transport(y, &origin);
        assert_eq!(frozen.tick(&mut arena, &c), Progress::Pending);
        c.publish(Arc::new(BTreeMap::from([(yi, Condition::TRUE)])));
        let mut after = c.transport(x, &interior);
        for _ in 0..100 {
            c.cleanup_tick();
        }
        loop {
            collect(&mut arena, &c, &[&frozen, &after]);
            if let Progress::Complete(value) = frozen.tick(&mut arena, &c) {
                assert_eq!(value, Condition::FALSE);
                break;
            }
        }
        drop(frozen);
        drop(origin);
        for _ in 0..100 {
            c.cleanup_tick();
        }
        assert!(c.changes.get(&0).is_none());
        assert!(c.changes.get(&1).is_some());
        loop {
            collect(&mut arena, &c, &[&after]);
            if let Progress::Complete(value) = after.tick(&mut arena, &c) {
                assert_eq!(value, x, "interior reader must not apply earlier x=true");
                break;
            }
        }
        drop(after);
        drop(interior);
        for _ in 0..100 {
            c.cleanup_tick();
        }
        assert_eq!(c.memory(), 1);
    }

    #[test]
    fn cancelling_composed_transport_at_each_suspension_releases_every_map() {
        for suspension in 0..48 {
            let mut arena = Arena::default();
            let (_, x) = arena.fresh_choice();
            let (_, y) = arena.fresh_choice();
            let input = boolean(&mut arena, Operation::And(x, y));
            let mut c = Coordinates::default();
            let origin = c.current();
            for _ in 0..19 {
                let (id, _) = arena.fresh_choice();
                c.publish(Arc::new(BTreeMap::from([(id, Condition::TRUE)])));
            }
            let mut transport = c.transport(input, &origin);
            drop(origin);
            for _ in 0..suspension {
                collect(&mut arena, &c, &[&transport]);
                if matches!(transport.tick(&mut arena, &c), Progress::Complete(_)) {
                    break;
                }
            }
            loop {
                collect(&mut arena, &c, &[&transport]);
                c.cleanup_tick();
                if transport.discard_tick() {
                    break;
                }
            }
            drop(transport);
            for _ in 0..200 {
                c.cleanup_tick();
            }
            assert_eq!(c.memory(), 1);
            collect(&mut arena, &c, &[]);
            assert!(!arena.contains(input));
        }
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
        assert!(coordinates.changes.get(&0).is_none());
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
    #[cfg(feature = "diagnostics")]
    pub(super) fn measure_coordinate_segments(&mut self) {
        let changes = &self.coordinates.changes;
        let mut d = changes.diagnostics.borrow().clone();
        d.retained_segments = changes.segments.len();
        d.retained_maps = changes.len();
        d.retained_prefixes = usize::from(self.coordinates.prefix.get().is_some());
        self.diagnostics.shared.coordinates.segments = d;
    }
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
            self.measure_coordinate_segments();
        }
        done
    }
    pub(super) fn transport_tick(&mut self, transport: &mut Transport, search: bool) -> Progress {
        #[cfg(not(feature = "diagnostics"))]
        let _ = search;
        #[cfg(feature = "diagnostics")]
        let before = transport.next;
        #[cfg(feature = "diagnostics")]
        let starting = transport.job.is_none() && transport.next != transport.target.id;
        let result = transport.tick(&mut self.arena, &self.coordinates);
        #[cfg(feature = "diagnostics")]
        {
            let d = if search {
                &mut self.diagnostics.shared.coordinates.search
            } else {
                &mut self.diagnostics.shared.coordinates.completion
            };
            d.calls += 1;
            d.transform_starts += u64::from(starting && transport.job.is_some());
            d.epochs_crossed += transport.next - before;
            let delta = std::mem::take(&mut transport.measured_transform);
            d.transform.calls += delta.calls;
            d.transform.work += delta.work;
            self.measure_coordinate_segments();
        }
        result
    }
}
