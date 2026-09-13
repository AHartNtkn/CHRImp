//! Bounded interactions. SIZE is sibling/retention/inspection count; rows is
//! residual width, work is continued applications, cadence is apps per rotation.
//! Defaults: rows=1, work=32, cadence=1. Work/cadence are application lower
//! bounds: reaching a capturable graph can advance the source further; phase
//! records retain actual application counts. Every retained view has exactly
//! keep(Vi),loop(Vi) for each query variable. Pending syntax is outside this
//! committed-multiset oracle, as in lifecycle's existing snapshot inspection.
//!
//! Conditional archives/inspectors use turn() <=> turn(),(fail;true), beside
//! an immutable width-sized keep/loop payload. Held output instead uses
//! loop(X) <=> (fail;loop(X)) for its independent continuing siblings. Captures
//! occur at the explicit choice birth after turn() is posted. Inspections select
//! the right arm of EVERY retained choice in the snapshot prefix: older failing
//! arms can still be pending at capture. This specifies the one known surviving
//! history, rather than enumerating exponentially many unfinished failure paths.
//! Its committed projection is one turn() plus the full keep/loop payload.
//! Raw conditional row counts are not projected multiplicities. Choice
//! high-water IDs demonstrate births, not concrete program or answer counts.
//! Archives pin the actual live choice prefix at their snapshot cutoff. Rotating
//! conditional archives capture before release (transient SIZE+1 views); partly
//! unread inspectors release their backing snapshot before continued source work.
//! Explicit collection checks pins while held, then their disappearance while
//! the source still runs after release. Held output instead validates its frozen
//! answer after collection; it does not claim ownership of later choice births.
//! After the declared held-work prefix, conditional views drain through the
//! projection-only API with source applications fixed. This keeps inspection
//! completion from extending the continuing-source duration without bound.
//! Post-release collection/progress uses the remaining interaction budget and is
//! included in interaction_exit. Phases report actual applications and memory;
//! no tick ratio claims CPU or physical GC cost. Final cleanup checks zero owners
//! and execution objects using lifecycle's existing reclamation oracle.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub rows: usize,
    pub work: u64,
    pub cadence: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            rows: 1,
            work: 32,
            cadence: 1,
        }
    }
}

