#![allow(dead_code)]
mod support;
use support::engine;
#[test]
fn wide_source_condition_gate() {
    let n = 128;
    let mut e = engine("", &format!("({})", vec!["true"; n].join(";")));
    let mut peak = 0;
    for _ in 0..2_000_000 {
        e.advance(1);
        peak = peak.max(e.memory().conditions);
        e.take_output();
        if e.exhausted() {
            break;
        }
    }
    assert!(e.exhausted());
    assert!(peak < 4000, "wide source conditions: {peak}");
}
