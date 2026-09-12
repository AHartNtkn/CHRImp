use chr::{
    condition::{Arena, Condition, Operation, Progress},
    store::Store,
};
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn repeated_support_substitution_scales_with_leaves_plus_distinct_conditions() {
    let mut arena = Arena::default();
    let mut input = Condition::TRUE;
    let mut expected = Condition::TRUE;
    let mut assignments = BTreeMap::new();
    for i in 0..64 {
        let (id, c) = arena.fresh_choice();
        if i == 63 {
            expected = input;
            assignments.insert(id, Condition::TRUE);
        }
        let mut job = arena.start(Operation::And(input, c));
        input = loop {
            if let Progress::Complete(c) = job.tick(&mut arena) {
                break c;
            }
        };
    }
    let mut store = Store::default();
    let mut root = store.empty();
    for i in 0..256 {
        root = store.insert(root, [i, 0, 0, 0], input);
    }
    let mut job = store.substitute(root.clone(), Arc::new(assignments));
    let mut ticks = 0;
    let result = loop {
        ticks += 1;
        if let Some(root) = job.tick(&mut store, &mut arena) {
            break root;
        }
        assert!(ticks < 200_000);
    };
    let value = store.get(&result, &[0; 4]).unwrap();
    assert_eq!(value, expected);
    for i in 0..256 {
        assert_eq!(store.get(&root, &[i, 0, 0, 0]), Some(input));
        assert_eq!(store.get(&result, &[i, 0, 0, 0]), Some(value));
    }
    assert!(ticks < 4_000, "repeated support took {ticks} steps");
}

#[test]
fn fixed_and_rotating_snapshots_preserve_the_working_set_reclamation_bound() {
    use chr::{
        engine::Engine,
        program::prepare,
        syntax::{parse_program, parse_query},
    };
    for rotate in [false, true] {
        let code = prepare(
            &parse_program("loop(X) <=> loop(X).").unwrap(),
            &parse_query("keep(A),loop(A)").unwrap(),
        )
        .unwrap();
        let mut e = Engine::new(Arc::new(code));
        while e.applications() < 4 {
            e.advance(1);
        }
        let mut view = e.capture_snapshot().unwrap();
        let mut retained_occurrences = (0..2).map(|r| e.facts(r).unwrap().count()).sum::<usize>();
        let mut peak = 0;
        let mut next_rotation = 256;
        for _ in 0..400_000 {
            e.advance(1);
            peak = peak.max(e.memory().occurrences);
            if rotate && e.applications() >= next_rotation && !e.collecting() {
                e.release_snapshot(view).unwrap();
                view = e.capture_snapshot().unwrap();
                retained_occurrences = (0..2).map(|r| e.facts(r).unwrap().count()).sum();
                next_rotation += 256;
            }
        }
        assert!(e.applications() > 1024);
        assert!(
            peak < 512,
            "fixed archive rotate={rotate}: {peak} occurrences"
        );
        e.cancel();
        for _ in 0..100_000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert_eq!(
            e.memory().occurrences,
            retained_occurrences,
            "exact retained keep/loop payloads"
        );
        e.release_snapshot(view).unwrap();
        e.maintain(100_000);
        assert!(!e.collecting());
        let m = e.memory();
        assert_eq!(
            m.occurrences
                + m.graph_nodes
                + m.pending_nodes
                + m.obligation_descriptors
                + m.conditions
                + m.snapshots,
            0
        );
    }
}
