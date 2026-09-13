#![cfg(feature = "diagnostics")]

use chr::{
    condition::{Arena, Condition},
    graph::{Graph, UpdateStatus},
    identity::Merge,
    matching::{MatchStatus, Matches},
    program::{Prepared, prepare},
    store::Root,
    syntax::{parse_program, parse_query},
};
use std::sync::Arc;

fn code() -> Arc<Prepared> {
    Arc::new(
        prepare(
            &parse_program("p(X,Y,I),r(X,Y,Z) ==> hit(I,Z).").unwrap(),
            &parse_query("true").unwrap(),
        )
        .unwrap(),
    )
}
fn post(g: &mut Graph, root: Root, relation: usize, args: Vec<u64>) -> (Root, u64) {
    let mut update = g.post(root, relation, args, Condition::TRUE).unwrap();
    let id = update.occurrence();
    loop {
        if let UpdateStatus::Complete(root) = update.tick(g) {
            return (root, id);
        }
    }
}
fn drain(m: &mut Matches, g: &Graph, a: &mut Arena) -> Vec<chr::matching::Match> {
    let mut found = vec![];
    for _ in 0..100_000 {
        match m.tick(g, a) {
            MatchStatus::Found(m) => found.push(m),
            MatchStatus::Done => return found,
            MatchStatus::Pending => {}
        }
    }
    panic!("restriction consumer did not finish");
}
fn fixture(n: u64) -> (Arc<Prepared>, Graph, Root) {
    let c = code();
    let mut g = Graph::new(c.signatures());
    let mut root = g.empty();
    for i in 0..n {
        root = post(&mut g, root, 1, vec![1, 100 + i, 1000 + i]).0;
        root = post(&mut g, root, 1, vec![200 + i, 2, 2000 + i]).0;
    }
    (c, g, root)
}
#[test]
fn unrelated_posts_share_the_correlated_scan_across_graph_versions() {
    let (c, mut g, mut root) = fixture(64);
    let mut a = Arena::default();
    let mut visits = 0;
    for i in 0..8 {
        let (next, p) = post(&mut g, root, 0, vec![1, 2, 3000 + i]);
        root = next;
        let mut m = Matches::new(
            &g,
            root.clone(),
            c.clone(),
            0,
            Condition::TRUE,
            Some((0, p)),
        )
        .unwrap();
        assert!(drain(&mut m, &g, &mut a).is_empty());
        visits += m.candidate_visits();
    }
    let work = g.restriction_diagnostics();
    assert_eq!(work.created, 1);
    assert_eq!(work.reused, 7);
    assert_eq!(work.producer_candidates, 64, "the initial scan is charged");
    assert_eq!(work.retained_rows, 0);
    assert!(
        visits <= 64 + 8,
        "independent anchors repeated the restriction: {visits}"
    );
}

#[test]
fn relevant_insertions_and_liveness_changes_preserve_old_snapshots() {
    let (c, mut g, root) = fixture(32);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let mut a = Arena::default();
    let mut first = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    assert!(drain(&mut first, &g, &mut a).is_empty());
    let (new, r1) = post(&mut g, root.clone(), 1, vec![1, 2, 9001]);
    let (new, r2) = post(&mut g, new, 1, vec![1, 2, 9001]);
    let mut stale = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    let mut fresh =
        Matches::new(&g, new.clone(), c.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
    assert!(drain(&mut stale, &g, &mut a).is_empty());
    let matches = drain(&mut fresh, &g, &mut a);
    assert_eq!(
        matches.iter().map(|m| m.occurrences[1]).collect::<Vec<_>>(),
        [r1, r2]
    );
    let (_, choice) = a.fresh_choice();
    let mut update = g.set_liveness(new.clone(), r1, choice).unwrap();
    let changed = loop {
        if let UpdateStatus::Complete(r) = update.tick(&mut g) {
            break r;
        }
    };
    drop(update);
    let mut old = Matches::new(&g, new, c.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
    assert!(
        drain(&mut old, &g, &mut a)
            .iter()
            .all(|m| m.support == Condition::TRUE)
    );
    let mut changed = Matches::new(&g, changed, c, 0, Condition::TRUE, Some((0, p))).unwrap();
    let matches = drain(&mut changed, &g, &mut a);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].support, choice);
    assert_eq!(matches[1].support, Condition::TRUE);
}

