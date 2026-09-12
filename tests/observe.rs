use chr::commit::StateRoot;
use chr::condition::{Arena, Condition, Operation, Progress};
use chr::engine::{Birth, Completion};
use chr::graph::{Graph, UpdateStatus};
use chr::identity::Merge;
use chr::observe::{Observe, ObserveStatus, Output};
use chr::program::{Prepared, prepare};
use chr::store::Root;
use chr::syntax::{parse_program, parse_query};
use std::collections::BTreeMap;
use std::sync::Arc;

fn code(query: &str) -> Arc<Prepared> {
    Arc::new(prepare(&parse_program("").unwrap(), &parse_query(query).unwrap()).unwrap())
}
fn completion(root: Root, support: Condition, last_choice: Option<u64>) -> Completion {
    Completion {
        id: 42,
        support,
        state: StateRoot {
            graph: root,
            history: Graph::new(&[]).empty(),
        },
        last_choice,
    }
}
fn birth(a: &mut Arena, births: &mut BTreeMap<u64, Birth>, support: Condition) -> (u64, Condition) {
    let (id, decision) = a.fresh_choice();
    births.insert(
        id,
        Birth {
            event: id,
            instruction: 0,
            start: 0,
            split: 1,
            end: 2,
            support,
            decision,
        },
    );
    (id, decision)
}
fn boolean(a: &mut Arena, op: Operation) -> Condition {
    let mut job = a.start(op);
    for _ in 0..100_000 {
        if let Progress::Complete(c) = job.tick(a) {
            return c;
        }
    }
    panic!("condition operation did not finish");
}
fn post(
    g: &mut Graph,
    root: Root,
    relation: usize,
    args: Vec<u64>,
    support: Condition,
) -> (Root, u64) {
    let mut job = g.post(root, relation, args, support).unwrap();
    let occurrence = job.occurrence();
    loop {
        if let UpdateStatus::Complete(root) = job.tick(g) {
            return (root, occurrence);
        }
    }
}
fn run(
    g: &mut Graph,
    a: &mut Arena,
    births: &BTreeMap<u64, Birth>,
    observer: &mut Observe,
) -> Vec<Output> {
    let mut events = Vec::new();
    for _ in 0..100_000 {
        let mut gc = g.collect(std::iter::once(observer.graph_root()));
        let mut supports = Vec::new();
        while !gc.done() {
            if let Some(c) = gc.tick(g) {
                supports.push(c);
            }
        }
        drop(gc);
        let mut gc = a.collect(
            traced_roots(observer, observer.condition_roots())
                .into_iter()
                .chain(supports)
                .chain(births.values().flat_map(|b| [b.support, b.decision])),
        );
        while !gc.tick(a) {}
        drop(gc);
        match observer.tick(g, a, births) {
            ObserveStatus::Event(output) => events.push(output),
            ObserveStatus::Pending => {}
            ObserveStatus::Done => {
                assert_eq!(observer.tick(g, a, births), ObserveStatus::Done);
                assert!(observer.condition_roots().all(|c| c.is_terminal()));
                return events;
            }
        }
    }
    panic!("finite observation did not finish");
}
fn empty_alternatives(events: &[Output], count: u64) {
    let expected: Vec<_> = (0..count)
        .flat_map(|alternative| {
            [
                Output::Begin {
                    completion: 42,
                    alternative,
                },
                Output::End,
            ]
        })
        .collect();
    assert_eq!(events, expected);
}

#[test]
fn empty_graph_preserves_active_duplicate_choices_and_empty_scope() {
    let code = code("true");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let mut births = BTreeMap::new();
    for count in 0..=2 {
        let last = if count == 0 {
            None
        } else {
            Some(birth(&mut a, &mut births, Condition::TRUE).0)
        };
        let mut observer = Observe::new(
            completion(g.empty(), Condition::TRUE, last),
            code.clone(),
            Arc::new(vec![]),
        );
        assert_eq!(observer.completion_id(), 42);
        empty_alternatives(&run(&mut g, &mut a, &births, &mut observer), 1 << count);
    }
    let mut observer = Observe::new(
        completion(g.empty(), Condition::FALSE, Some(1)),
        code,
        Arc::new(vec![]),
    );
    assert!(run(&mut g, &mut a, &births, &mut observer).is_empty());
}

