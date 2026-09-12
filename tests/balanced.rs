#![allow(dead_code)]
mod support;
use support::{Reader, engine, facts};
#[test]
fn nested_duplicate_failure_and_fresh_correlated_arms() {
    let mut e = engine(
        "p(X) <=> q(X,Y).",
        "(p(A);p(A);fail;(p(A);(p(A);fail))),tag(A)",
    );
    let mut reader = Reader::default();
    let mut answers = 0;
    for _ in 0..500_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            answers += 1;
            assert_eq!(facts(&e, &a), ["q", "tag"]);
            let q = a
                .rows
                .iter()
                .find(|r| e.program().signatures()[r.relation].name == "q")
                .unwrap();
            assert_eq!(q.ports[0], a.variables[0]);
            assert_ne!(q.ports[1], a.variables[0]);
        }
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    assert_eq!(answers, 4);
    assert_eq!(e.applications(), 4);
}
#[test]
fn two_flat_choices_cross_product_and_common_work_are_preserved() {
    let mut e = engine(
        "p(X) <=> q(X). q(X) <=> done(X).",
        "(true;true;true;true;true),(true;true;true),p(A)",
    );
    let mut reader = Reader::default();
    let mut answers = 0;
    for _ in 0..500_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            answers += 1;
            assert_eq!(facts(&e, &a), ["done"]);
        }
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    assert_eq!(answers, 15);
    assert_eq!(e.applications(), 2);
}
#[test]
fn flat_finite_siblings_are_fair_and_cancel_reclaims_each_prefix() {
    for prefix in 0..180 {
        let mut e = engine(
            "loop(X) <=> loop(X).",
            "(loop(A);answer(A);fail;answer(A);loop(A);answer(A);answer(A))",
        );
        for _ in 0..prefix {
            e.advance(1);
            e.take_output();
            if prefix % 7 == 0 {
                e.request_collection();
                e.maintain(1);
            }
        }
        e.cancel();
        for _ in 0..200_000 {
            e.advance(1);
            e.take_output();
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done(), "prefix {prefix}");
        let m = e.memory();
        assert_eq!(
            (
                m.graph_nodes,
                m.occurrences,
                m.conditions,
                m.pending_nodes,
                m.obligation_descriptors,
                m.choices
            ),
            (0, 0, 0, 0, 0, 0)
        );
    }
    let mut e = engine(
        "loop(X) <=> loop(X).",
        "(loop(A);answer(A);fail;answer(A);loop(A);answer(A);answer(A))",
    );
    let mut reader = Reader::default();
    let mut answers = 0;
    for _ in 0..200_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            assert_eq!(facts(&e, &a), ["answer"]);
            answers += 1;
        }
        if answers == 4 {
            break;
        }
    }
    assert_eq!(answers, 4);
    assert!(!e.exhausted());
}
#[test]
fn conditional_identity_stays_correlated_with_selected_arms() {
    let mut e = engine(
        "tag(X),p(X) ==> hit(X).",
        "(A=B,tag(A);A=C,tag(A);tag(A)),(p(B);q(C))",
    );
    let mut reader = Reader::default();
    let mut seen = Vec::new();
    let mut hits = 0;
    for _ in 0..500_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            let v = &a.variables;
            let b = v[0] == v[1];
            let c = v[0] == v[2];
            let has_p = a
                .rows
                .iter()
                .any(|r| e.program().signatures()[r.relation].name == "p");
            let has_hit = a
                .rows
                .iter()
                .any(|r| e.program().signatures()[r.relation].name == "hit");
            assert_eq!(has_hit, b && has_p);
            hits += usize::from(has_hit);
            seen.push((b, c, has_p));
        }
        if e.delivery_done() {
            break;
        }
    }
    seen.sort();
    let mut expected = vec![
        (true, false, true),
        (true, false, false),
        (false, true, true),
        (false, true, false),
        (false, false, true),
        (false, false, false),
    ];
    expected.sort();
    assert_eq!(seen, expected);
    assert_eq!(hits, 1);
    assert_eq!(e.applications(), 1);
}
