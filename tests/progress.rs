mod support;
use support::{Reader, engine, facts, finish};
#[test]
fn growing_search_does_not_block_a_finite_sibling_or_budget_return() {
    for budget in [1, 7, 100] {
        let mut e = engine("loop(X) <=> (loop(X);loop(X)).", "loop(X);answer(X)");
        let mut reader = Reader::default();
        let mut answer = None;
        for _ in 0..50000 / budget {
            e.advance(budget);
            if let Some(a) = reader.next(&mut e) {
                answer = Some(a);
                break;
            }
        }
        let a = answer.expect("finite sibling must finish despite growing frontier");
        assert_eq!(facts(&e, &a), ["answer"]);
        assert!(!e.exhausted());
        let before = e.applications();
        e.advance(10000);
        assert!(e.applications() > before);
        assert!(reader.next(&mut e).is_none());
    }
}
#[test]
fn queued_output_does_not_stop_source_progress_and_delivery_resumes() {
    let rules = (0..32)
        .map(|i| format!("p{i}(X) <=> p{}(X).", i + 1))
        .collect::<String>();
    let mut e = engine(&rules, "fast(X);p0(X)");
    e.advance(100000);
    assert_eq!(
        e.applications(),
        32,
        "source runs with one output event queued"
    );
    let first = finish(&mut e);
    let second = finish(&mut e);
    assert_ne!(first.id, second.id);
    assert_eq!(facts(&e, &first), ["fast"]);
    assert_eq!(facts(&e, &second), ["p32"]);
    assert!(e.exhausted());
}
#[test]
fn failure_and_completion_leave_no_stranded_work_or_lane_request() {
    for query in ["fail;done(X)", "(fail;done(X)),(fail;done(Y))", "fail"] {
        let mut e = engine("", query);
        let mut reader = Reader::default();
        let mut outputs = 0;
        for _ in 0..20000 {
            e.advance(1);
            if reader.next(&mut e).is_some() {
                outputs += 1;
            }
            if e.delivery_done() && e.pending_tasks() == 0 {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(e.pending_tasks(), 0);
        if query == "fail" {
            assert_eq!(outputs, 0);
        } else {
            assert_eq!(outputs, 1);
        }
    }
}
