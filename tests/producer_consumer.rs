#[allow(dead_code)]
mod support;
use chr::{
    engine::{Engine, SnapshotKind},
    program::prepare,
    syntax::{parse_program, parse_query},
};
use std::sync::Arc;
use support::{Answer, Reader};

const THEORY: &str = "copper(X,Y) \\ copper(X,Z) <=> Y=Z. tide(X) \\ tide(X) <=> true. copper(X,Y),tide(X) <=> fail.";
fn engine(extra: &str, query: &str) -> Engine {
    Engine::with_history(
        Arc::new(
            prepare(
                &parse_program(&format!("{THEORY} {extra}")).unwrap(),
                &parse_query(query).unwrap(),
            )
            .unwrap(),
        ),
        true,
    )
}
fn answers(e: &mut Engine) -> Vec<Answer> {
    let mut r = Reader::default();
    let mut out = vec![];
    for _ in 0..2_000_000 {
        e.advance(1);
        if let Some(a) = r.next(e) {
            out.push(a);
        }
        if e.delivery_done() {
            return out;
        }
    }
    panic!("finite execution did not complete")
}
fn count(e: &Engine, a: &Answer, name: &str) -> usize {
    a.rows
        .iter()
        .filter(|r| e.program().signatures()[r.relation].name == name)
        .count()
}
#[test]
fn renamed_consumer_rejects_before_birth_and_keeps_original_surviving_body() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X),orbit(X)).",
        "crane(A),route(A)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 1);
    assert_eq!(count(&e, &out[0], "orbit"), 2);
    assert_eq!(count(&e, &out[0], "tide"), 1);
    assert_eq!(count(&e, &out[0], "crane"), 1);
    assert_eq!(count(&e, &out[0], "velvet"), 0);
    assert!(
        !e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Choice)),
        "a proven failing arm must not allocate a search decision"
    );
}

#[test]
fn incompatible_kept_tag_certifies_surviving_marker_retirement() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. tide(X) \\ crane(X) <=> true. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),route(A)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 1);
    assert_eq!(count(&e, &out[0], "orbit"), 1);
    assert_eq!(count(&e, &out[0], "crane"), 0);
    assert!(
        !e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Choice)),
        "the retained incompatible tag must certify rejection even though the marker can retire"
    );
}

#[test]
fn conditional_identity_rejects_only_the_supported_arm() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),(A=B;true),route(B)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 3);
    let mut cases = out
        .iter()
        .map(|a| {
            (
                a.variables[0] == a.variables[1],
                count(&e, a, "copper"),
                count(&e, a, "tide"),
            )
        })
        .collect::<Vec<_>>();
    cases.sort();
    assert_eq!(cases, [(false, 0, 1), (false, 1, 0), (true, 0, 1)]);
}

#[test]
fn consumer_field_and_repeated_local_tests_never_bind_identities() {
    let mut e = engine(
        "copper(X,Y),crane(X,Y,Z,Z) <=> fail. route(X,Y) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A,B,C,D),(C=D;true),route(A,B)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 3);
    let mut cases = out
        .iter()
        .map(|a| (a.variables[2] == a.variables[3], count(&e, a, "copper")))
        .collect::<Vec<_>>();
    cases.sort();
    assert_eq!(cases, [(false, 0), (false, 1), (true, 0)]);
}

#[test]
fn mutable_marker_disqualifies_guard_even_when_eraser_is_not_enabled_yet() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. crane(X),erase(X) <=> true. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),route(A)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 1);
    assert_eq!(count(&e, &out[0], "tide"), 1);
    // Current liveness is insufficient: this marker has no persistent witness.
    assert!(
        e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Choice))
    );
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. crane(X),erase(X) <=> true. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),erase(A),route(A)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 2);
    assert_eq!(out.iter().map(|a| count(&e, a, "copper")).sum::<usize>(), 1);
}

#[test]
fn rhs_or_wrong_key_constructor_does_not_certify_marker_retirement() {
    for consumer in [
        "crane(X),erase(X) <=> tide(X).",
        "tide(Y) \\ crane(X),erase(X) <=> true.",
    ] {
        let mut e = engine(
            &format!(
                "copper(X,Y),crane(X) <=> fail. {consumer} route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X))."
            ),
            "crane(A),route(A)",
        );
        let out = answers(&mut e);
        assert_eq!(out.len(), 1);
        assert!(
            e.snapshots()
                .any(|s| matches!(s.kind, SnapshotKind::Choice))
        );
    }
}

#[test]
fn duplicate_source_arms_and_empty_context_keep_all_alternatives() {
    for query in ["crane(A),route(B)", "route(B)"] {
        let mut e = engine(
            "copper(X,Y),crane(X) <=> fail. route(X) <=> (copper(X,Y),velvet(Y);copper(X,Z),velvet(Z);tide(X),orbit(X)).",
            query,
        );
        let out = answers(&mut e);
        assert_eq!(out.len(), 3);
        assert_eq!(out.iter().map(|a| count(&e, a, "velvet")).sum::<usize>(), 2);
    }
}

