//! First-view promotion and cancellation with a wide, wholly unposted body.
//! SIZE grows both the number of relations and their arity. The exact prefix
//! follows query-variable initialization, with zero posts/applications and
//! SIZE^2 pending ports, independent of scheduled-task representation.
use super::*;

fn checkpoint(phase: &str, e: &Engine, elapsed: Duration) {
    crate::report::emit(
        "phase",
        json!({
            "phase": phase, "elapsed_ms": ms(elapsed), "applications": e.applications(),
            "memory": crate::report::memory(crate::memory(e))
        }),
    );
    crate::report_diagnostics(phase, e);
}

fn projection(
    e: &mut Engine,
    snapshot: ViewId,
    n: usize,
    limit: u64,
    timeout: Duration,
) -> Result<bool, String> {
    let id = e
        .start_inspection(Some(snapshot), vec![])
        .map_err(|e| e.to_string())?;
    let mut b = Budget::new(limit, timeout);
    let (mut relations, mut bindings, mut answers, mut begins) = (0, 0, 0, 0);
    let mut ports = None;
    loop {
        if !b.available() {
            return Ok(false);
        }
        during(Phase::Inspection, || e.advance_inspection(id, 1)).map_err(|e| e.to_string())?;
        b.ticks += 1;
        if let Some(event) =
            during(Phase::Delivery, || e.take_inspection_output(id)).map_err(|e| e.to_string())?
        {
            during(Phase::Validator, || -> Result<(), String> {
                match event {
                    Output::Begin { .. } => begins += 1,
                    Output::Variable { slot, variable } => {
                        check(slot == 0 && variable == 0, "pending query binding")?;
                        bindings += 1;
                    }
                    Output::ExpressionRelation { relation } => {
                        check(relation == 0 && ports.is_none(), "pending relation/order")?;
                        relations += 1;
                        ports = Some(0);
                    }
                    Output::ExpressionVariable { variable } => {
                        check(variable == 0, "pending identity")?;
                        *ports.as_mut().ok_or("pending port outside relation")? += 1;
                    }
                    Output::ExpressionEnd => {
                        if let Some(count) = ports.take() {
                            check(count == n, "pending arity")?;
                        }
                    }
                    Output::End => answers += 1,
                    Output::PendingBegin { .. } | Output::PendingEnd => {}
                    Output::Expression {
                        operator: chr::observe::ExpressionKind::And,
                    } => {}
                    _ => return Err("unexpected committed fact or pending operator".into()),
                }
                Ok(())
            })?;
        }
        if e.inspection_status(id).map_err(|e| e.to_string())?.done {
            break;
        }
    }
    check(
        relations == n && ports.is_none() && bindings == 1 && begins == 1 && answers == 1,
        "pending syntax multiplicity or incomplete projection",
    )?;
    check(e.applications() == 0, "projection advanced source")?;
    e.release_inspection(id).map_err(|e| e.to_string())?;
    Ok(b.in_time())
}

pub(super) fn run(case: &str, n: usize, limit: u64, timeout: Duration) -> Result<bool, String> {
    check(n >= 2, "pending probe requires SIZE >= 2")?;
    let start = Instant::now();
    let ports = vec!["A"; n].join(",");
    let query = vec![format!("hold({ports})"); n].join(",");
    let code = during(Phase::Setup, || -> Result<_, String> {
        Ok(Arc::new(
            crate::allocation::prepare(
                &parse_program("").map_err(|e| e.to_string())?,
                &parse_query(&query).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        ))
    })?;
    let mut e = during(Phase::Setup, || Engine::new(code.clone()));
    checkpoint("pending_setup", &e, start.elapsed());
    let mut b = Budget::new(limit, timeout);
    while e.query_variables().is_empty() {
        if !b.step(&mut e) {
            return Ok(false);
        }
        check(e.take_output().is_none(), "premature pending probe output")?;
    }
    // This is an exact semantic prefix, not equivalence inferred from ticks.
    check(
        e.query_variables().len() == 1
            && e.applications() == 0
            && e.facts(0).map_err(|e| e.to_string())?.count() == 0,
        "unposted prefix",
    )?;
    checkpoint("pending_admitted", &e, b.start.elapsed());
    let snapshots = if case == "life-pending-snapshot" {
        let start = Instant::now();
        let first =
            during(Phase::Inspection, || e.capture_snapshot()).map_err(|e| e.to_string())?;
        checkpoint("pending_first_capture", &e, start.elapsed());
        let start = Instant::now();
        let second =
            during(Phase::Inspection, || e.capture_snapshot()).map_err(|e| e.to_string())?;
        checkpoint("pending_second_capture", &e, start.elapsed());
        vec![first, second]
    } else {
        vec![]
    };
    if !cancel(&mut e, limit, timeout) {
        return Ok(false);
    }
    let start = Instant::now();
    for snapshot in snapshots {
        if !projection(&mut e, snapshot, n, limit, timeout)? {
            return Ok(false);
        }
    }
    checkpoint("pending_validated", &e, start.elapsed());
    if !release(&mut e, limit, timeout)? {
        return Ok(false);
    }
    let start = Instant::now();
    super::drop_prepared_engine(e, code)?;
    crate::report::emit(
        "phase",
        json!({
            "phase": "pending_drop", "elapsed_ms": ms(start.elapsed()), "prepared_released": true
        }),
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_unposted_prefix_survives_first_capture_cancel_projection_and_release() {
        for n in [4, 16] {
            for case in ["life-pending-cancel", "life-pending-snapshot"] {
                assert!(run(case, n, 1_000_000, Duration::from_secs(5)).unwrap());
            }
        }
    }
}