#[test]
fn inactive_nested_birth_does_not_multiply_answers() {
    let code = code("true");
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let mut births = BTreeMap::new();
    let (_, c) = birth(&mut a, &mut births, Condition::TRUE);
    let (last, _) = birth(&mut a, &mut births, c);
    let mut observer = Observe::new(
        completion(g.empty(), Condition::TRUE, Some(last)),
        code,
        Arc::new(vec![]),
    );
    empty_alternatives(&run(&mut g, &mut a, &births, &mut observer), 3);
}

#[test]
fn correlated_births_prune_by_completion_support_and_ignore_future_births() {
    let code = code("true");
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let mut births = BTreeMap::new();
    let (_, c) = birth(&mut a, &mut births, Condition::TRUE);
    let (_, d) = birth(&mut a, &mut births, c);
    let (last, e) = birth(&mut a, &mut births, c.not());
    birth(&mut a, &mut births, Condition::TRUE);
    let cd = boolean(&mut a, Operation::And(c, d));
    let ce = boolean(&mut a, Operation::And(c.not(), e));
    let support = boolean(&mut a, Operation::Or(cd, ce));
    let mut observer = Observe::new(
        completion(g.empty(), support, Some(last)),
        code,
        Arc::new(vec![]),
    );
    empty_alternatives(&run(&mut g, &mut a, &births, &mut observer), 2);
}

#[test]
fn conditional_query_variables_and_ordered_ports_are_resolved_in_each_history() {
    let code = code("p(X,Y), q(X), zero()");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let mut births = BTreeMap::new();
    let (last, c) = birth(&mut a, &mut births, Condition::TRUE);
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![0, 1], Condition::TRUE);
    let (root, duplicate) = post(&mut g, root, 0, vec![0, 1], Condition::TRUE);
    let (root, q) = post(&mut g, root, 1, vec![1], c);
    let (root, zero) = post(&mut g, root, 2, vec![], Condition::TRUE);
    let mut merge = Merge::new(&g, root, 0, 1, c);
    let root = loop {
        if let Some(root) = merge.tick(&mut g, &mut a) {
            break root;
        }
    };
    let mut observer = Observe::new(
        completion(root.clone(), Condition::TRUE, Some(last)),
        code,
        Arc::new(vec![0, 1]),
    );
    assert_eq!(observer.graph_root(), root);
    let (_later, _) = post(&mut g, root, 1, vec![999], Condition::TRUE);
    let events = run(&mut g, &mut a, &births, &mut observer);
    let mut alternatives = Vec::new();
    let mut current = Vec::new();
    for output in events {
        match output {
            Output::Begin {
                completion,
                alternative,
            } => {
                assert_eq!(completion, 42);
                assert_eq!(alternative as usize, alternatives.len());
            }
            Output::End => alternatives.push(std::mem::take(&mut current)),
            event => current.push(event),
        }
    }
    let expected = |merged: bool| {
        let y = if merged { 0 } else { 1 };
        let mut events = vec![
            Output::Variable {
                slot: 0,
                variable: 0,
            },
            Output::Variable {
                slot: 1,
                variable: y,
            },
        ];
        for occurrence in [p, duplicate] {
            events.extend([
                Output::Fact {
                    occurrence,
                    relation: 0,
                },
                Output::Port { variable: 0 },
                Output::Port { variable: y },
                Output::EndFact,
            ]);
        }
        if merged {
            events.extend([
                Output::Fact {
                    occurrence: q,
                    relation: 1,
                },
                Output::Port { variable: 0 },
                Output::EndFact,
            ]);
        }
        events.extend([
            Output::Fact {
                occurrence: zero,
                relation: 2,
            },
            Output::EndFact,
        ]);
        events
    };
    assert_eq!(alternatives.len(), 2);
    assert!(alternatives.contains(&expected(true)) && alternatives.contains(&expected(false)));
    assert_eq!(g.occurrence_count(), 4);
    assert_eq!(
        a.fresh_choice().0,
        1,
        "observation cannot introduce choices"
    );
}

