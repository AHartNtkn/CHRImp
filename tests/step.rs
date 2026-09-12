mod support;
use chr::engine::{Engine, InspectionError};
use support::engine;

fn finish_step(e: &mut Engine) {
    for _ in 0..200_000 {
        e.advance(1);
        if e.step_status().done {
            return;
        }
    }
    panic!("step did not finish");
}

#[test]
fn one_application_is_not_one_service_tick_and_done_stays_paused() {
    let mut e = engine("p(X) <=> p(X).", "p(A)");
    e.request_step(vec![]).unwrap();
    e.advance(1);
    assert!(!e.step_status().done);
    assert_eq!(e.applications(), 0);
    finish_step(&mut e);
    assert_eq!(e.applications(), 1);
    assert_eq!(e.step_status().rule, Some(0));
    assert_eq!(e.step_status().shared, Some(false));
    let first = e.step_status().event.unwrap();
    e.advance(10000);
    assert_eq!(e.applications(), 1);
    e.request_step(vec![]).unwrap();
    finish_step(&mut e);
    assert_eq!(e.applications(), 2);
    assert_ne!(e.step_status().event.unwrap(), first);
    e.resume().unwrap();
    e.advance(10000);
    assert!(e.applications() > 2);
}

#[test]
fn no_application_finishes_for_normal_form_failure_and_contradictory_selection() {
    for query in ["p(A)", "fail"] {
        let mut e = engine("", query);
        e.request_step(vec![]).unwrap();
        finish_step(&mut e);
        assert_eq!(e.applications(), 0);
        assert_eq!(e.step_status().event, None);
    }
    let mut e = engine("p(X) <=> p(X).", "(p(A);p(A))");
    while e.choices().next().is_none() {
        e.advance(1);
    }
    let choice = *e.choices().next().unwrap().0;
    e.request_step(vec![(choice, true), (choice, false)])
        .unwrap();
    finish_step(&mut e);
    assert_eq!(e.step_status().event, None);
    assert_eq!(e.applications(), 0);
    assert_eq!(
        e.request_step(vec![(u64::MAX, true)]),
        Err(InspectionError::UnknownChoice)
    );
}

#[test]
fn unrelated_applications_do_not_finish_the_selected_step() {
    let mut e = engine(
        "left(X) <=> left(X). right(X) <=> right(X).",
        "(left(A);right(A))",
    );
    while e.choices().next().is_none() {
        e.advance(1);
    }
    let choice = *e.choices().next().unwrap().0;
    e.request_step(vec![(choice, false)]).unwrap();
    finish_step(&mut e);
    assert_eq!(e.step_status().rule, Some(1));
    assert_eq!(e.step_status().shared, Some(false));
    let count = e.applications();
    e.advance(10000);
    assert_eq!(e.applications(), count);
}

#[test]
fn shared_application_reports_support_outside_selection() {
    let mut e = engine("p(X) <=> p(X).", "p(A),(true;true)");
    while e.choices().next().is_none() {
        e.advance(1);
    }
    let choice = *e.choices().next().unwrap().0;
    let before = e.applications();
    e.request_step(vec![(choice, true)]).unwrap();
    finish_step(&mut e);
    assert_eq!(e.applications(), before + 1);
    assert_eq!(e.step_status().shared, Some(true));
}

#[test]
fn selected_normal_form_finishes_with_a_diverging_sibling_and_unread_output() {
    let mut e = engine("loop(X) <=> loop(X).", "(done(A);loop(A))");
    while e.choices().next().is_none() {
        e.advance(1);
    }
    let choice = *e.choices().next().unwrap().0;
    e.request_step(vec![(choice, true)]).unwrap();
    finish_step(&mut e);
    assert_eq!(e.step_status().event, None);
    e.resume().unwrap();
    let answer = support::finish(&mut e);
    assert_eq!(support::facts(&e, &answer), ["done"]);
}

