//! Bounded interactions. SIZE is sibling/retention/inspection count; rows is
//! residual width, work is continued applications, cadence is apps per rotation.
//! Defaults: rows=1, work=32, cadence=1. Work/cadence are application lower
//! bounds: reaching a capturable graph can advance the source further; phase
//! records retain actual application counts. Every retained view has exactly
//! keep(Vi),loop(Vi) for each query variable. Pending syntax is outside this
//! committed-multiset oracle, as in lifecycle's existing snapshot inspection.
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
fn capture(e: &mut Engine, b: &mut Budget, width: usize) -> Result<Option<ViewId>, String> {
    let relation = e
        .program()
        .signatures()
        .iter()
        .position(|s| s.name == "loop")
        .unwrap();
    while b.step(e) {
        check(
            e.take_output().is_none(),
            "continuing view source emitted answer",
        )?;
        if e.facts(relation).map_err(|e| e.to_string())?.count() != width {
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
        if !b.step(e) {
            return Ok(false);
        }
    }
    check(r.answers == 1 && !r.open, "inspection answer multiplicity")?;
    Ok(b.in_time())
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
        n.checked_mul(rows).is_some(),
        "interaction dimensions overflow",
    )?;
    let held = case == "life-held-output";
    let query = if held {
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
    let code = Arc::new(
        prepare(
            &parse_program("loop(X) <=> loop(X).").map_err(|e| e.to_string())?,
            &parse_query(&query).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
    );
    let mut e = Engine::new(code);
    let mut b = Budget::new(limit, timeout);
    let mut views = VecDeque::new();
    let mut readers = Vec::new();
    let mut output = Reader::default();
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
                let Some(id) = capture(&mut e, &mut b, rows)? else {
                    return Ok(false);
                };
                views.push_back(id);
            }
            if case == "life-inspections" {
                for _ in 0..n {
                    let id = loop {
                        if !b.available() {
                            return Ok(false);
                        }
                        match e.start_inspection(Some(views[0]), vec![]) {
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
            }
        }
        phase(
            "interaction_admitted",
            &e,
            &b,
            json!({"views":views.len(),"inspections":readers.len()}),
        );
        let start = e.applications();
        let mut next = cadence;
        let mut rotations = 0;
        while e.applications() - start < work {
            if !b.step(&mut e) {
                return Ok(false);
            }
            if !held {
                check(e.take_output().is_none(), "unexpected source answer")?;
            }
            if case == "life-archive-rotate" && e.applications() - start >= next {
                let id = *views.front().unwrap();
                if !release_view(&mut e, id, &mut b, false)? {
                    return Ok(false);
                }
                views.pop_front();
                let Some(id) = capture(&mut e, &mut b, rows)? else {
                    return Ok(false);
                };
                views.push_back(id);
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
        phase(
            "interaction_held",
            &e,
            &b,
            json!({"continued_applications":e.applications()-start,"rotations":rotations}),
        );
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
                        match e.start_inspection(Some(snapshot), vec![]) {
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
            json!({"answers":if held {output.answers} else {readers.iter().map(|(_,r)|r.answers).sum()}}),
        );
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
    #[test]
    fn cancellation_releases_partly_unread_inspection_owners() {
        let code = Arc::new(
            prepare(
                &parse_program("loop(X) <=> loop(X).").unwrap(),
                &parse_query("keep(A),loop(A)").unwrap(),
            )
            .unwrap(),
        );
        let mut e = Engine::new(code);
        let mut b = Budget::new(100_000, Duration::from_secs(5));
        let snapshot = capture(&mut e, &mut b, 1).unwrap().unwrap();
        let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
        let mut r = Reader::default();
        while !r.open {
            assert!(b.step(&mut e));
            if let Some(o) = e.take_inspection_output(id).unwrap() {
                read(&mut r, o, &e, &["keep", "loop"], 1, &mut b).unwrap();
            }
        }
        assert_eq!(r.answers, 0);
        assert_eq!(e.memory().inspections, 1);
        assert!(cancel(&mut e, 100_000, Duration::from_secs(5)));
        assert!(release(&mut e, 100_000, Duration::from_secs(5)).unwrap());
        assert!(zero(e.memory()));
        assert_eq!(e.pending_tasks(), 0);
    }
}