#[test]
fn late_conditional_alias_uses_general_matching_without_poisoning_old_restriction() {
    let (c, mut g, root) = fixture(16);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let mut a = Arena::default();
    let mut first = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    assert!(drain(&mut first, &g, &mut a).is_empty());
    let (_, choice) = a.fresh_choice();
    let mut merge = Merge::new(&g, root.clone(), 2, 100, choice);
    let merged = loop {
        if let Some(r) = merge.tick(&mut g, &mut a) {
            break r;
        }
    };
    drop(merge);
    let mut changed =
        Matches::new(&g, merged, c.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
    let matches = drain(&mut changed, &g, &mut a);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].support, choice);
    assert_eq!(g.restriction_diagnostics().created, 1);
    let mut old = Matches::new(&g, root, c, 0, Condition::TRUE, Some((0, p))).unwrap();
    assert!(drain(&mut old, &g, &mut a).is_empty());
    assert_eq!(g.restriction_diagnostics().producer_candidates, 16);
}

#[test]
fn paused_subscriber_survives_other_progress_cancellation_and_collection() {
    let (c, mut g, root) = fixture(64);
    let (root, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let (root, q) = post(&mut g, root, 0, vec![1, 2, 9002]);
    let mut a = Arena::default();
    let (_, choice) = a.fresh_choice();
    let mut slow = Matches::new(&g, root.clone(), c.clone(), 0, choice, Some((0, p))).unwrap();
    while g.restriction_diagnostics().producer_candidates < 3 {
        assert!(matches!(slow.tick(&g, &mut a), MatchStatus::Pending));
    }
    let mut fast =
        Matches::new(&g, root.clone(), c.clone(), 0, choice.not(), Some((0, q))).unwrap();
    let result = drain(&mut fast, &g, &mut a);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].occurrences, [q, hit]);
    assert_eq!(result[0].support, choice.not());
    drop(fast);
    let mut collector = g.collect(vec![root.clone(), slow.root()].into_iter());
    while !collector.done() {
        collector.tick(&mut g);
    }
    drop(collector);
    let result = drain(&mut slow, &g, &mut a);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].occurrences, [p, hit]);
    assert_eq!(result[0].support, choice);
    assert_eq!(g.restriction_diagnostics().producer_candidates, 65);
    drop(slow);
    drop(root);
    let mut collector = g.collect(std::iter::empty());
    while !collector.done() {
        collector.tick(&mut g);
    }
    drop(collector);
    while !g.release_tick() {}
    assert_eq!(g.restriction_diagnostics().retained_rows, 0);
    assert_eq!(g.restriction_diagnostics().entries, 0);
    assert_eq!(g.occurrence_count(), 0);
    assert_eq!(g.index_node_count(), 0);
}

#[test]
fn abandoned_partial_producer_is_never_a_completed_negative_answer() {
    let (c, mut g, root) = fixture(64);
    let (root, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let mut a = Arena::default();
    let mut canceled = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    while g.restriction_diagnostics().producer_candidates < 3 {
        assert!(matches!(canceled.tick(&g, &mut a), MatchStatus::Pending));
    }
    while !canceled.discard_tick() {}
    drop(canceled);
    let mut next = Matches::new(&g, root, c, 0, Condition::TRUE, Some((0, p))).unwrap();
    let result = drain(&mut next, &g, &mut a);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].occurrences, [p, hit]);
    assert_eq!(g.restriction_diagnostics().created, 2);
    assert_eq!(g.restriction_diagnostics().producer_candidates, 68);
}

#[test]
fn unfinished_producer_is_protected_by_surviving_subscribers_different_snapshot() {
    let (c, mut g, root) = fixture(64);
    let (root, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let mut a = Arena::default();
    let mut first = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    while g.restriction_diagnostics().producer_candidates < 3 {
        assert!(matches!(first.tick(&g, &mut a), MatchStatus::Pending));
    }
    let (root, q) = post(&mut g, root, 0, vec![1, 2, 9002]);
    let mut survivor = Matches::new(&g, root, c, 0, Condition::TRUE, Some((0, q))).unwrap();
    while g.restriction_diagnostics().reused == 0 {
        assert!(matches!(survivor.tick(&g, &mut a), MatchStatus::Pending));
    }
    while !first.discard_tick() {}
    drop(first);
    let mut conditions: Vec<_> = survivor.condition_roots().collect();
    let mut collector = g.collect(vec![survivor.root()].into_iter());
    while !collector.done() {
        if let Some(c) = collector.tick(&mut g) {
            conditions.push(c);
        }
    }
    drop(collector);
    let mut collector = a.collect(conditions.into_iter());
    while !collector.tick(&mut a) {}
    drop(collector);
    assert_eq!(g.restriction_diagnostics().entries, 0);
    let result = drain(&mut survivor, &g, &mut a);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].occurrences, [q, hit]);
    assert_eq!(g.restriction_diagnostics().created, 1);
    assert_eq!(g.restriction_diagnostics().producer_candidates, 65);
}