fn pending(o: &Output) -> bool {
    matches!(
        o,
        Output::PendingBegin { .. }
            | Output::PendingEnd
            | Output::Expression { .. }
            | Output::ExpressionRelation { .. }
            | Output::ExpressionVariable { .. }
            | Output::ExpressionEnd
    )
}
fn read(
    r: &mut Reader,
    o: Output,
    e: &Engine,
    names: &[&str],
    width: usize,
    b: &mut Budget,
) -> Result<(), String> {
    let t = observation::start(b.detailed);
    let conditional_view =
        names == ["keep", "loop"] && e.program().signatures().iter().any(|s| s.name == "turn");
    if conditional_view && matches!(o, Output::End) {
        check(
            r.rows
                .iter()
                .filter(|(name, ports)| name == "turn" && ports.is_empty())
                .count()
                == 1,
            "conditional view must contain exactly one nullary continuation",
        )?;
        check(
            !r.rows
                .iter()
                .any(|(name, ports)| name == "turn" && !ports.is_empty()),
            "conditional continuation arity",
        )?;
        r.rows.retain(|(name, _)| name != "turn");
    }
    let result = if pending(&o) && names == ["keep", "loop"] {
        Ok(false)
    } else {
        r.push(o, e.program().signatures(), names, width)
    };
    b.validator += observation::elapsed(t);
    result.map(|_| ())
}
fn phase(name: &str, e: &Engine, b: &Budget, data: Value) {
    crate::report::emit(
        "phase",
        json!({"phase":name,"applications":e.applications(),
        "ticks":b.ticks,"elapsed_ms":ms(b.start.elapsed()),"memory":crate::report::memory(crate::memory(e)),
        "validator_ms":b.detailed.then(|| ms(b.validator)),
        "max_step_ms":b.detailed.then(|| ms(b.maximum)),"data":data}),
    );
}
fn capture(
    e: &mut Engine,
    b: &mut Budget,
    width: usize,
    conditional: bool,
) -> Result<Option<ViewId>, String> {
    let relation = e
        .program()
        .signatures()
        .iter()
        .position(|s| s.name == if conditional { "turn" } else { "loop" })
        .unwrap();
    let mut last_choice = e.choices().next_back().map(|(&id, _)| id);
    while b.step(e) {
        check(
            e.take_output().is_none(),
            "continuing view source emitted answer",
        )?;
        if conditional {
            // turn() is posted before this body's choice instruction. Capture
            // the birth before its fail arm can trigger unpinned compaction.
            let choice = e.choices().next_back().map(|(&id, _)| id);
            let born = choice > last_choice;
            last_choice = last_choice.max(choice);
            if !born {
                continue;
            }
        } else if e.facts(relation).map_err(|e| e.to_string())?.count() != width {
            continue;
        }
        match e.capture_snapshot() {
            Ok(id) => return Ok(Some(id)),
            Err(InspectionError::Busy | InspectionError::Initializing) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(None)
}
fn release_view(
    e: &mut Engine,
    id: ViewId,
    b: &mut Budget,
    inspection: bool,
) -> Result<bool, String> {
    while b.available() {
        let result = if inspection {
            e.release_inspection(id)
        } else {
            e.release_snapshot(id)
        };
        match result {
            Ok(()) => {
                b.ticks += 1;
                return Ok(b.in_time());
            }
            Err(InspectionError::Busy) => {
                if !b.step(e) {
                    break;
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}
fn drain_view(
    e: &mut Engine,
    id: ViewId,
    r: &mut Reader,
    width: usize,
    b: &mut Budget,
) -> Result<bool, String> {
    let projection_only = e.program().signatures().iter().any(|s| s.name == "turn");
    let applications = e.applications();
    loop {
        if !b.available() {
            return Ok(false);
        }
        if let Some(o) = e.take_inspection_output(id).map_err(|e| e.to_string())? {
            read(r, o, e, &["keep", "loop"], width, b)?;
        }
        if e.inspection_status(id).map_err(|e| e.to_string())?.done {
            break;
        }
        if projection_only {
            let t = observation::start(b.detailed);
            during(Phase::Inspection, || e.advance_inspection(id, 1)).map_err(|e| e.to_string())?;
            b.maximum = b.maximum.max(observation::elapsed(t));
            b.ticks += 1;
            check(
                e.applications() == applications,
                "projection advanced conditional source",
            )?;
        } else if !b.step(e) {
            return Ok(false);
        }
    }
    check(r.answers == 1 && !r.open, "inspection answer multiplicity")?;
    Ok(b.in_time())
}

// One explicit birth and one failing arm per committed rewrite. The independent
// payload width does not change the rate/number of continuing source streams.
const CONDITIONAL_RULE: &str = "turn() <=> turn(),(fail;true).";
const CONDITIONAL_HELD_RULE: &str = "loop(X) <=> (fail;loop(X)).";

fn inspection_selection(
    e: &Engine,
    snapshot: ViewId,
    conditional: bool,
) -> Result<Vec<(u64, bool)>, String> {
    if !conditional {
        return Ok(Vec::new());
    }
    let last = e
        .snapshot_info(snapshot)
        .map_err(|e| e.to_string())?
        .last_choice
        .ok_or("conditional snapshot has no choice")?;
    // All choices here come from (fail;true), with no query disjunction. A
    // capture can retain several unfinished fail arms. Selecting only the newest
    // coordinate leaves those older arms unconstrained and asks the projector to
    // enumerate pending histories, not the one independently known survivor.
    Ok(e.choices()
        .filter(|(id, _)| **id <= last)
        .map(|(&id, _)| (id, false))
        .collect())
}

fn pinned_choices(e: &Engine, views: &VecDeque<ViewId>) -> Result<Vec<u64>, String> {
    let mut cutoff = None;
    for &id in views {
        cutoff = cutoff.max(e.snapshot_info(id).map_err(|e| e.to_string())?.last_choice);
    }
    Ok(e.choices()
        .filter(|(id, _)| cutoff.is_some_and(|last| **id <= last))
        .map(|(&id, _)| id)
        .collect())
}

// Both lists are sorted public choice IDs. Merge once, rather than scanning
// the live prefix afresh for every pin in a sustained-retention workload.
fn live_pins(e: &Engine, pins: &[u64]) -> usize {
    let mut live = e.choices().map(|(&id, _)| id).peekable();
    pins.iter()
        .filter(|&&pin| {
            while live.peek().is_some_and(|&id| id < pin) {
                live.next();
            }
            live.peek() == Some(&pin)
        })
        .count()
}

fn check_pins(e: &Engine, pins: &[u64]) -> Result<(), String> {
    check(
        live_pins(e, pins) == pins.len(),
        "held view lost a pinned choice",
    )
}

fn collect(e: &mut Engine, b: &mut Budget, held_output: bool) -> Result<bool, String> {
    e.request_collection();
    while e.collecting() {
        if !b.step(e) {
            return Ok(false);
        }
        if !held_output {
            check(
                e.take_output().is_none(),
                "continuing conditional source emitted an answer",
            )?;
        }
    }
    Ok(b.in_time())
}

fn reclaim_choices(
    e: &mut Engine,
    pins: &[u64],
    width: usize,
    b: &mut Budget,
) -> Result<bool, String> {
    loop {
        if !collect(e, b, false)? {
            return Ok(false);
        }
        if live_pins(e, pins) == 0 {
            check(
                !e.exhausted(),
                "source exhausted during post-release reclamation",
            )?;
            return Ok(true);
        }
        // Let unfinished source bodies settle between collection attempts. The
        // shared budget censors stalled progress; this is not a fixed tick ceiling.
        let start = e.applications();
        while e.applications() - start < width as u64 {
            if !b.step(e) {
                return Ok(false);
            }
            check(e.take_output().is_none(), "unexpected post-release answer")?;
        }
    }
}

pub(super) fn run(
    case: &str,
    n: usize,
    options: Options,
    limit: u64,
    timeout: Duration,
) -> Result<bool, String> {
    let Options {
        rows,
        work,
        cadence,
    } = options;
    check(
        n > 0 && rows > 0 && work > 0 && cadence > 0,
        "positive interaction dimensions required",
    )?;
    check(
        n.checked_mul(rows).is_some() && n.checked_add(1).is_some(),
        "interaction dimensions overflow",
    )?;
    let conditional = case.ends_with("-conditional");
    let case = case.strip_suffix("-conditional").unwrap_or(case);
    check(
        matches!(
            case,
            "life-held-output" | "life-archive-fixed" | "life-archive-rotate" | "life-inspections"
        ),
        "unknown interaction case",
    )?;
    let held = case == "life-held-output";
    let mut query = if held {
        format!(
            "({});{}",
            vec!["loop(V0)"; n].join(";"),
            (0..rows)
                .map(|i| format!("done(V{i})"))
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        (0..rows)
            .map(|i| format!("keep(V{i}),loop(V{i})"))
            .collect::<Vec<_>>()
            .join(",")
    };
    if conditional && !held {
        query.push_str(",turn()");
    }
    let code = Arc::new(
        prepare(
            &parse_program(if conditional && held {
                CONDITIONAL_HELD_RULE
            } else if conditional {
                CONDITIONAL_RULE
            } else {
                "loop(X) <=> loop(X)."
            })
            .map_err(|e| e.to_string())?,
            &parse_query(&query).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
    );
    let mut e = Engine::new(code);
    let mut b = Budget::new(limit, timeout);
    let mut views = VecDeque::new();
    let mut readers = Vec::new();
    let mut output = Reader::default();
    let mut inspection_pins = Vec::new();
    let result = (|| -> Result<bool, String> {
        if held {
            while !output.open && b.step(&mut e) {
                if let Some(o) = e.take_output() {
                    check(
                        matches!(o, Output::Begin { .. }),
                        "first output was not Begin",
                    )?;
                    read(&mut output, o, &e, &["done"], rows, &mut b)?;
                }
            }
            if !output.open {
                return Ok(false);
            }
        } else {
            for _ in 0..if case == "life-inspections" { 1 } else { n } {
                let Some(id) = capture(&mut e, &mut b, rows, conditional)? else {
                    return Ok(false);
                };
                views.push_back(id);
            }
            if case == "life-inspections" {
                if conditional {
                    inspection_pins = pinned_choices(&e, &views)?;
                }
                for _ in 0..n {
                    let id = loop {
                        if !b.available() {
                            return Ok(false);
                        }
                        match e.start_inspection(
                            Some(views[0]),
                            inspection_selection(&e, views[0], conditional)?,
                        ) {
                            Ok(id) => {
                                b.ticks += 1;
                                break id;
                            }
                            Err(InspectionError::Busy | InspectionError::Initializing) => {
                                if !b.step(&mut e) {
                                    return Ok(false);
                                }
                            }
                            Err(e) => return Err(e.to_string()),
                        }
                    };
                    let mut r = Reader::default();
                    while !r.open && b.step(&mut e) {
                        if let Some(o) = e.take_inspection_output(id).map_err(|e| e.to_string())? {
                            read(&mut r, o, &e, &["keep", "loop"], rows, &mut b)?;
                        }
                    }
                    if !r.open {
                        return Ok(false);
                    }
                    readers.push((id, r));
                }
                if conditional {
                    // The partly unread inspectors must own the old roots themselves.
                    let snapshot = views[0];
                    if !release_view(&mut e, snapshot, &mut b, false)? {
                        return Ok(false);
                    }
                    views.pop_front();
                }
            }
        }
        phase(
            "interaction_admitted",
            &e,
            &b,
            json!({"views":views.len(),"inspections":readers.len(),"conditional":conditional}),
        );
        let start = e.applications();
        let first_choice = e.choices().next_back().map(|(&id, _)| id);
        let mut latest_choice = first_choice;
        let mut next = cadence;
        let mut rotations = 0;
        while e.applications() - start < work || (conditional && latest_choice <= first_choice) {
            if !b.step(&mut e) {
                return Ok(false);
            }
            latest_choice = latest_choice.max(e.choices().next_back().map(|(&id, _)| id));
            if !held {
                check(e.take_output().is_none(), "unexpected source answer")?;
            }
            if case == "life-archive-rotate" && e.applications() - start >= next {
                // Conditional rotation overlaps capture/release, as in the XOR
                // pinning control. No unowned gap may compact the retained prefix.
                if conditional {
                    let Some(id) = capture(&mut e, &mut b, rows, true)? else {
                        return Ok(false);
                    };
                    views.push_back(id);
                }
                let id = *views.front().unwrap();
                if !release_view(&mut e, id, &mut b, false)? {
                    return Ok(false);
                }
                views.pop_front();
                if !conditional {
                    let Some(id) = capture(&mut e, &mut b, rows, false)? else {
                        return Ok(false);
                    };
                    views.push_back(id);
                }
                latest_choice = latest_choice.max(e.choices().next_back().map(|(&id, _)| id));
                rotations += 1;
                next = (e.applications() - start)
                    .checked_add(cadence)
                    .ok_or("cadence overflow")?;
            }
        }
        check(!e.exhausted(), "continuing source exhausted")?;
        if !held {
            check(
                e.snapshots().count() == views.len(),
                "archive ownership mismatch",
            )?;
        }
        let pins = if conditional && !held {
            if case == "life-inspections" {
                inspection_pins.clone()
            } else {
                pinned_choices(&e, &views)?
            }
        } else {
            Vec::new()
        };
        if conditional {
            check(
                latest_choice > first_choice,
                "conditional source created no new choices",
            )?;
            if !held {
                check(
                    !pins.is_empty(),
                    "conditional owners retained no choice evidence",
                )?;
            }
            if !collect(&mut e, &mut b, held)? {
                return Ok(false);
            }
            check_pins(&e, &pins)?;
            if case == "life-inspections" {
                for (id, r) in &readers {
                    check(
                        r.open
                            && r.answers == 0
                            && !e.inspection_status(*id).map_err(|e| e.to_string())?.done,
                        "held inspector lost its partly unread projection",
                    )?;
                }
                check(
                    e.snapshots().count() == 0,
                    "inspector pin evidence still has a snapshot owner",
                )?;
            }
        }
        phase(
            "interaction_held",
            &e,
            &b,
            json!({"continued_applications":e.applications()-start,"rotations":rotations,
                "conditional":conditional,"first_choice":first_choice,"latest_choice":latest_choice,
                "pinned_choices_checked":pins.len(),"output_partly_unread":output.open,
                "rotation_peak_views":if conditional && rotations > 0 {n+1} else {views.len()}}),
        );
        let projection_start = e.applications();
        if held {
            while output.answers == 0 && b.step(&mut e) {
                if let Some(o) = e.take_output() {
                    read(&mut output, o, &e, &["done"], rows, &mut b)?;
                }
            }
            if output.answers == 0 {
                return Ok(false);
            }
            check(
                output.answers == 1 && !output.open,
                "held answer incomplete",
            )?;
        } else {
            if case != "life-inspections" {
                for &snapshot in &views {
                    let id = loop {
                        if !b.available() {
                            return Ok(false);
                        }
                        match e.start_inspection(
                            Some(snapshot),
                            inspection_selection(&e, snapshot, conditional)?,
                        ) {
                            Ok(id) => {
                                b.ticks += 1;
                                break id;
                            }
                            Err(InspectionError::Busy) => {
                                if !b.step(&mut e) {
                                    return Ok(false);
                                }
                            }
                            Err(e) => return Err(e.to_string()),
                        }
                    };
                    readers.push((id, Reader::default()));
                }
            }
            for (id, r) in &mut readers {
                if !drain_view(&mut e, *id, r, rows, &mut b)? {
                    return Ok(false);
                }
                if !release_view(&mut e, *id, &mut b, true)? {
                    return Ok(false);
                }
            }
            check(e.inspections().count() == 0, "inspection handles retained")?;
        }
        phase(
            "interaction_resumed",
            &e,
            &b,
            json!({"answers":if held {output.answers} else {readers.iter().map(|(_,r)|r.answers).sum()},
                "projection_only":conditional && !held,
                "source_applications_during_projection":e.applications()-projection_start}),
        );
        if conditional {
            while let Some(&id) = views.front() {
                if !release_view(&mut e, id, &mut b, false)? {
                    return Ok(false);
                }
                views.pop_front();
            }
            // Reclamation is checked while the source is still continuing, not
            // only after cancellation. Final cancel/release also checks all memory.
            if !reclaim_choices(&mut e, &pins, rows, &mut b)? {
                return Ok(false);
            }
            phase(
                "interaction_unpinned",
                &e,
                &b,
                json!({"released_choice_pins":pins.len(),"snapshots":e.snapshots().count(),
                         "inspections":e.inspections().count(),"source_continuing":!e.exhausted()}),
            );
        }
        Ok(b.in_time())
    })();
    phase(
        "interaction_exit",
        &e,
        &b,
        json!({"complete":matches!(result, Ok(true)),
        "censored":matches!(result, Ok(false)),"error":result.as_ref().err()}),
    );
    // Cleanup also runs for censored or semantically invalid interaction phases.
    let canceled = cancel(&mut e, limit, timeout);
    let clean = if canceled {
        release(&mut e, limit, timeout)?
    } else {
        false
    };
    result.map(|complete| complete && clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_interaction_dimensions_and_censoring() {
        for case in [
            "life-held-output",
            "life-archive-fixed",
            "life-archive-rotate",
            "life-inspections",
            "life-held-output-conditional",
            "life-archive-fixed-conditional",
            "life-archive-rotate-conditional",
            "life-inspections-conditional",
        ] {
            for (n, rows, work, cadence) in [(1, 3, 7, 2), (3, 1, 11, 4)] {
                assert!(
                    run(
                        case,
                        n,
                        Options {
                            rows,
                            work,
                            cadence
                        },
                        500_000,
                        Duration::from_secs(10)
                    )
                    .unwrap(),
                    "{case}"
                );
            }
            assert!(
                !run(case, 2, Options::default(), 1, Duration::from_secs(1)).unwrap(),
                "{case}"
            );
        }
    }
    fn conditional_source() -> Engine {
        Engine::new(Arc::new(
            prepare(
                &parse_program(CONDITIONAL_RULE).unwrap(),
                &parse_query("keep(A),loop(A),turn()").unwrap(),
            )
            .unwrap(),
        ))
    }

    fn advance_to(e: &mut Engine, target: u64, b: &mut Budget) {
        while e.applications() < target {
            assert!(b.step(e), "bounded conditional progress");
            assert!(e.take_output().is_none());
        }
        assert!(!e.exhausted());
    }

    #[test]
    fn failing_arm_leaves_one_known_answer_and_one_application() {
        let code = prepare(
            &parse_program("once(X) <=> (fail;done(X)).").unwrap(),
            &parse_query("keep(A),once(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut b = Budget::new(100_000, Duration::from_secs(5));
        let mut reader = Reader::default();
        while !e.delivery_done() {
            assert!(b.step(&mut e));
            if let Some(o) = e.take_output() {
                read(&mut reader, o, &e, &["keep", "done"], 1, &mut b).unwrap();
            }
        }
        assert_eq!(reader.answers, 1);
        assert!(!reader.open);
        assert_eq!(e.applications(), 1);
        assert!(e.exhausted());
        assert!(cancel(&mut e, 100_000, Duration::from_secs(5)));
        assert!(release(&mut e, 100_000, Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn rotating_conditional_prefix_is_pinned_then_reclaimed_before_cancel() {
        let mut e = conditional_source();
        let mut b = Budget::new(2_000_000, Duration::from_secs(15));
        let mut views = VecDeque::new();
        // Same calculable overlap as xor_snapshot_pins_choices_until_released:
        // a fixed one-view population retains successively newer prefixes.
        for target in (8..=256).step_by(8) {
            advance_to(&mut e, target, &mut b);
            let id = capture(&mut e, &mut b, 1, true).unwrap().unwrap();
            views.push_back(id);
            if views.len() > 1 {
                assert!(release_view(&mut e, views[0], &mut b, false).unwrap());
                views.pop_front();
            }
            assert_eq!(e.memory().snapshots, 1);
        }
        let pins = pinned_choices(&e, &views).unwrap();
        assert!(
            pins.len() >= 16,
            "exercise retention beyond the unpinned control: {}",
            pins.len()
        );
        assert!(collect(&mut e, &mut b, false).unwrap());
        check_pins(&e, &pins).unwrap();
        assert!(check_pins(&e, &[u64::MAX]).is_err());
        let selection = inspection_selection(&e, views[0], true).unwrap();
        assert_eq!(selection.len(), pins.len());
        let inspection = e.start_inspection(Some(views[0]), selection).unwrap();
        let mut reader = Reader::default();
        assert!(drain_view(&mut e, inspection, &mut reader, 1, &mut b).unwrap());
        assert_eq!(reader.answers, 1);
        assert!(release_view(&mut e, inspection, &mut b, true).unwrap());
        assert!(release_view(&mut e, views[0], &mut b, false).unwrap());
        assert!(reclaim_choices(&mut e, &pins, 1, &mut b).unwrap());
        assert_eq!(e.memory().snapshots, 0);
        assert!(!e.choices().any(|(&id, _)| pins.contains(&id)));
        assert!(
            e.memory().choices < 16,
            "single-survivor compaction control: {:?}",
            e.memory()
        );
        let next = e.applications() + 8;
        advance_to(&mut e, next, &mut b);
        assert!(cancel(&mut e, 100_000, Duration::from_secs(5)));
        assert!(release(&mut e, 100_000, Duration::from_secs(5)).unwrap());
        assert!(zero(e.memory()));
    }

    #[test]
    fn unread_inspection_pins_without_snapshot_and_cancel_reclaims() {
        for work in [16, 64] {
            for count in [1, 3] {
                let mut e = conditional_source();
                let mut b = Budget::new(500_000, Duration::from_secs(10));
                let snapshot = capture(&mut e, &mut b, 1, true).unwrap().unwrap();
                let pins = pinned_choices(&e, &VecDeque::from([snapshot])).unwrap();
                assert!(!pins.is_empty());
                let mut readers = Vec::new();
                for _ in 0..count {
                    let id = loop {
                        match e.start_inspection(
                            Some(snapshot),
                            inspection_selection(&e, snapshot, true).unwrap(),
                        ) {
                            Ok(id) => break id,
                            Err(InspectionError::Busy) => assert!(b.step(&mut e)),
                            Err(error) => panic!("{error}"),
                        }
                    };
                    let mut reader = Reader::default();
                    while !reader.open {
                        assert!(b.step(&mut e));
                        if let Some(o) = e.take_inspection_output(id).unwrap() {
                            read(&mut reader, o, &e, &["keep", "loop"], 1, &mut b).unwrap();
                        }
                    }
                    readers.push((id, reader));
                }
                assert!(release_view(&mut e, snapshot, &mut b, false).unwrap());
                assert_eq!(e.memory().snapshots, 0);
                let target = e.applications() + work;
                advance_to(&mut e, target, &mut b);
                assert!(collect(&mut e, &mut b, false).unwrap());
                check_pins(&e, &pins).unwrap();
                assert_eq!(e.memory().inspections, count);
                for (id, reader) in &readers {
                    assert!(reader.open && reader.answers == 0);
                    assert!(!e.inspection_status(*id).unwrap().done);
                }
                assert!(cancel(&mut e, 100_000, Duration::from_secs(5)));
                assert!(release(&mut e, 100_000, Duration::from_secs(5)).unwrap());
                assert!(zero(e.memory()));
                assert_eq!(e.pending_tasks(), 0);
            }
        }
    }
    #[test]
    fn cancellation_releases_partly_unread_inspection_owners() {
        let code = prepare(
            &parse_program("loop(X) <=> loop(X).").unwrap(),
            &parse_query("keep(A),loop(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        let mut b = Budget::new(100_000, Duration::from_secs(5));
        let snapshot = capture(&mut e, &mut b, 1, false).unwrap().unwrap();
        let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
        let mut reader = Reader::default();
        while !reader.open {
            assert!(b.step(&mut e));
            if let Some(o) = e.take_inspection_output(id).unwrap() {
                read(&mut reader, o, &e, &["keep", "loop"], 1, &mut b).unwrap();
            }
        }
        assert_eq!(reader.answers, 0);
        assert_eq!(e.memory().inspections, 1);
        assert!(cancel(&mut e, 100_000, Duration::from_secs(5)));
        assert!(release(&mut e, 100_000, Duration::from_secs(5)).unwrap());
        assert!(zero(e.memory()));
        assert_eq!(e.pending_tasks(), 0);
    }
}
