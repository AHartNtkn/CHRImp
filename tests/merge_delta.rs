use chr::{
    condition::{Arena, Condition, Operation, Progress},
    graph::{Graph, UpdateStatus},
    identity::{Merge, MergeDelta},
    matching::{MatchStatus, Matches},
    program::{Prepared, prepare},
    store::Root,
    syntax::{parse_program, parse_query},
    trace::{Cursor, Step, Trace},
    wake::{Wake, WakeStatus},
};
use std::{collections::BTreeMap, sync::Arc};
type Tuples = BTreeMap<Vec<u64>, Condition>;
fn boolop(a: &mut Arena, op: Operation) -> Condition {
    let mut j = a.start(op);
    for _ in 0..100000 {
        if let Progress::Complete(c) = j.tick(a) {
            return c;
        }
    }
    panic!("boolean timeout")
}
fn trace(j: &impl Trace) -> Vec<Condition> {
    let mut cursor = Cursor::default();
    let mut result = vec![];
    for _ in 0..100000 {
        match j.trace(&mut cursor) {
            Step::Root(c) => result.push(c),
            Step::Done => return result,
            Step::Pending => {}
        }
    }
    panic!("trace timeout")
}
fn gc(g: &mut Graph, a: &mut Arena, roots: Vec<Root>, mut conditions: Vec<Condition>) {
    let mut c = g.collect(roots.into_iter());
    while !c.done() {
        if let Some(x) = c.tick(g) {
            conditions.push(x)
        }
    }
    drop(c);
    let mut c = a.collect(conditions.into_iter());
    while !c.tick(a) {}
}
fn post(g: &mut Graph, root: Root, r: usize, args: Vec<u64>, scope: Condition) -> Root {
    let mut j = g.post(root, r, args, scope).unwrap();
    loop {
        if let UpdateStatus::Complete(root) = j.tick(g) {
            return root;
        }
    }
}
fn merge(
    g: &mut Graph,
    a: &mut Arena,
    root: Root,
    x: u64,
    y: u64,
    c: Condition,
    keep: &[Condition],
) -> (Root, MergeDelta) {
    let mut j = Merge::new(g, root, x, y, c);
    for _ in 0..100000 {
        let mut roots = trace(&j);
        roots.extend(keep);
        gc(g, a, j.roots().to_vec(), roots);
        if let Some(root) = j.tick(g, a) {
            let delta = j.take_delta().expect("changed merge");
            assert_eq!(delta.root(), root);
            assert!(j.take_delta().is_none(), "delta transferred twice");
            return (root, delta);
        }
    }
    panic!("merge timeout")
}
fn matches(
    g: &Graph,
    a: &mut Arena,
    root: Root,
    code: Arc<Prepared>,
    rule: usize,
    anchor: Option<(usize, u64)>,
    scope: Condition,
) -> Tuples {
    let mut j = Matches::new(g, root, code, rule, scope, anchor).unwrap();
    let mut out = Tuples::new();
    for _ in 0..1000000 {
        match j.tick(g, a) {
            MatchStatus::Found(m) => {
                let old = out.get(&m.occurrences).copied().unwrap_or(Condition::FALSE);
                out.insert(m.occurrences, boolop(a, Operation::Or(old, m.support)));
            }
            MatchStatus::Done => return out,
            MatchStatus::Pending => {}
        }
    }
    panic!("match timeout")
}
#[test]
fn opposite_conditional_losers_cover_every_new_multihead_match_on_frozen_root() {
    let code = Arc::new(
        prepare(
            &parse_program(
                "p(X,X),q(X),r(X) ==> hit(X). q(X),q(X) ==> pair(X). p(X,X),q(X) <=> done(X).",
            )
            .unwrap(),
            &parse_query("true").unwrap(),
        )
        .unwrap(),
    );
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let keep = [c, d];
    let mut root = g.empty();
    let relation = |name: &str| {
        code.signatures()
            .iter()
            .position(|s| s.name == name)
            .unwrap()
    };
    for args in [
        vec![0, 1],
        vec![2, 3],
        vec![4, 5],
        vec![6, 7],
        vec![0, 0],
        vec![1, 1],
    ] {
        root = post(&mut g, root, relation("p"), args, Condition::TRUE)
    }
    for v in 0..8 {
        root = post(
            &mut g,
            root,
            relation("q"),
            vec![v],
            if v % 2 == 0 { d } else { Condition::TRUE },
        );
        root = post(&mut g, root, relation("r"), vec![v], Condition::TRUE)
    }
    for (x, y) in [(0, 4), (1, 5), (2, 6), (3, 7)] {
        root = merge(&mut g, &mut a, root, x, y, Condition::TRUE, &keep).0;
    }
    root = merge(&mut g, &mut a, root, 0, 2, c, &keep).0;
    root = merge(&mut g, &mut a, root, 1, 3, c.not(), &keep).0;
    let before: Vec<_> = (0..code.rules().len())
        .map(|r| {
            matches(
                &g,
                &mut a,
                root.clone(),
                code.clone(),
                r,
                None,
                Condition::TRUE,
            )
        })
        .collect();
    // Keep pre-merge oracle conditions through GC inside Merge.
    let mut held = keep.to_vec();
    held.extend(before.iter().flat_map(|m| m.values().copied()));
    let (frozen, delta) = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE, &held);
    let scopes: Vec<_> = delta.condition_roots().collect();
    assert_eq!(scopes.len(), 2);
    assert!(scopes.contains(&c) && scopes.contains(&c.not()));
    assert_eq!(trace(&delta), scopes);
    // The detached token is the only graph owner across this handoff collection.
    let mut token_roots = held.clone();
    token_roots.extend(trace(&delta));
    gc(&mut g, &mut a, vec![delta.root()], token_roots);
    let after: Vec<_> = (0..code.rules().len())
        .map(|r| {
            matches(
                &g,
                &mut a,
                frozen.clone(),
                code.clone(),
                r,
                None,
                Condition::TRUE,
            )
        })
        .collect();
    held.extend(after.iter().flat_map(|m| m.values().copied()));
    let mut wake = Wake::from_delta(&g, delta);
    assert_eq!(wake.root(), frozen);
    // Mutate a newer root. The delta must continue reading only its frozen root.
    let newer = post(
        &mut g,
        frozen.clone(),
        relation("q"),
        vec![0],
        Condition::TRUE,
    );
    let mut found = BTreeMap::new();
    for tick in 0..100000 {
        let mut roots = held.clone();
        roots.extend(trace(&wake));
        roots.extend(found.values().copied());
        gc(&mut g, &mut a, vec![frozen.clone(), newer.clone()], roots);
        match wake.tick(&g, &mut a) {
            WakeStatus::Found {
                occurrence,
                support,
            } => {
                let old = found.get(&occurrence).copied().unwrap_or(Condition::FALSE);
                assert_eq!(
                    boolop(&mut a, Operation::And(old, support)),
                    Condition::FALSE
                );
                found.insert(occurrence, boolop(&mut a, Operation::Or(old, support)));
            }
            WakeStatus::Done => break,
            WakeStatus::Pending => {}
        }
        assert!(tick < 99999);
    }
    let mut novel_count = 0;
    for rule in 0..code.rules().len() {
        let mut covered = Tuples::new();
        for (&id, &scope) in &found {
            let fact = g.fact(frozen.clone(), id).unwrap();
            for (head, atom) in code.heads(&code.rules()[rule]).iter().enumerate() {
                if atom.relation == fact.relation {
                    for (tuple, c) in matches(
                        &g,
                        &mut a,
                        frozen.clone(),
                        code.clone(),
                        rule,
                        Some((head, id)),
                        scope,
                    ) {
                        let old = covered.get(&tuple).copied().unwrap_or(Condition::FALSE);
                        covered.insert(tuple, boolop(&mut a, Operation::Or(old, c)));
                    }
                }
            }
        }
        for (tuple, &scope) in &after[rule] {
            let old = before[rule].get(tuple).copied().unwrap_or(Condition::FALSE);
            let novel = boolop(&mut a, Operation::Difference(scope, old));
            if novel != Condition::FALSE {
                novel_count += 1;
            }
            let supplied = covered.get(tuple).copied().unwrap_or(Condition::FALSE);
            assert_eq!(
                boolop(&mut a, Operation::Difference(novel, supplied)),
                Condition::FALSE,
                "rule{rule} tuple{tuple:?}"
            );
        }
    }
    assert!(novel_count > 10);
    assert!(wake.condition_roots().all(Condition::is_terminal));
}
fn conditional_delta() -> (Graph, Arena, MergeDelta) {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let root = g.empty();
    let root = merge(&mut g, &mut a, root, 0, 2, c, &[c]).0;
    let root = merge(&mut g, &mut a, root, 1, 3, c.not(), &[c]).0;
    let (_, delta) = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE, &[c]);
    assert_eq!(delta.condition_roots().count(), 2);
    (g, a, delta)
}
#[test]
fn delta_discard_traces_remaining_supported_seeds_until_released() {
    let (mut g, mut a, delta) = conditional_delta();
    let root = delta.root();
    let mut wake = Wake::from_delta(&g, delta);
    for _ in 0..1000 {
        let roots = trace(&wake);
        gc(&mut g, &mut a, vec![root.clone()], roots);
        if wake.discard_tick() {
            assert!(wake.condition_roots().all(Condition::is_terminal));
            return;
        }
    }
    panic!("discard timeout")
}
#[test]
fn detached_delta_owns_graph_and_conditions_through_incremental_cancel() {
    let (mut g, mut a, mut delta) = conditional_delta();
    for remaining in [2, 1, 0] {
        let roots = trace(&delta);
        assert_eq!(roots, delta.condition_roots().collect::<Vec<_>>());
        assert_eq!(roots.len(), remaining);
        gc(&mut g, &mut a, vec![delta.root()], roots);
        assert_eq!(delta.discard_tick(), remaining == 0);
    }
    assert!(delta.discard_tick());
    assert!(trace(&delta).is_empty());
}
#[test]
#[should_panic(expected = "discarded merge delta cannot activate")]
fn partially_canceled_delta_cannot_be_used_to_activate() {
    let (g, _, mut delta) = conditional_delta();
    assert!(!delta.discard_tick());
    let _ = Wake::from_delta(&g, delta);
}
#[test]
fn unchanged_merge_has_no_activation_token() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let root = g.empty();
    let mut merge = Merge::new(&g, root.clone(), 0, 0, Condition::TRUE);
    assert_eq!(merge.tick(&mut g, &mut a), Some(root));
    assert!(merge.take_delta().is_none());
}
