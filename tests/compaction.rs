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
        assert!(e.delivery_done());
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