#[test]
fn first_output_does_not_materialize_independent_choice_product() {
    let code = code("true");
    let g = Graph::new(&[]);
    let mut a = Arena::default();
    let mut births = BTreeMap::new();
    let mut last = None;
    for _ in 0..20 {
        last = Some(birth(&mut a, &mut births, Condition::TRUE).0);
    }
    let mut observer = Observe::new(
        completion(g.empty(), Condition::TRUE, last),
        code,
        Arc::new(vec![]),
    );
    for _ in 0..20_000 {
        match observer.tick(&g, &mut a, &births) {
            ObserveStatus::Event(event) => {
                assert_eq!(
                    event,
                    Output::Begin {
                        completion: 42,
                        alternative: 0
                    }
                );
                return;
            }
            ObserveStatus::Pending => {}
            ObserveStatus::Done => panic!("missing alternatives"),
        }
    }
    panic!("first answer waits for a choice product");
}

#[test]
fn output_json_has_scalar_tagged_events() {
    use serde_json::json;
    for (event, expected) in [
        (
            Output::Begin {
                completion: 7,
                alternative: 2,
            },
            json!({"kind":"begin","completion":7,"alternative":2}),
        ),
        (
            Output::Variable {
                slot: 3,
                variable: 4,
            },
            json!({"kind":"variable","slot":3,"variable":4}),
        ),
        (
            Output::Fact {
                occurrence: 8,
                relation: 9,
            },
            json!({"kind":"fact","occurrence":8,"relation":9}),
        ),
        (
            Output::Port { variable: 10 },
            json!({"kind":"port","variable":10}),
        ),
        (Output::EndFact, json!({"kind":"end_fact"})),
        (Output::End, json!({"kind":"end"})),
    ] {
        assert_eq!(serde_json::to_value(event).unwrap(), expected);
    }
}

#[test]
#[should_panic(expected = "multiple representatives")]
fn incomplete_history_cannot_silently_select_one_representative() {
    let code = code("true");
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let mut merge = Merge::new(&g, g.empty(), 0, 1, c);
    let root = loop {
        if let Some(root) = merge.tick(&mut g, &mut a) {
            break root;
        }
    };
    // An inconsistent caller omits the birth that distinguishes representatives.
    let mut observer = Observe::new(
        completion(root, Condition::TRUE, None),
        code,
        Arc::new(vec![1]),
    );
    run(&mut g, &mut a, &BTreeMap::new(), &mut observer);
}

fn traced_roots(
    job: &impl chr::trace::Trace,
    expected: impl Iterator<Item = Condition>,
) -> Vec<Condition> {
    use chr::trace::{Cursor, Step};
    let mut expected: Vec<_> = expected.collect();
    let mut cursor = Cursor::default();
    let mut roots = Vec::new();
    for _ in 0..16 * expected.len() + 256 {
        match job.trace(&mut cursor) {
            Step::Root(root) => roots.push(root),
            Step::Pending => {}
            Step::Done => {
                assert_eq!(job.trace(&mut cursor), Step::Done);
                roots.sort_unstable();
                expected.sort_unstable();
                assert_eq!(roots, expected, "incremental trace changed root inventory");
                return roots;
            }
        }
    }
    panic!("root walk exceeded its linear step budget");
}

