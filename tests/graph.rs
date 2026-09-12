use chr::condition::{Arena, Condition, Operation, Progress};
use chr::graph::{Graph, UpdateStatus};
use chr::program::Signature;

fn graph() -> Graph {
    Graph::new(&[
        Signature {
            name: "p".into(),
            arity: 2,
        },
        Signature {
            name: "q".into(),
            arity: 1,
        },
    ])
}

#[test]
fn posting_preserves_port_order_multiplicity_and_coherent_snapshots() {
    let mut graph = graph();
    let original = graph.empty();
    let mut one = graph
        .post(original.clone(), 0, vec![7, 9], Condition::TRUE)
        .unwrap();
    let id = one.occurrence();
    assert!(graph.fact(original, id).is_none());
    let root = loop {
        if let UpdateStatus::Complete(root) = one.tick(&mut graph) {
            break root;
        }
    };
    assert_eq!(graph.fact(root.clone(), id).unwrap().args, &[7, 9]);
    assert_eq!(
        graph.port(root.clone(), 0, 0, 7).unwrap().next(&graph),
        Some((id, Condition::TRUE))
    );
    assert_eq!(
        graph.port(root.clone(), 0, 1, 7).unwrap().next(&graph),
        None
    );
    let mut duplicate = graph
        .post(root.clone(), 0, vec![7, 9], Condition::TRUE)
        .unwrap();
    let duplicate_id = duplicate.occurrence();
    assert_ne!(id, duplicate_id);
    let root2 = loop {
        if let UpdateStatus::Complete(root) = duplicate.tick(&mut graph) {
            break root;
        }
    };
    let mut rows = graph.relation(root2, 0).unwrap();
    assert_eq!(rows.next(&graph), Some((id, Condition::TRUE)));
    assert_eq!(rows.next(&graph), Some((duplicate_id, Condition::TRUE)));
    assert_eq!(rows.next(&graph), None);
    assert_eq!(
        graph.relation(root.clone(), 0).unwrap().next(&graph),
        Some((id, Condition::TRUE))
    );
    assert!(graph.fact(root, duplicate_id).is_none());
}

#[test]
fn conditional_consumption_updates_every_index_without_changing_sibling_support() {
    let mut graph = graph();
    let mut conditions = Arena::default();
    let (_, choice) = conditions.fresh_choice();
    let mut post = graph
        .post(graph.empty(), 0, vec![4, 4], Condition::TRUE)
        .unwrap();
    let id = post.occurrence();
    let root = loop {
        if let UpdateStatus::Complete(root) = post.tick(&mut graph) {
            break root;
        }
    };
    let mut difference = conditions.start(Operation::Difference(Condition::TRUE, choice));
    let support = loop {
        if let Progress::Complete(c) = difference.tick(&mut conditions) {
            break c;
        }
    };
    let mut update = graph.set_liveness(root.clone(), id, support).unwrap();
    let modified = loop {
        if let UpdateStatus::Complete(root) = update.tick(&mut graph) {
            break root;
        }
    };
    assert_eq!(graph.fact(root, id).unwrap().support, Condition::TRUE);
    assert_eq!(
        graph.fact(modified.clone(), id).unwrap().support,
        choice.not()
    );
    for mut cursor in [
        graph.relation(modified.clone(), 0).unwrap(),
        graph.port(modified.clone(), 0, 0, 4).unwrap(),
        graph.port(modified.clone(), 0, 1, 4).unwrap(),
        graph.incidence(modified.clone(), 4),
    ] {
        assert_eq!(cursor.next(&graph), Some((id, choice.not())));
        assert_eq!(cursor.next(&graph), None);
    }
    let mut remove = graph.set_liveness(modified, id, Condition::FALSE).unwrap();
    let empty = loop {
        if let UpdateStatus::Complete(root) = remove.tick(&mut graph) {
            break root;
        }
    };
    assert!(graph.fact(empty.clone(), id).is_none());
    assert_eq!(graph.incidence(empty, 4).next(&graph), None);
}

#[test]
fn collection_traces_live_conditions_and_retained_inspections_only() {
    let mut graph = graph();
    let mut conditions = Arena::default();
    let (_, c) = conditions.fresh_choice();
    let mut post = graph.post(graph.empty(), 1, vec![10], c).unwrap();
    let id = post.occurrence();
    let snapshot = loop {
        if let UpdateStatus::Complete(root) = post.tick(&mut graph) {
            break root;
        }
    };
    let mut remove = graph
        .set_liveness(snapshot.clone(), id, Condition::FALSE)
        .unwrap();
    let root = loop {
        if let UpdateStatus::Complete(root) = remove.tick(&mut graph) {
            break root;
        }
    };
    let mut gc = graph.collect([root.clone(), snapshot.clone()].into_iter());
    let mut supports = Vec::new();
    while !gc.done() {
        if let Some(support) = gc.tick(&mut graph) {
            supports.push(support);
        }
    }
    drop(gc);
    let mut gc = conditions.collect(supports.into_iter());
    while !gc.tick(&mut conditions) {}
    drop(gc);
    assert!(graph.fact(snapshot, id).is_some());
    assert!(conditions.contains(c));
    assert_eq!(graph.occurrence_count(), 1);
    let mut gc = graph.collect([root].into_iter());
    while !gc.done() {
        gc.tick(&mut graph);
    }
    drop(gc);
    assert_eq!(graph.occurrence_count(), 0);
    drop(post);
    drop(remove);
    while !graph.release_tick() {}
    assert_eq!(graph.index_node_count(), 0);
}

