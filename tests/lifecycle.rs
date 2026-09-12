use chr::engine::Engine;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;
#[test]
fn pending_index_is_reclaimed_across_budget_returns_during_continuing_execution() {
    let code = Arc::new(
        prepare(
            &parse_program("loop(X) <=> loop(X).").unwrap(),
            &parse_query("loop(X)").unwrap(),
        )
        .unwrap(),
    );
    let mut e = Engine::new(code);
    let mut max_nodes = 0;
    for _ in 0..200000 {
        e.advance(1);
        max_nodes = max_nodes.max(e.pending_index_nodes());
    }
    assert!(!e.exhausted());
    assert!(e.applications() > 100);
    assert!(e.pending_collections() > 3);
    assert!(
        max_nodes < 4096,
        "bounded pending work must not retain prior index versions: {max_nodes}"
    );
}
