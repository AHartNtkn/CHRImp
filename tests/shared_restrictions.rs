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

#[allow(dead_code)]
#[path = "../examples/measure/allocation.rs"]
mod allocation;

fn allocation_checkpoint(n: u64, phase: &str) {
    let checkpoint = allocation::snapshot();
    println!(
        "prefix_allocation n={n} phase={phase} data={}",
        serde_json::to_string(&checkpoint).unwrap()
    );
}

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
    assert_eq!(work.retained_rows, 64);
    assert!(
        visits <= 64 + 8,
        "independent anchors repeated the restriction: {visits}"
    );
}

#[test]
fn growing_prefix_versions_preserve_order_cutoffs_and_release() {
    for n in [128, 512, 2048] {
        allocation_checkpoint(n, "start");
        let (c, mut g, root) = fixture(n);
        let (mut root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
        let mut a = Arena::default();
        let mut hits = Vec::new();
        let mut snapshots = Vec::new();
        allocation_checkpoint(n, "prepared");
        for version in 0..9 {
            if version != 0 {
                let (next, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
                root = next;
                hits.push(hit);
            }
            let mut m = Matches::new(
                &g,
                root.clone(),
                c.clone(),
                0,
                Condition::TRUE,
                Some((0, p)),
            )
            .unwrap();
            let result = drain(&mut m, &g, &mut a);
            assert_eq!(
                result.iter().map(|m| m.occurrences[1]).collect::<Vec<_>>(),
                hits
            );
            assert!(result.iter().all(|m| m.support == Condition::TRUE));
            snapshots.push((root.clone(), hits.clone()));
        }
        allocation_checkpoint(n, "updated");
        println!(
            "prefix_growth n={n} phase=updated diagnostics={} graph_nodes={} graph_allocations={}",
            serde_json::to_string(&g.restriction_diagnostics()).unwrap(),
            g.index_node_count(),
            g.index_allocations()
        );
        for (snapshot, expected) in snapshots {
            let mut m =
                Matches::new(&g, snapshot, c.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
            assert_eq!(
                drain(&mut m, &g, &mut a)
                    .iter()
                    .map(|m| m.occurrences[1])
                    .collect::<Vec<_>>(),
                expected
            );
        }
        println!(
            "prefix_growth n={n} phase=readback diagnostics={}",
            serde_json::to_string(&g.restriction_diagnostics()).unwrap()
        );
        allocation_checkpoint(n, "readback");
        println!("prefix_lookup n={n} counts={:?}", g.index_prefix_counts());
        drop(root);
        let mut collector = g.collect(std::iter::empty());
        let mut cleanup_ticks = 0;
        while !collector.done() {
            collector.tick(&mut g);
            cleanup_ticks += 1;
            assert!(cleanup_ticks < 1_000_000);
        }
        drop(collector);
        while !g.release_tick() {
            cleanup_ticks += 1;
            assert!(cleanup_ticks < 1_000_000);
        }
        assert_eq!(g.restriction_diagnostics().retained_rows, 0);
        assert_eq!(g.restriction_diagnostics().entries, 0);
        assert_eq!(g.occurrence_count(), 0);
        assert_eq!(g.index_node_count(), 0);
        println!("prefix_growth n={n} phase=released cleanup_ticks={cleanup_ticks}");
        allocation_checkpoint(n, "released");
        drop((g, a, c));
        allocation_checkpoint(n, "dropped");
    }
}

#[test]
fn large_prefix_readers_survive_interrupted_service_and_collection() {
    for interruption in [0, 1, 2, 3, 8, 32, 128, 256, 512, 1024] {
        let (c, mut g, root) = fixture(256);
        let (root, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
        let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
        let mut a = Arena::default();
        let mut slow = Matches::new(
            &g,
            root.clone(),
            c.clone(),
            0,
            Condition::TRUE,
            Some((0, p)),
        )
        .unwrap();
        let mut got = Vec::new();
        for _ in 0..interruption {
            if let MatchStatus::Found(m) = slow.tick(&g, &mut a) {
                got.push(m.occurrences[1]);
            }
        }
        let (next, extra) = post(&mut g, root.clone(), 1, vec![1, 2, 9001]);
        let mut fast = Matches::new(&g, next.clone(), c, 0, Condition::TRUE, Some((0, p))).unwrap();
        assert_eq!(
            drain(&mut fast, &g, &mut a)
                .iter()
                .map(|m| m.occurrences[1])
                .collect::<Vec<_>>(),
            [hit, extra]
        );
        while !fast.discard_tick() {}
        drop(fast);
        drop(next);
        let mut collector = g.collect(vec![root.clone(), slow.root()].into_iter());
        while !collector.done() {
            collector.tick(&mut g);
        }
        drop(collector);
        got.extend(
            drain(&mut slow, &g, &mut a)
                .iter()
                .map(|m| m.occurrences[1]),
        );
        assert_eq!(got, [hit]);
        while !slow.discard_tick() {}
        drop(slow);
        drop(root);
        let mut collector = g.collect(std::iter::empty());
        while !collector.done() {
            collector.tick(&mut g);
        }
        drop(collector);
        while !g.release_tick() {}
        assert_eq!(g.restriction_diagnostics().retained_rows, 0);
        assert_eq!(g.occurrence_count(), 0);
        assert_eq!(g.index_node_count(), 0);
    }
}

#[test]
fn large_prefix_lagging_reader_keeps_shared_production_after_cache_clear() {
    let (c, mut g, root) = fixture(512);
    let (root, hit) = post(&mut g, root, 1, vec![1, 2, 9001]);
    let (root, p) = post(&mut g, root, 0, vec![1, 2, 9000]);
    let mut a = Arena::default();
    let mut slow = Matches::new(
        &g,
        root.clone(),
        c.clone(),
        0,
        Condition::TRUE,
        Some((0, p)),
    )
    .unwrap();
    while g.restriction_diagnostics().producer_candidates < 3 {
        assert!(matches!(slow.tick(&g, &mut a), MatchStatus::Pending));
    }
    let mut fast = Matches::new(&g, root.clone(), c, 0, Condition::TRUE, Some((0, p))).unwrap();
    assert_eq!(drain(&mut fast, &g, &mut a)[0].occurrences, [p, hit]);
    drop(fast);
    let produced = g.restriction_diagnostics().producer_candidates;
    assert_eq!(produced, 513);
    let mut collector = g.collect(vec![root.clone(), slow.root()].into_iter());
    while !collector.done() {
        collector.tick(&mut g);
    }
    drop(collector);
    assert_eq!(drain(&mut slow, &g, &mut a)[0].occurrences, [p, hit]);
    assert_eq!(
        g.restriction_diagnostics().producer_candidates,
        produced,
        "a lagging alternative must reuse earlier shared computation after eviction"
    );
    drop(slow);
    drop(root);
    let mut collector = g.collect(std::iter::empty());
    while !collector.done() {
        collector.tick(&mut g);
    }
    drop(collector);
    while !g.release_tick() {}
    assert_eq!(g.restriction_diagnostics().retained_rows, 0);
    assert_eq!(g.occurrence_count(), 0);
    assert_eq!(g.index_node_count(), 0);
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

#[test]
fn distinct_binding_values_share_one_projection_per_bucket() {
    let n = 8;
    let c = code();
    let mut g = Graph::new(c.signatures());
    let mut root = g.empty();
    for i in 0..n {
        for j in 0..n {
            root = post(&mut g, root, 1, vec![100 + i, 200 + j, 1000 + i * n + j]).0;
            root = post(&mut g, root, 1, vec![300 + i, 400 + j, 2000 + i * n + j]).0;
        }
    }
    let mut a = Arena::default();
    for i in 0..n {
        for j in 0..n {
            let (next, p) = post(&mut g, root, 0, vec![100 + i, 400 + j, 3000 + i * n + j]);
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
        }
    }
    let work = g.restriction_diagnostics();
    assert_eq!(
        work.producer_candidates,
        n * n,
        "one input pass per queried bucket, not one per binding tuple"
    );
    assert_eq!(work.created, n);
    assert_eq!(work.reused, n * n - n);
    assert_eq!(
        work.retained_rows,
        (n * n) as usize,
        "every partitioned input row is charged even for empty queries"
    );
}

#[test]
fn different_productive_values_share_partitions_with_independent_positions_and_support() {
    let n = 4;
    let c = code();
    let mut g = Graph::new(c.signatures());
    let mut root = g.empty();
    let mut a = Arena::default();
    let (_, choice) = a.fresh_choice();
    let mut expected = vec![vec![]; (n * n) as usize];
    // Equal port cardinalities choose X. Interleave different partition values
    // between duplicates so a paused cursor must follow its own appended links.
    for copy in 0..2 {
        for i in 0..n {
            for j in 0..n {
                let mut update = g
                    .post(
                        root,
                        1,
                        vec![100 + i, 200 + j, 1000 + i * n + j],
                        if copy == 0 { Condition::TRUE } else { choice },
                    )
                    .unwrap();
                expected[(i * n + j) as usize].push(update.occurrence());
                root = loop {
                    if let UpdateStatus::Complete(r) = update.tick(&mut g) {
                        break r;
                    }
                };
            }
        }
    }
    for i in 0..n {
        let (next, p) = post(&mut g, root, 0, vec![100 + i, 200, 2000 + i * n]);
        root = next;
        let mut paused = Matches::new(
            &g,
            root.clone(),
            c.clone(),
            0,
            Condition::TRUE,
            Some((0, p)),
        )
        .unwrap();
        let mut first = None;
        for _ in 0..10000 {
            match paused.tick(&g, &mut a) {
                MatchStatus::Found(m) => {
                    first = Some(m);
                    break;
                }
                MatchStatus::Pending => {}
                MatchStatus::Done => panic!("first productive partition is missing"),
            }
        }
        let first = first.expect("first productive partition must progress");
        assert_eq!(first.occurrences, [p, expected[(i * n) as usize][0]]);
        assert_eq!(first.support, Condition::TRUE);
        for j in 1..n {
            let (next, q) = post(&mut g, root, 0, vec![100 + i, 200 + j, 2000 + i * n + j]);
            root = next;
            let scope = if j % 2 == 0 { Condition::TRUE } else { choice };
            let mut other =
                Matches::new(&g, root.clone(), c.clone(), 0, scope, Some((0, q))).unwrap();
            let matches = drain(&mut other, &g, &mut a);
            assert_eq!(matches.len(), 2);
            assert_eq!(
                matches.iter().map(|m| m.occurrences[1]).collect::<Vec<_>>(),
                expected[(i * n + j) as usize]
            );
            assert!(matches.iter().all(|m| m.occurrences[0] == q));
            assert_eq!(matches[0].support, scope);
            assert_eq!(matches[1].support, choice);
        }
        let remaining = drain(&mut paused, &g, &mut a);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].occurrences, [p, expected[(i * n) as usize][1]]);
        assert_eq!(remaining[0].support, choice);
    }
    let work = g.restriction_diagnostics();
    assert_eq!(work.created, n);
    assert_eq!(work.reused, n * (n - 1));
    assert_eq!(work.producer_candidates, 2 * n * n);
    assert_eq!(work.projected_ports, 2 * n * n);
    assert_eq!(work.retained_rows, (2 * n * n) as usize);
    assert_eq!(work.retained_partitions, (n * n) as usize);
    let mut collector = g.collect(vec![root].into_iter());
    while !collector.done() {
        collector.tick(&mut g);
    }
    drop(collector);
    assert_eq!(g.restriction_diagnostics().retained_rows, 0);
    assert_eq!(g.restriction_diagnostics().retained_partitions, 0);
}