#[test]
fn empty_bucket_dependency_changes_when_its_first_row_arrives() {
    let c = code();
    let mut g = Graph::new(c.signatures());
    let empty = g.empty();
    let (root, _) = post(&mut g, empty, 1, vec![10, 20, 30]);
    let (root, _) = post(&mut g, root, 1, vec![11, 21, 31]);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 3]);
    let mut a = Arena::default();
    let mut old = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    assert!(drain(&mut old, &g, &mut a).is_empty());
    assert_eq!(g.restriction_diagnostics().producer_candidates, 0);
    let (next, hit) = post(&mut g, root.clone(), 1, vec![1, 2, 99]);
    let mut new = Matches::new(&g, next, c.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
    assert_eq!(drain(&mut new, &g, &mut a)[0].occurrences, [p, hit]);
    let mut old = Matches::new(&g, root, c, 0, Condition::TRUE, Some((0, p))).unwrap();
    assert!(drain(&mut old, &g, &mut a).is_empty());
    assert_eq!(g.restriction_diagnostics().created, 2);
    assert_eq!(g.restriction_diagnostics().reused, 1);
}

#[test]
fn registry_eviction_keeps_live_consumers_valid_and_bounds_retention() {
    let c = code();
    let mut g = Graph::new(c.signatures());
    let mut root = g.empty();
    let mut hits = vec![];
    for i in 0..40 {
        let (next, id) = post(&mut g, root, 1, vec![1, 100 + i, 200 + i]);
        root = next;
        hits.push(id);
    }
    let mut a = Arena::default();
    let mut consumers = vec![];
    for i in 0..40 {
        let (next, p) = post(&mut g, root, 0, vec![1, 100 + i, 300 + i]);
        root = next;
        let mut m = Matches::new(
            &g,
            root.clone(),
            c.clone(),
            0,
            Condition::TRUE,
            Some((0, p)),
        )
        .unwrap();
        for _ in 0..1000 {
            if g.restriction_diagnostics().created == i + 1 {
                break;
            }
            assert!(matches!(m.tick(&g, &mut a), MatchStatus::Pending));
        }
        assert_eq!(g.restriction_diagnostics().created, i + 1);
        assert!(g.restriction_diagnostics().entries <= 32);
        consumers.push((p, m));
    }
    for ((p, mut m), hit) in consumers.into_iter().zip(hits) {
        let result = drain(&mut m, &g, &mut a);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].occurrences, [p, hit]);
    }
    let work = g.restriction_diagnostics();
    assert_eq!(work.producer_candidates, 40);
    assert_eq!(work.entries, 32);
    assert_eq!(work.retained_rows, 32);
}

#[test]
fn wider_and_larger_restrictions_continue_through_the_general_matcher() {
    for (arity, n) in [(9, 4), (3, 4097)] {
        let arguments = (0..arity)
            .map(|i| format!("V{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("p(V0,V1,I),r({arguments}) ==> hit(I).");
        let c = Arc::new(
            prepare(
                &parse_program(&source).unwrap(),
                &parse_query("true").unwrap(),
            )
            .unwrap(),
        );
        let mut g = Graph::new(c.signatures());
        let mut root = g.empty();
        for i in 0..n {
            let mut left = vec![1, 100 + i];
            left.extend((2..arity).map(|p| 100_000 + p as u64));
            root = post(&mut g, root, 1, left).0;
            let mut right = vec![10_000 + i, 2];
            right.extend((2..arity).map(|p| 200_000 + p as u64));
            root = post(&mut g, root, 1, right).0;
        }
        let mut args = vec![1, 2];
        args.extend((2..arity).map(|p| 300_000 + p as u64));
        let (root, hit) = post(&mut g, root, 1, args);
        let (root, p) = post(&mut g, root, 0, vec![1, 2, 9]);
        let mut m = Matches::new(&g, root, c, 0, Condition::TRUE, Some((0, p))).unwrap();
        let result = drain(&mut m, &g, &mut Arena::default());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].occurrences, [p, hit]);
        assert_eq!(g.restriction_diagnostics().created, 0);
        assert_eq!(g.restriction_diagnostics().producer_candidates, 0);
    }
}
