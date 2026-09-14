#![cfg(feature = "diagnostics")]
use chr::{
    condition::{Arena, Condition},
    graph::{Graph, UpdateStatus},
    program::Signature,
    store::Root,
};
#[allow(dead_code)]
#[path = "../examples/measure/allocation.rs"]
mod allocation;
use allocation::{Phase, during};

fn checkpoint(label: &str, g: &Graph) {
    let allocation = allocation::snapshot();
    println!(
        "liveness_case={}",
        serde_json::json!({
            "phase": label, "allocation": allocation,
            "graph_allocations": g.index_allocations(),
            "graph_pages": g.index_page_counts(),
            "liveness": g.field_update_diagnostics(),
            "rows": g.occurrence_count(), "nodes": g.index_node_count()
        })
    );
}
fn prune(g: &mut Graph, a: &mut Arena, root: Root, active: Condition) -> Root {
    let mut p = g.prune(root, active);
    for _ in 0..1_000_000 {
        if let Some(r) = p.tick(g, a) {
            return r;
        }
    }
    panic!("finite prune failed to finish");
}
fn collect(g: &mut Graph, a: &mut Arena, roots: Vec<Root>, active: Option<Condition>) {
    let mut supports: Vec<_> = active.into_iter().collect();
    let mut gc = g.collect(roots.into_iter());
    for _ in 0..1_000_000 {
        if gc.done() {
            break;
        }
        if let Some(c) = gc.tick(g) {
            supports.push(c);
        }
    }
    assert!(gc.done());
    drop(gc);
    let mut gc = a.collect(supports.into_iter());
    for _ in 0..1_000_000 {
        if gc.tick(a) {
            return;
        }
    }
    panic!("finite collection failed to finish");
}
#[test]
fn sparse_dense_and_archive_certificate_lifecycle() {
    let selector = std::env::var("CHRIMP_LIVENESS_CASE").unwrap_or_else(|_| "256/8".into());
    let cancel = selector.ends_with("/cancel");
    let selector = selector.strip_suffix("/cancel").unwrap_or(&selector);
    let (n, changed) = selector.split_once('/').unwrap();
    let n: usize = n.parse().unwrap();
    let changed: usize = changed.parse().unwrap();
    assert!(changed <= n && n > 0);
    let mut a = Arena::default();
    let (_, active) = a.fresh_choice();
    let mut g = Graph::new(&[Signature {
        name: "p".into(),
        arity: 2,
    }]);
    let (mut root, ids) = during(Phase::Setup, || {
        let mut root = g.empty();
        let mut ids = Vec::new();
        for i in 0..n {
            // Two distinct occurrences per argument tuple.
            let v = (i / 2) as u64;
            let mut u = g.post(root, 0, vec![v, v + n as u64], active).unwrap();
            ids.push(u.occurrence());
            root = loop {
                if let UpdateStatus::Complete(r) = u.tick(&mut g) {
                    break r;
                }
            };
        }
        (prune(&mut g, &mut a, root, active), ids)
    });
    let original = root.clone();
    checkpoint("prepared", &g);
    root = during(Phase::Engine, || {
        for &id in ids.iter().take(changed) {
            let mut u = g.set_liveness(root, id, Condition::TRUE).unwrap();
            root = loop {
                if let UpdateStatus::Complete(r) = u.tick(&mut g) {
                    break r;
                }
            };
        }
        root
    });
    checkpoint("mutated", &g);
    if cancel {
        during(Phase::Validator, || {
            for (i, &id) in ids.iter().enumerate() {
                assert_eq!(
                    g.fact(root.clone(), id).unwrap().support,
                    if i < changed { Condition::TRUE } else { active }
                );
                assert_eq!(g.fact(original.clone(), id).unwrap().support, active);
            }
        });
        during(Phase::Cleanup, || {
            drop(root);
            drop(original);
            collect(&mut g, &mut a, vec![], None);
        });
        assert_eq!(
            (g.occurrence_count(), g.index_node_count(), a.node_count()),
            (0, 0, 0)
        );
        checkpoint("canceled", &g);
        during(Phase::EngineDrop, || {
            drop(g);
            drop(a);
            drop(ids);
        });
        println!(
            "liveness_case={}",
            serde_json::json!({"phase":"dropped", "allocation":allocation::snapshot()})
        );
        return;
    }
    root = during(Phase::Engine, || prune(&mut g, &mut a, root, active));
    checkpoint("pruned", &g);
    during(Phase::Validator, || {
        for (i, &id) in ids.iter().enumerate() {
            for r in [&root, &original] {
                let fact = g.fact(r.clone(), id).unwrap();
                assert_eq!(fact.support, active);
                assert_eq!(fact.args, &[(i / 2) as u64, (i / 2) as u64 + n as u64]);
            }
        }
    });
    during(Phase::Inspection, || {
        collect(
            &mut g,
            &mut a,
            vec![root.clone(), original.clone()],
            Some(active),
        )
    });
    checkpoint("retained", &g);
    root = during(Phase::Engine, || prune(&mut g, &mut a, root, active));
    checkpoint("repeated", &g);
    during(Phase::Cleanup, || {
        drop(root);
        drop(original);
        collect(&mut g, &mut a, vec![], None);
    });
    assert_eq!(
        (g.occurrence_count(), g.index_node_count(), a.node_count()),
        (0, 0, 0)
    );
    checkpoint("released", &g);
    during(Phase::EngineDrop, || {
        drop(g);
        drop(a);
        drop(ids);
    });
    println!(
        "liveness_case={}",
        serde_json::json!({"phase":"dropped", "allocation":allocation::snapshot()})
    );
}
