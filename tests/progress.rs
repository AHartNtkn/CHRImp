use chr::condition::Condition;
use chr::engine::Engine;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;
fn engine(program: &str, query: &str) -> Engine {
    Engine::new(Arc::new(
        prepare(
            &parse_program(program).unwrap(),
            &parse_query(query).unwrap(),
        )
        .unwrap(),
    ))
}
#[test]
fn growing_search_does_not_block_a_finite_sibling_or_budget_return() {
    for budget in [1, 7, 100] {
        let mut e = engine("loop(X) <=> (loop(X);loop(X)).", "loop(X);answer(X)");
        let mut completed = None;
        for _ in 0..20_000 / budget {
            e.advance(budget);
            if let Some(c) = e.take_completion() {
                completed = Some(c);
                break;
            }
        }
        let c = completed.expect("finite sibling must finish despite a growing frontier");
        assert_eq!(c.support, e.choices().next().unwrap().1.decision.not());
        assert!(!e.exhausted());
        let before = e.applications();
        e.advance(10000);
        assert!(e.applications() > before);
        assert!(e.take_completion().is_none());
    }
}
#[test]
fn queued_completion_does_not_stop_source_progress_and_delivery_resumes() {
    let rules = (0..32)
        .map(|i| format!("p{i}(X) <=> p{}(X).", i + 1))
        .collect::<String>();
    let mut e = engine(&rules, "fast(X);p0(X)");
    e.advance(100000);
    assert_eq!(
        e.applications(),
        32,
        "source runs while one completion is queued"
    );
    let first = e.take_completion().expect("first completion retained");
    let mut second = None;
    for _ in 0..10000 {
        e.advance(1);
        if let Some(c) = e.take_completion() {
            second = Some(c);
            break;
        }
    }
    let second = second.expect("remaining completion after output release");
    assert_eq!(first.support, second.support.not());
    assert_ne!(first.id, second.id);
    assert!(e.exhausted());
}
#[test]
fn failure_and_completion_leave_no_stranded_work_or_lane_request() {
    for query in ["fail;done(X)", "(fail;done(X)),(fail;done(Y))", "fail"] {
        let mut e = engine("", query);
        let mut outputs = 0;
        for _ in 0..20000 {
            e.advance(1);
            if e.take_completion().is_some() {
                outputs += 1;
            }
            if e.exhausted() && e.pending_tasks() == 0 {
                break;
            }
        }
        assert!(e.exhausted());
        assert_eq!(e.pending_tasks(), 0);
        if query == "fail" {
            assert_eq!(outputs, 0);
            assert_eq!(e.failed(), Condition::TRUE);
        } else {
            assert!(outputs > 0);
        }
    }
}
