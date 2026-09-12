mod support;
use support::engine;

#[test]
fn continuing_single_survivor_reclaims_completed_choices() {
    for body in ["(fail;loop())", "((left();right()),fail;loop())"] {
        let mut e = engine(&format!("loop() <=> {body}."), "loop()");
        for target in [16, 32, 64, 128, 256] {
            for _ in 0..2_000_000 {
                if e.applications() >= target {
                    break;
                }
                e.advance(1);
                assert!(e.take_output().is_none());
            }
            assert!(e.applications() >= target, "fair continuing progress");
            e.request_collection();
            for _ in 0..2_000_000 {
                e.advance(1);
                assert!(e.take_output().is_none());
                if !e.collecting() {
                    break;
                }
            }
            assert!(!e.collecting(), "finite maintenance");
            let memory = e.memory();
            assert!(memory.choices < 16, "application {target}: {memory:?}");
            assert!(memory.conditions < 2048, "application {target}: {memory:?}");
        }
        assert!(!e.exhausted());
    }
}

#[test]
fn compaction_preserves_duplicate_surviving_answers_and_propagation() {
    use support::{Reader, facts, finish};
    let program = "start(X) <=> (fail;live(X)). live(X) ==> witness(X,Y).";
    for (query, applications) in [("start(A),(true;true)", 2), ("(start(A);start(A))", 4)] {
        let mut reference = engine(program, query);
        let first = finish(&mut reference);
        let expected = facts(&reference, &first);
        // The two explicit alternatives share their source applications but remain
        // two answer events even when their graphs are equal.
        let mut e = engine(program, query);
        let mut reader = Reader::default();
        let mut answers = Vec::new();
        for step in 0..100_000 {
            if step % 71 == 0 && !e.collecting() {
                e.request_collection();
            }
            e.advance(1);
            if let Some(answer) = reader.next(&mut e) {
                answers.push(answer);
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(
            e.delivery_done(),
            "query={query} apps={} answers={} gc={} collecting={} memory={:?}",
            e.applications(),
            answers.len(),
            e.collections(),
            e.collecting(),
            e.memory()
        );
        assert_eq!(answers.len(), 2);
        for answer in &answers {
            assert_eq!(facts(&e, answer), expected);
        }
        assert_eq!(e.applications(), applications);
    }
}

#[test]
fn automatic_collection_bounds_a_continuing_choice_stream() {
    let mut e = engine("loop() <=> (fail;loop()).", "loop()");
    let mut peak_choices = 0;
    let mut peak_conditions = 0;
    let mut peak_coordinates = 0;
    for _ in 0..300_000 {
        e.advance(1);
        assert!(e.take_output().is_none());
        let memory = e.memory();
        peak_choices = peak_choices.max(memory.choices);
        peak_conditions = peak_conditions.max(memory.conditions);
        peak_coordinates = peak_coordinates.max(memory.coordinate_records);
    }
    assert!(e.applications() > 256, "{} applications", e.applications());
    assert!(e.collections() > 4);
    assert!(peak_choices < 128, "{peak_choices} choices");
    assert!(
        peak_conditions < 16_384,
        "{peak_conditions} condition nodes"
    );
    assert!(
        peak_coordinates < 512,
        "{peak_coordinates} coordinate records"
    );
    eprintln!(
        "choice stream: {} applications, {} collections, peaks choices={peak_choices}, conditions={peak_conditions}, coordinates={peak_coordinates}",
        e.applications(),
        e.collections()
    );
}

#[test]
fn independent_continuing_alternatives_reclaim_their_local_choices() {
    let mut e = engine("loop(X) <=> (fail;loop(X)).", "(loop(A);loop(B))");
    for target in [32, 64, 128] {
        for _ in 0..2_000_000 {
            if e.applications() >= target {
                break;
            }
            e.advance(1);
            assert!(e.take_output().is_none());
        }
        assert!(e.applications() >= target);
        e.request_collection();
        for _ in 0..2_000_000 {
            e.advance(1);
            if !e.collecting() {
                break;
            }
        }
        assert!(!e.collecting());
        assert!(e.memory().choices < 16, "target {target}: {:?}", e.memory());
        assert_eq!(
            e.choices().next().map(|(&id, _)| id),
            Some(0),
            "the surviving sibling distinction must remain"
        );
    }
}

// Each new (a();b()) is forced to agree with the initial left/right choice.
// Both arms survive globally, but the repeated choices add no independent answer.
const DEPENDENT_CHOICE_RULES: &str = r"
    left() \ a() <=> true.
    right() \ b() <=> true.
    left(),b() <=> fail.
    right(),a() <=> fail.
";

#[test]
fn dependent_choices_preserve_finite_answers_and_duplicates() {
    use support::{Reader, facts};
    for iterations in [4, 8, 16] {
        let mut program = DEPENDENT_CHOICE_RULES.to_owned();
        for i in 0..iterations {
            program.push_str(&format!(" l{i}() <=> l{}(),(a();b()).", i + 1));
        }
        for (query, copies) in [
            ("(left();right()),l0()", 1),
            ("(left();right()),l0(),(true;true)", 2),
        ] {
            let mut e = engine(&program, query);
            let mut reader = Reader::default();
            let mut answers = Vec::new();
            for tick in 0..5_000_000 {
                if tick % 997 == 0 && !e.collecting() {
                    e.request_collection();
                }
                e.advance(1);
                if let Some(answer) = reader.next(&mut e) {
                    answers.push(answer);
                }
                if e.delivery_done() {
                    break;
                }
            }
            assert!(e.delivery_done(), "iterations={iterations}, query={query}");
            assert!(e.exhausted());
            assert_eq!(answers.len(), 2 * copies);
            let ids = answers
                .iter()
                .map(|a| a.id)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(ids.len(), answers.len(), "distinct answer events survive");
            let mut actual = answers.iter().map(|a| facts(&e, a)).collect::<Vec<_>>();
            actual.sort();
            let mut expected = Vec::new();
            for side in ["left", "right"] {
                for _ in 0..copies {
                    let mut row = vec![format!("l{iterations}"), side.to_owned()];
                    row.sort();
                    expected.push(row);
                }
            }
            expected.sort();
            assert_eq!(
                actual, expected,
                "only the two consistent histories survive"
            );
        }
    }
}

#[test]
fn continuing_dependent_choices_are_bounded_after_collection() {
    let program = format!("loop() <=> loop(),(a();b()). {DEPENDENT_CHOICE_RULES}");
    let mut e = engine(&program, "(left();right()),loop()");
    for target in [16, 32, 64, 128, 256] {
        for _ in 0..5_000_000 {
            if e.applications() >= target {
                break;
            }
            e.advance(1);
            assert!(e.take_output().is_none());
        }
        assert!(e.applications() >= target, "fair progress to {target}");
        e.request_collection();
        for _ in 0..5_000_000 {
            if !e.collecting() {
                break;
            }
            e.advance(1);
            assert!(e.take_output().is_none());
        }
        assert!(!e.collecting(), "finite collection at {target}");
        let memory = e.memory();
        assert_eq!(memory.snapshots, 0);
        assert_eq!(memory.inspections, 0);
        // One independent persistent choice plus a small unfinished frontier;
        // this ceiling must not grow with the number of completed iterations.
        assert!(memory.choices < 16, "target {target}: {memory:?}");
        assert!(!e.exhausted());
    }
}

// a is selected exactly for XOR(left, up); b for equivalence. Neither a
// constant nor a single older coordinate can express all four surviving cases.
const XOR_CHOICE_RULES: &str = r"
    left(),down() \ a() <=> true.
    right(),up() \ a() <=> true.
    left(),up() \ b() <=> true.
    right(),down() \ b() <=> true.
    left(),up(),a() <=> fail.
    right(),down(),a() <=> fail.
    left(),down(),b() <=> fail.
    right(),up(),b() <=> fail.
";

#[test]
fn xor_dependent_choices_preserve_four_finite_answers() {
    use support::{Reader, facts};
    for iterations in [4, 8, 16] {
        let mut program = XOR_CHOICE_RULES.to_owned();
        for i in 0..iterations {
            let choice = if i + 1 == iterations {
                "(a(),first();b(),second())"
            } else {
                "(a();b())"
            };
            program.push_str(&format!(" l{i}() <=> l{}(),{choice}.", i + 1));
        }
        let mut e = engine(&program, "(left();right()),(up();down()),l0()");
        let mut reader = Reader::default();
        let mut answers = Vec::new();
        for tick in 0..5_000_000 {
            if tick % 997 == 0 && !e.collecting() {
                e.request_collection();
            }
            e.advance(1);
            if let Some(answer) = reader.next(&mut e) {
                answers.push(answer);
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done(), "iterations={iterations}");
        assert!(e.exhausted());
        assert_eq!(answers.len(), 4);
        assert_eq!(
            answers
                .iter()
                .map(|a| a.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        let mut actual = answers.iter().map(|a| facts(&e, a)).collect::<Vec<_>>();
        actual.sort();
        let mut expected = Vec::new();
        for (side, vertical, outcome) in [
            ("left", "up", "second"),
            ("left", "down", "first"),
            ("right", "up", "first"),
            ("right", "down", "second"),
        ] {
            let mut row = vec![
                format!("l{iterations}"),
                side.into(),
                vertical.into(),
                outcome.into(),
            ];
            row.sort();
            expected.push(row);
        }
        expected.sort();
        assert_eq!(
            actual, expected,
            "the final outcome is XOR of both older choices"
        );
        assert_eq!(
            e.memory().snapshots,
            0,
            "ordinary execution records no history"
        );
        assert_eq!(e.memory().inspections, 0);
    }
}

fn advance_xor_to(e: &mut chr::engine::Engine, target: u64) {
    for _ in 0..5_000_000 {
        if e.applications() >= target {
            break;
        }
        e.advance(1);
        assert!(e.take_output().is_none());
    }
    assert!(e.applications() >= target, "fair XOR progress to {target}");
    assert!(!e.exhausted());
}

fn collect_xor(e: &mut chr::engine::Engine) {
    e.request_collection();
    for _ in 0..5_000_000 {
        if !e.collecting() {
            break;
        }
        e.advance(1);
        assert!(e.take_output().is_none());
    }
    assert!(!e.collecting(), "finite XOR collection");
}

#[test]
fn continuing_xor_dependent_choices_are_bounded_after_collection() {
    let program = format!("loop() <=> loop(),(a();b()). {XOR_CHOICE_RULES}");
    let mut e = engine(&program, "(left();right()),(up();down()),loop()");
    for target in [32, 64, 128, 256, 512] {
        advance_xor_to(&mut e, target);
        collect_xor(&mut e);
        let memory = e.memory();
        assert_eq!(memory.snapshots, 0, "history is off; no user-held views");
        assert_eq!(memory.inspections, 0);
        // Two independent choices plus bounded unfinished work, regardless of
        // how many previous outcomes were functions of those two coordinates.
        assert!(memory.choices < 16, "target {target}: {memory:?}");
    }
}

#[test]
fn xor_snapshot_pins_choices_until_released() {
    use chr::engine::InspectionError;
    let program = format!("loop() <=> loop(),(a();b()). {XOR_CHOICE_RULES}");
    let mut e = engine(&program, "(left();right()),(up();down()),loop()");
    assert_eq!(e.memory().snapshots, 0);
    let mut held = None;
    // Refresh one explicit snapshot often enough to retain the growing prefix.
    // This is intentional user ownership, unlike the unpinned workload above.
    for target in (8..=256).step_by(8) {
        advance_xor_to(&mut e, target);
        let mut captured = None;
        for _ in 0..5_000_000 {
            match e.capture_snapshot() {
                Ok(id) => {
                    captured = Some(id);
                    break;
                }
                Err(InspectionError::Busy | InspectionError::Initializing) => {
                    e.advance(1);
                    assert!(e.take_output().is_none());
                }
                Err(error) => panic!("snapshot capture: {error:?}"),
            }
        }
        let snapshot = captured.expect("finite snapshot capture");
        if let Some(previous) = held.replace(snapshot) {
            e.release_snapshot(previous).unwrap();
        }
        assert_eq!(e.memory().snapshots, 1);
    }
    let held = held.unwrap();
    let cutoff = e.snapshot_info(held).unwrap().last_choice.unwrap();
    let pinned = e
        .choices()
        .filter(|(id, _)| **id <= cutoff)
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    assert!(
        pinned.len() >= 16,
        "exercise ownership beyond the unpinned ceiling"
    );
    collect_xor(&mut e);
    for id in &pinned {
        assert!(
            e.choices().any(|(&live, _)| live == *id),
            "snapshot still owns choice {id}"
        );
    }
    assert_eq!(e.memory().snapshots, 1);
    assert_eq!(e.memory().inspections, 0);
    e.release_snapshot(held).unwrap();
    collect_xor(&mut e);
    let memory = e.memory();
    assert_eq!(memory.snapshots, 0);
    assert!(
        memory.choices < 16,
        "released snapshot permits compaction: {memory:?}"
    );
}