#[test]
fn invalid_posts_are_rejected_before_allocating_occurrences() {
    let mut graph = graph();
    assert!(
        graph
            .post(graph.empty(), 0, vec![0], Condition::TRUE)
            .is_err()
    );
    assert!(
        graph
            .post(graph.empty(), 4, vec![], Condition::TRUE)
            .is_err()
    );
    assert!(graph.port(graph.empty(), 0, 2, 0).is_err());
    assert_eq!(graph.occurrence_count(), 0);
}

#[test]
fn unchanged_liveness_does_not_rewalk_ports_or_allocate_index_versions() {
    let mut graph = graph();
    let mut post = graph
        .post(graph.empty(), 0, vec![1, 2], Condition::TRUE)
        .unwrap();
    let id = post.occurrence();
    let root = loop {
        if let UpdateStatus::Complete(root) = post.tick(&mut graph) {
            break root;
        }
    };
    let before = graph.index_node_count();
    let mut unchanged = graph
        .set_liveness(root.clone(), id, Condition::TRUE)
        .unwrap();
    assert_eq!(unchanged.tick(&mut graph), UpdateStatus::Complete(root));
    assert_eq!(graph.index_node_count(), before);
}

#[test]
fn an_unpublished_post_survives_collection_and_finishes_its_indexes() {
    let mut graph = graph();
    let empty = graph.empty();
    let mut post = graph
        .post(empty.clone(), 0, vec![3, 8], Condition::TRUE)
        .unwrap();
    let id = post.occurrence();
    assert_eq!(post.tick(&mut graph), UpdateStatus::Pending);
    let mut gc = graph.collect(post.roots().into_iter());
    while !gc.done() {
        gc.tick(&mut graph);
    }
    drop(gc);
    assert!(graph.fact(empty, id).is_none());
    let root = loop {
        if let UpdateStatus::Complete(root) = post.tick(&mut graph) {
            break root;
        }
    };
    assert_eq!(graph.fact(root.clone(), id).unwrap().args, &[3, 8]);
    assert_eq!(
        graph.port(root.clone(), 0, 0, 3).unwrap().next(&graph),
        Some((id, Condition::TRUE))
    );
    assert_eq!(
        graph.port(root, 0, 1, 8).unwrap().next(&graph),
        Some((id, Condition::TRUE))
    );
}

#[test]
fn owned_collection_freezes_metadata_and_pending_updates_until_drop() {
    use chr::graph::Collector;
    use chr::store::Root;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn begin(graph: &mut Graph, roots: [Root; 2]) -> Collector<std::array::IntoIter<Root, 2>> {
        graph.collect(roots.into_iter())
    }
    for cutoff in 0..48 {
        let mut g = graph();
        let mut update = g.post(g.empty(), 0, vec![3, 8], Condition::TRUE).unwrap();
        let id = update.occurrence();
        let staged = update.roots()[1].clone();
        let mut gc = begin(&mut g, update.roots());
        let mut foreign = graph();
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(|| g.collect([staged.clone()].into_iter()))).is_err()
        );
        for _ in 0..cutoff {
            gc.tick(&mut g);
        }
        assert_eq!(g.fact(staged.clone(), id).unwrap().args, [3, 8]);
        let counts = (g.occurrence_count(), g.index_node_count());
        assert!(
            catch_unwind(AssertUnwindSafe(|| g.post(
                staged.clone(),
                1,
                vec![9],
                Condition::TRUE
            )))
            .is_err()
        );
        assert!(
            catch_unwind(AssertUnwindSafe(|| g.set_liveness(
                staged,
                id,
                Condition::FALSE
            )))
            .is_err()
        );
        assert!(catch_unwind(AssertUnwindSafe(|| update.tick(&mut g))).is_err());
        assert_eq!((g.occurrence_count(), g.index_node_count()), counts);
        drop(gc);
        let root = loop {
            if let UpdateStatus::Complete(root) = update.tick(&mut g) {
                break root;
            }
        };
        assert_eq!(
            g.port(root.clone(), 0, 1, 8).unwrap().next(&g),
            Some((id, Condition::TRUE))
        );
        let next = g.post(root, 1, vec![9], Condition::TRUE).unwrap();
        assert_eq!(next.occurrence(), id + 1);
        let mut gc = begin(&mut g, next.roots());
        while !gc.done() {
            gc.tick(&mut g);
        }
        assert_eq!(gc.tick(&mut g), None);
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        drop(gc);
    }
}
