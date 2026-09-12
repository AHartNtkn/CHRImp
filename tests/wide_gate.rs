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

#[test]
fn complete_delivery_scales_with_supported_alternatives_and_rows() {
    use chr::observe::Output;
    for shape in ["empty", "same", "distinct", "nested"] {
        let mut first_work = 0;
        for n in [64, 96, 128, 192, 256] {
            let query = (0..n)
                .map(|i| match shape {
                    "empty" => "true".to_owned(),
                    "same" => "p(A)".to_owned(),
                    "distinct" => format!("p{i}(A)"),
                    _ => "(p(A);q(A))".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(";");
            let mut e = engine("", &query);
            let mut ids = std::collections::BTreeSet::new();
            let (mut ticks, mut events, mut answers, mut exhausted_at) = (0, 0, 0, 0);
            let (mut first, mut first_end, mut peak_conditions, mut collection_work) = (0, 0, 0, 0);
            while !e.delivery_done() {
                ticks += 1;
                assert!(ticks < 10_000_000);
                collection_work += usize::from(e.collecting());
                e.advance(1);
                peak_conditions = peak_conditions.max(e.memory().conditions);
                if exhausted_at == 0 && e.exhausted() {
                    exhausted_at = ticks;
                }
                if let Some(event) = e.take_output() {
                    events += 1;
                    if first == 0 {
                        first = ticks;
                    }
                    if matches!(event, Output::End) && first_end == 0 {
                        first_end = ticks;
                    }
                    if let Output::Begin {
                        completion,
                        alternative,
                    } = event
                    {
                        assert!(ids.insert((completion, alternative)));
                        answers += 1;
                    }
                }
            }
            let count = if shape == "nested" { 2 * n } else { n };
            assert_eq!(answers, count);
            assert_eq!(events, count * if shape == "empty" { 2 } else { 6 });
            eprintln!(
                "delivery {shape} n={n} ticks={ticks} first={first} first_end={first_end} after_exhaustion={} events={events} collection_work={collection_work} peak_conditions={peak_conditions}",
                ticks - exhausted_at
            );

            if n == 64 {
                first_work = ticks;
            }
            if n == 256 {
                // Fourfold input would require sixteenfold work for a full scan
                // per answer. Span collection thresholds, rather than fitting
                // a single adjacent-size transition.
                assert!(ticks < first_work * 12, "{shape}: {first_work} -> {ticks}");
            }
        }
    }
}