#[test]
fn discard_cancels_nested_projection_without_enumerating_remaining_alternatives() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let prepared = code("p(X)");
    let mut reached_fact = false;
    let mut reached_variable = false;
    for (prefix, after_fact) in [
        0, 1, 2, 3, 7, 35, 110, 175, 210, 240, 280, 330, 390, 450, 550,
    ]
    .into_iter()
    .map(|n| (n, false))
    .chain([1, 3, 8].into_iter().map(|n| (n, true)))
    {
        let mut g = Graph::new(prepared.signatures());
        let mut a = Arena::default();
        let mut births = BTreeMap::new();
        let mut last = None;
        for _ in 0..20 {
            last = Some(birth(&mut a, &mut births, Condition::TRUE).0);
        }
        let empty = g.empty();
        let mut merging = Merge::new(&g, empty, 0, 1, Condition::TRUE);
        let root = loop {
            if let Some(root) = merging.tick(&mut g, &mut a) {
                break root;
            }
        };
        let (root, occurrence) = post(&mut g, root, 0, vec![1], Condition::TRUE);
        let variables = Arc::new(vec![1]);
        let mut observer = Observe::new(
            completion(root.clone(), Condition::TRUE, last),
            prepared.clone(),
            variables.clone(),
        );
        let mut waiting_for_fact = after_fact;
        let mut remaining = prefix;
        for _ in 0..10_000 {
            if !waiting_for_fact && remaining == 0 {
                break;
            }
            let status = observer.tick(&g, &mut a, &births);
            let saw_fact = matches!(&status, ObserveStatus::Event(Output::Fact { .. }));
            match status {
                ObserveStatus::Event(Output::Fact { .. }) => reached_fact = true,
                ObserveStatus::Event(Output::Variable { .. }) => reached_variable = true,
                ObserveStatus::Done => {
                    panic!("a million alternatives cannot finish in this prefix")
                }
                _ => {}
            }
            if waiting_for_fact {
                if saw_fact {
                    waiting_for_fact = false;
                }
            } else {
                remaining -= 1;
            }
        }
        assert!(
            !waiting_for_fact && remaining == 0,
            "projection must reach the requested cancellation boundary"
        );
        // Cancellation no longer needs future causal births; only the graph
        // and remaining owned continuation roots are traced from here onward.
        drop(births);
        drop(merging);
        let mut calls = 0;
        loop {
            let mut supports = traced_roots(&observer, observer.condition_roots());
            let mut gc = g.collect([observer.graph_root()].into_iter());
            while !gc.done() {
                if let Some(c) = gc.tick(&mut g) {
                    supports.push(c);
                }
            }
            drop(gc);
            let mut gc = a.collect(supports.into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            let done = observer.discard_tick();
            calls += 1;
            assert_eq!(observer.graph_root(), root);
            assert!(g.fact(root.clone(), occurrence).is_some());
            assert!(
                catch_unwind(AssertUnwindSafe(|| observer.tick(
                    &g,
                    &mut a,
                    &BTreeMap::new()
                )))
                .is_err()
            );
            if done {
                break;
            }
            assert!(
                calls < 32,
                "cancel must discard scratch rather than enumerate 2^20 alternatives"
            );
        }
        assert_eq!(
            Arc::strong_count(&variables),
            1,
            "discard retains variable-array ownership"
        );
        assert!(observer.condition_roots().all(|c| c.is_terminal()));
        assert!(observer.discard_tick());
        assert!(observer.discard_tick());
        let roots = traced_roots(&observer, observer.condition_roots());
        let mut gc = a.collect(roots.into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        assert_eq!(
            a.node_count(),
            0,
            "no remaining choice support is needed after discard"
        );
        assert_eq!(observer.graph_root(), root);
        drop(observer);
        let mut gc = g.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut g);
        }
        drop(gc);
        assert_eq!(g.occurrence_count(), 0);
    }
    assert!(
        reached_fact && reached_variable,
        "prefixes must exercise actual nested projection"
    );
}