#[test]
fn suspended_step_survives_collection_and_can_resume_before_finishing() {
    for prefix in (0..350).step_by(7) {
        let mut e = engine("p(X) <=> p(X).", "p(A),(true;true),(true;true)");
        while e.choices().count() < 2 {
            e.advance(1);
        }
        let choices = e.choices().map(|(&id, _)| (id, true)).collect();
        e.request_step(choices).unwrap();
        let before = e.applications();
        e.advance(prefix);
        e.request_collection();
        for _ in 0..200000 {
            e.advance(1);
            if !e.collecting() {
                break;
            }
        }
        assert!(!e.collecting(), "collection stuck at prefix {prefix}");
        finish_step(&mut e);
        assert_eq!(e.applications(), before + 1);
        e.resume().unwrap();
        e.advance(5000);
        assert!(e.applications() > before + 1);
    }
    let mut e = engine("p(X) <=> p(X).", "p(A)");
    e.request_step(vec![]).unwrap();
    e.advance(1);
    e.resume().unwrap();
    e.advance(10000);
    assert!(e.applications() > 1);
}

#[test]
fn source_cancellation_drains_both_unfinished_and_paused_steps() {
    for prefix in [1, 25, 5000] {
        let mut e = engine("p(X) <=> p(X).", "p(A)");
        e.request_step(vec![]).unwrap();
        e.advance(prefix);
        let before = e.applications();
        e.cancel();
        for _ in 0..200_000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done(), "cancellation stuck at {prefix}");
        assert!(e.step_status().done);
        assert_eq!(e.applications(), before);
        assert_eq!(e.memory().graph_nodes, 0);
        assert_eq!(e.request_step(vec![]), Err(InspectionError::Canceled));
        assert_eq!(e.resume(), Err(InspectionError::Canceled));
    }
}

#[test]
fn nested_choice_selection_excludes_its_inactive_sibling() {
    let mut e = engine(
        "left(X) <=> left(X). right(X) <=> right(X). other(X) <=> other(X).",
        "(left(A);(right(A);other(A)))",
    );
    while e.choices().count() < 2 {
        e.advance(1);
    }
    let ids = e.choices().map(|(&id, _)| id).collect::<Vec<_>>();
    e.request_step(vec![(ids[1], true)]).unwrap();
    finish_step(&mut e);
    assert_eq!(e.step_status().rule, Some(1));
    assert_eq!(e.step_status().shared, Some(false));
    let count = e.applications();
    e.request_step(vec![(ids[0], true), (ids[1], true)])
        .unwrap();
    finish_step(&mut e);
    assert_eq!(e.step_status().event, None);
    assert_eq!(e.applications(), count);
}

#[test]
fn paused_selected_step_can_be_inspected_after_collection_until_resume() {
    use chr::observe::Output;
    let mut e = engine("p(X) <=> p(X).", "(fail;p(A))");
    while e.choices().next().is_none() {
        e.advance(1);
    }
    let choice = *e.choices().next().unwrap().0;
    let selection = vec![(choice, false)];
    e.request_step(selection.clone()).unwrap();
    finish_step(&mut e);
    let applications = e.applications();
    assert_eq!(e.step_status().rule, Some(0));
    e.request_collection();
    for _ in 0..200_000 {
        e.advance(1);
        if !e.collecting() {
            break;
        }
    }
    assert!(!e.collecting());
    assert_eq!(e.applications(), applications);
    let view = e.start_inspection(None, selection).unwrap();
    let mut alternatives = 0;
    for _ in 0..200_000 {
        e.advance_inspection(view, 1).unwrap();
        if matches!(
            e.take_inspection_output(view).unwrap(),
            Some(Output::Begin { .. })
        ) {
            alternatives += 1;
        }
        let status = e.inspection_status(view).unwrap();
        assert_eq!(
            status.error, None,
            "the paused selection still owns its choice IDs"
        );
        if status.done {
            break;
        }
    }
    assert!(e.inspection_status(view).unwrap().done);
    assert_eq!(alternatives, 1);
    assert_eq!(e.applications(), applications);
    e.release_inspection(view).unwrap();
    // Resuming releases the paused selection; its fixed coordinate can retire.
    e.resume().unwrap();
    e.advance(1000);
    e.request_collection();
    for _ in 0..200_000 {
        e.advance(1);
        if !e.collecting() {
            break;
        }
    }
    assert!(!e.collecting());
    assert!(e.choices().all(|(&id, _)| id != choice));
    assert!(e.applications() > applications);
}