#[test]
fn actual_behavior_evaluator_rejects_constant_before_its_binary_subsplit() {
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../examples/behavior-synthesis.chrnb")).unwrap();
    let program = serde_json::from_value(document["program"].clone()).unwrap();
    let code = prepare(
        &program,
        &parse_query("no_c(T),nil(Sp),eval(T,Sp,O)").unwrap(),
    )
    .unwrap();
    let body = code
        .rules()
        .iter()
        .find(|r| {
            r.heads
                .iter()
                .any(|h| code.signatures()[h.relation].name == "nil")
                && r.heads
                    .iter()
                    .any(|h| code.signatures()[h.relation].name == "eval")
        })
        .unwrap()
        .body;
    let mut e = Engine::with_history(Arc::new(code), true);
    let mut reader = Reader::default();
    let mut found = vec![];
    for _ in 0..1_000_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            found.push(a);
        }
        if found.len() == 2 {
            break;
        }
    }
    assert_eq!(found.len(), 2);
    for a in &found {
        assert_ne!(a.variables[0], a.variables[2]);
        assert_eq!(count(&e, a, "constant"), 0);
        assert_eq!(count(&e, a, "no_c"), 0);
        let tags = (count(&e, a, "k"), count(&e, a, "s"));
        assert!(matches!(tags, (2, 0) | (0, 2)));
    }
    assert!(
        e.choices()
            .any(|(_, b)| b.instruction == body && b.start == 0 && b.end == 4)
    );
    assert!(
        !e.choices()
            .any(|(_, b)| b.instruction == body && b.start == 0 && b.end == 2),
        "constant/k subrange has only k viable under the actual no_c theory"
    );
    e.cancel();
    for _ in 0..1_000_000 {
        e.advance(1);
        if e.cancel_done() {
            break;
        }
    }
    assert!(e.cancel_done());
}

#[test]
fn conditional_all_failed_scope_does_not_escape_to_siblings() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. tide(X),stone(X) <=> fail. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "(crane(A),stone(A);crane(A);stone(A);true),route(A)",
    );
    let out = answers(&mut e);
    assert_eq!(out.len(), 4);
    let mut cases = out
        .iter()
        .map(|a| {
            (
                count(&e, a, "crane"),
                count(&e, a, "stone"),
                count(&e, a, "copper"),
                count(&e, a, "tide"),
            )
        })
        .collect::<Vec<_>>();
    cases.sort();
    assert_eq!(
        cases,
        [(0, 0, 0, 1), (0, 0, 1, 0), (0, 1, 1, 0), (1, 0, 0, 1)]
    );
}

#[test]
fn source_step_exposes_original_disjunction_before_compiled_rejection() {
    use chr::observe::{ExpressionKind, Output};
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. tide(X) \\ crane(X) <=> true. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),route(A)",
    );
    e.request_step(vec![]).unwrap();
    for _ in 0..200_000 {
        e.advance(1);
        if e.step_status().done {
            break;
        }
    }
    assert!(e.step_status().done);
    assert_eq!(e.applications(), 1);
    let event = e.step_status().event.unwrap();
    let snapshot = e
        .snapshots()
        .find(|s| matches!(s.kind, SnapshotKind::Application { event: id, .. } if id == event))
        .unwrap()
        .id;
    let id = e.start_inspection(Some(snapshot), vec![]).unwrap();
    let mut saw_or = false;
    let mut names = vec![];
    for _ in 0..200_000 {
        e.advance_inspection(id, 1).unwrap();
        if let Some(o) = e.take_inspection_output(id).unwrap() {
            match o {
                Output::Expression {
                    operator: ExpressionKind::Or,
                } => saw_or = true,
                Output::ExpressionRelation { relation } => {
                    names.push(e.program().signatures()[relation].name.clone())
                }
                _ => {}
            }
        }
        if e.inspection_status(id).unwrap().done {
            break;
        }
    }
    assert!(e.inspection_status(id).unwrap().done);
    e.release_inspection(id).unwrap();
    assert!(saw_or);
    assert_eq!(names, ["copper", "velvet", "tide", "orbit"]);
    e.resume().unwrap();
    let out = answers(&mut e);
    assert_eq!(out.len(), 1);
    assert_eq!(count(&e, &out[0], "orbit"), 1);
    assert!(
        !e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Choice))
    );
}

#[test]
fn conditional_rejection_survives_collection_with_retained_history() {
    let mut e = engine(
        "copper(X,Y),crane(X) <=> fail. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).",
        "crane(A),(A=B;true),route(B)",
    );
    let mut reader = Reader::default();
    let mut cases = vec![];
    for _ in 0..200_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            cases.push((
                a.variables[0] == a.variables[1],
                count(&e, &a, "copper"),
                count(&e, &a, "tide"),
            ));
        }
        e.request_collection();
        e.maintain(200_000);
        assert!(!e.collecting());
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    assert!(e.snapshots().count() > 1);
    cases.sort();
    assert_eq!(cases, [(false, 0, 1), (false, 1, 0), (true, 0, 1)]);
}

#[test]
fn compatible_family_cannot_certify_marker_retirement_as_failure() {
    let source = "other(X) \\ other(X) <=> true. copper(X,Y),crane(X) <=> fail. other(X) \\ crane(X) <=> true. route(X) <=> (copper(X,Y),velvet(Y);tide(X),orbit(X)).";
    let mut e = engine(source, "crane(A),route(A)");
    let out = answers(&mut e);
    assert_eq!(out.len(), 1);
    assert_eq!(count(&e, &out[0], "tide"), 1);
    assert!(
        e.snapshots()
            .any(|s| matches!(s.kind, SnapshotKind::Choice))
    );
    let mut e = engine(source, "crane(A),other(A),route(A)");
    let out = answers(&mut e);
    assert_eq!(out.len(), 2);
    assert_eq!(out.iter().map(|a| count(&e, a, "copper")).sum::<usize>(), 1);
    for a in &out {
        assert_eq!(count(&e, a, "other"), 1);
        assert_eq!(count(&e, a, "crane"), 0);
        assert_eq!(a.rows.len(), 3);
    }
}
