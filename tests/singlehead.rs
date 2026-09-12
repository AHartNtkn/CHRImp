mod support;
use support::{Reader, engine, facts};

#[test]
fn single_head_conditional_repetition_freshness_and_history_under_gc() {
    let mut e = engine(
        "p(X,X) ==> hit(X,F). link(X,Y) <=> X=Y.",
        "p(A,B),p(A,B),(link(A,B);true)",
    );
    let mut reader = Reader::default();
    let mut answers = Vec::new();
    let mut service = 0;
    let mut next_gc = 11;
    for _ in 0..1_000_000 {
        if !e.collecting() && service >= next_gc {
            e.request_collection();
            next_gc = service + 11;
        }
        let collecting = e.collecting();
        e.advance(1);
        if !collecting {
            service += 1;
        }
        if let Some(a) = reader.next(&mut e) {
            answers.push(a);
        }
        if e.delivery_done() {
            break;
        }
    }
    assert!(e.delivery_done());
    assert_eq!(answers.len(), 2);
    assert_eq!(e.applications(), 3);
    for a in answers {
        let merged = a.variables[0] == a.variables[1];
        assert_eq!(
            facts(&e, &a),
            if merged {
                vec!["hit", "hit", "p", "p"]
            } else {
                vec!["p", "p"]
            }
        );
        let hits = a
            .rows
            .iter()
            .filter(|r| e.program().signatures[r.relation].name == "hit")
            .collect::<Vec<_>>();
        if merged {
            assert_ne!(hits[0].ports[1], hits[1].ports[1]);
            for hit in hits {
                assert_eq!(hit.ports[0], a.variables[0]);
                assert!(!a.variables.contains(&hit.ports[1]));
            }
        }
    }
}

#[test]
fn single_head_rejects_unmerged_repeated_ports() {
    let mut e = engine("p(X,Y,X) <=> bad(Y).", "p(A,B,C)");
    let a = support::finish(&mut e);
    assert_eq!(facts(&e, &a), ["p"]);
    assert_eq!(a.rows[0].ports, a.variables);
    assert_eq!(e.applications(), 0);
}
