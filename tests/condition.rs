use chr::condition::{Arena, Condition, Operation, Progress};

fn finish(arena: &mut Arena, op: Operation) -> Condition {
    let mut job = arena.start(op);
    for _ in 0..100_000 {
        if let Progress::Complete(result) = job.tick(arena) {
            return result;
        }
    }
    panic!("finite condition operation did not complete");
}

#[test]
fn boolean_operations_match_truth_tables_and_are_canonical() {
    let mut a = Arena::default();
    let (_, x) = a.fresh_choice();
    let (_, y) = a.fresh_choice();
    let (_, z) = a.fresh_choice();
    let xy = finish(&mut a, Operation::And(x, y));
    let formula = finish(&mut a, Operation::Or(xy, z));
    let xz = finish(&mut a, Operation::Or(x, z));
    let yz = finish(&mut a, Operation::Or(y, z));
    assert_eq!(formula, finish(&mut a, Operation::And(xz, yz)));
    let difference = finish(&mut a, Operation::Difference(formula, y));
    for bits in 0..8 {
        let v = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        assert_eq!(
            a.evaluate(formula, |i| v[i as usize]),
            (v[0] && v[1]) || v[2]
        );
        assert_eq!(
            a.evaluate(difference, |i| v[i as usize]),
            ((v[0] && v[1]) || v[2]) && !v[1]
        );
        assert_eq!(
            a.evaluate(formula.not(), |i| v[i as usize]),
            !((v[0] && v[1]) || v[2])
        );
    }
}

#[test]
fn uniform_operations_and_negation_need_no_apply_work_or_nodes() {
    let mut a = Arena::default();
    let (_, x) = a.fresh_choice();
    let nodes = a.node_count();
    for (op, expected) in [
        (Operation::And(x, x), x),
        (Operation::And(x, Condition::TRUE), x),
        (Operation::Or(x, x.not()), Condition::TRUE),
        (Operation::And(x, x.not()), Condition::FALSE),
        (Operation::Difference(x, x), Condition::FALSE),
        (Operation::Difference(x, Condition::FALSE), x),
    ] {
        let job = a.start(op);
        assert_eq!(job.result(), Some(expected));
        assert_eq!(job.work(), 0);
        assert_eq!(job.scratch_capacity(), 0);
    }
    assert_eq!(x.not().not(), x);
    assert_eq!(a.node_count(), nodes);
}

#[test]
fn suspended_jobs_share_nodes_and_survive_collection() {
    let mut a = Arena::default();
    let choices: Vec<_> = (0..9).map(|_| a.fresh_choice().1).collect();
    let mut left = Condition::TRUE;
    let mut right = Condition::FALSE;
    for &c in &choices {
        left = finish(&mut a, Operation::And(left, c));
        right = finish(&mut a, Operation::Or(right, c));
    }
    let mut one = a.start(Operation::Difference(right, left));
    let mut two = a.start(Operation::And(right, left));
    assert_eq!(one.tick(&mut a), Progress::Pending);
    two.tick(&mut a);
    let roots: Vec<_> = one
        .roots()
        .chain(two.roots())
        .chain([left, right])
        .collect();
    let mut gc = a.collect(roots.into_iter());
    let mut ticks = 0;
    while !gc.tick(&mut a) {
        ticks += 1;
        assert!(ticks < 10_000);
    }
    drop(gc);
    let result = loop {
        two.tick(&mut a);
        if let Progress::Complete(c) = one.tick(&mut a) {
            break c;
        }
    };
    let second = loop {
        if let Progress::Complete(c) = two.tick(&mut a) {
            break c;
        }
    };
    assert_eq!(second, left);
    for bits in 0..512 {
        assert_eq!(
            a.evaluate(result, |i| bits & (1 << i) != 0),
            bits != 0 && bits != 511
        );
    }
}

#[test]
fn reclamation_does_not_revive_retired_identities() {
    let mut a = Arena::default();
    let (_, keep) = a.fresh_choice();
    let (_, dead) = a.fresh_choice();
    for _ in 0..32 {
        for _ in 0..100 {
            a.fresh_choice();
        }
        let mut gc = a.collect([keep].into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        assert_eq!(a.node_count(), 1);
        assert!(a.contains(keep));
        assert!(!a.contains(dead));
    }
    assert!(
        a.unique_capacity() < 16,
        "dead table storage must be released"
    );
    let (_, new) = a.fresh_choice();
    assert_ne!(new, dead);
    assert!(!a.contains(dead));
    let mut other = Arena::default();
    let (_, foreign) = other.fresh_choice();
    assert!(!a.contains(foreign));
}

#[test]
fn collection_preserves_complemented_descendants_and_drops_weak_cache() {
    let mut a = Arena::default();
    let (_, x) = a.fresh_choice();
    let (_, y) = a.fresh_choice();
    let result = finish(&mut a, Operation::And(x.not(), y));
    assert!(a.cache_len() > 0);
    let mut gc = a.collect([result].into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert_eq!(a.cache_len(), 0);
    for bits in 0..4 {
        assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), bits == 2);
    }
    let mut gc = a.collect(std::iter::empty());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert_eq!(a.node_count(), 0);
}

#[test]
fn all_three_choice_functions_have_unique_truth_table_representatives() {
    let mut a = Arena::default();
    let variables: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let mut minterms = Vec::new();
    for assignment in 0..8 {
        let mut term = Condition::TRUE;
        for (i, &variable) in variables.iter().enumerate() {
            let literal = if assignment & (1 << i) != 0 {
                variable
            } else {
                variable.not()
            };
            term = finish(&mut a, Operation::And(term, literal));
        }
        minterms.push(term);
    }
    let mut functions = Vec::new();
    for table in 0..256 {
        let mut function = Condition::FALSE;
        for (i, &term) in minterms.iter().enumerate() {
            if table & (1 << i) != 0 {
                function = finish(&mut a, Operation::Or(function, term));
            }
        }
        functions.push(function);
    }
    assert_eq!(
        functions
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        256
    );
    for left in 0..256 {
        assert_eq!(functions[left].not(), functions[255 ^ left]);
        for right in 0..256 {
            assert_eq!(
                finish(&mut a, Operation::And(functions[left], functions[right])),
                functions[left & right]
            );
            assert_eq!(
                finish(&mut a, Operation::Or(functions[left], functions[right])),
                functions[left | right]
            );
            assert_eq!(
                finish(
                    &mut a,
                    Operation::Difference(functions[left], functions[right])
                ),
                functions[left & !right]
            );
        }
    }
    assert!(a.cache_len() <= 1024);
}

#[test]
fn deep_operations_and_collection_do_not_use_the_call_stack() {
    let mut a = Arena::default();
    let variables: Vec<_> = (0..12_000).map(|_| a.fresh_choice().1).collect();
    let mut all = Condition::TRUE;
    let mut any = Condition::FALSE;
    for &variable in variables.iter().rev() {
        all = finish(&mut a, Operation::And(variable, all));
        any = finish(&mut a, Operation::Or(variable, any));
    }
    assert_eq!(finish(&mut a, Operation::And(all, any)), all);
    let mut gc = a.collect([all, any].into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert!(a.evaluate(all, |_| true));
    assert!(!a.evaluate(all, |i| i != 6_000));
    assert!(a.evaluate(any, |i| i == 6_000));
    assert!(!a.evaluate(any, |_| false));
}

#[test]
fn compact_shared_suffixes_are_not_recomputed_after_cache_eviction() {
    let mut a = Arena::default();
    let (depth, width) = (8, 1100);
    let variables: Vec<_> = (0..depth * (width + 1))
        .map(|_| a.fresh_choice().1)
        .collect();
    let mut formula = a.fresh_choice().1;
    let y = a.fresh_choice().1;
    for layer in (0..depth).rev() {
        let base = layer * (width + 1);
        let x = variables[base];
        let mut high = formula.not();
        for &variable in variables[base + 1..base + width + 1].iter().rev() {
            high = finish(&mut a, Operation::And(variable, high));
        }
        let low = finish(&mut a, Operation::And(x.not(), formula));
        high = finish(&mut a, Operation::And(x, high));
        formula = finish(&mut a, Operation::Or(low, high));
    }
    let mut gc = a.collect([formula, y].into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    let bound = 32 * a.node_count() as u64;
    let mut job = a.start(Operation::And(formula, y));
    for _ in 0..10_000 {
        job.tick(&mut a);
    }
    // Completed subproblems must survive collection even after they leave the
    // active traversal path and the bounded shared cache has evicted them.
    let mut gc = a.collect(job.roots());
    while !gc.tick(&mut a) {}
    drop(gc);
    while job.result().is_none() && job.work() < bound {
        job.tick(&mut a);
    }
    assert!(
        job.result().is_some(),
        "compact shared suffixes caused repeated work: {} ticks",
        job.work()
    );
    assert_eq!(
        job.scratch_capacity(),
        0,
        "completed operation releases scratch"
    );
    let result = job.result().unwrap();
    for seed in 0..32_u64 {
        let assignment = |i: u64| (i.wrapping_mul(6364136223846793005).wrapping_add(seed)) & 8 != 0;
        assert_eq!(
            a.evaluate(result, assignment),
            a.evaluate(formula, assignment) && a.evaluate(y, assignment)
        );
    }
}

#[test]
fn an_empty_arena_does_not_keep_scanning_its_historical_peak() {
    let mut a = Arena::default();
    for _ in 0..10_000 {
        a.fresh_choice();
    }
    let mut gc = a.collect(std::iter::empty());
    while !gc.tick(&mut a) {}
    drop(gc);
    let mut gc = a.collect(std::iter::empty());
    let mut work = 0;
    while !gc.tick(&mut a) {
        work += 1;
    }
    drop(gc);
    assert!(
        work < 10,
        "empty arena still scanned historical slots: {work}"
    );
    assert_eq!(a.unique_capacity(), 0);
}

#[test]
fn owned_collection_freezes_choices_and_jobs_until_dropped() {
    use chr::condition::Collector;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn begin(arena: &mut Arena, roots: Vec<Condition>) -> Collector<std::vec::IntoIter<Condition>> {
        arena.collect(roots.into_iter())
    }
    for cutoff in 0..48 {
        let mut arena = Arena::default();
        let x = arena.fresh_choice().1;
        let y = arena.fresh_choice().1;
        let mut job = arena.start(Operation::And(x, y));
        let mut gc = begin(&mut arena, job.roots().collect());
        let mut foreign = Arena::default();
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| arena.collect([x].into_iter()))).is_err());
        for _ in 0..cutoff {
            gc.tick(&mut arena);
        }
        assert!(arena.evaluate(x, |_| true));
        let nodes = arena.node_count();
        assert!(catch_unwind(AssertUnwindSafe(|| arena.fresh_choice())).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut arena))).is_err());
        assert_eq!((arena.node_count(), job.work()), (nodes, 0));
        drop(gc);
        assert_eq!(
            arena.fresh_choice().0,
            2,
            "rejected writes cannot consume identities"
        );
        let result = loop {
            if let Progress::Complete(c) = job.tick(&mut arena) {
                break c;
            }
        };
        let mut gc = begin(&mut arena, vec![result]);
        while !gc.tick(&mut arena) {}
        assert!(gc.tick(&mut arena));
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        drop(gc);
        for bits in 0..4 {
            assert_eq!(arena.evaluate(result, |i| bits & (1 << i) != 0), bits == 3);
        }
    }
}

fn traced_job(job: &chr::condition::Job) -> Vec<Condition> {
    use chr::trace::{Cursor, Step, Trace};
    let mut cursor = Cursor::default();
    let mut roots = Vec::new();
    let expected = job.roots().count();
    for _ in 0..(4 * expected + 20) {
        match job.trace(&mut cursor) {
            Step::Root(c) => roots.push(c),
            Step::Pending => {}
            Step::Done => {
                assert_eq!(job.trace(&mut cursor), Step::Done);
                let mut old: Vec<_> = job.roots().collect();
                old.sort();
                roots.sort();
                assert_eq!(
                    roots, old,
                    "trace must preserve every legacy root, including duplicates"
                );
                return roots;
            }
        }
    }
    panic!("trace did not yield a bounded sequence of scalar roots");
}

#[test]
fn job_trace_matches_inventory_then_gc_and_resume_at_every_phase() {
    let mut a = Arena::default();
    let choices: Vec<_> = (0..6).map(|_| a.fresh_choice().1).collect();
    let mut all = Condition::TRUE;
    let mut any = Condition::FALSE;
    for &c in choices.iter().rev() {
        all = finish(&mut a, Operation::And(c, all));
        any = finish(&mut a, Operation::Or(c, any));
    }
    let mut job = a.start(Operation::Difference(any, all));
    for _ in 0..10_000 {
        let roots = traced_job(&job);
        let mut gc = a.collect(roots.into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        if let Progress::Complete(result) = job.tick(&mut a) {
            traced_job(&job);
            assert_eq!(job.result(), Some(result));
            assert_eq!(job.scratch_capacity(), 0);
            for bits in 0..64 {
                assert_eq!(
                    a.evaluate(result, |i| bits & (1 << i) != 0),
                    bits != 0 && bits != 63
                );
            }
            return;
        }
    }
    panic!("traced job did not finish");
}

#[test]
fn large_job_memo_traces_scalars_and_cleans_up_before_completion() {
    let mut a = Arena::default();
    let choices: Vec<_> = (0..300).map(|_| a.fresh_choice().1).collect();
    let mut all = Condition::TRUE;
    for &c in choices.iter().rev() {
        all = finish(&mut a, Operation::And(c, all));
    }
    let y = a.fresh_choice().1;
    let mut job = a.start(Operation::And(all, y));
    let mut peak = 0;
    let mut traced_large = false;
    let mut cleanup_ticks = 0;
    for _ in 0..10_000 {
        let before = job.roots().count();
        peak = peak.max(before);
        if before > 600 && !traced_large {
            traced_job(&job);
            traced_large = true;
        }
        let work = job.work();
        let status = job.tick(&mut a);
        let after = job.roots().count();
        assert!(
            before.saturating_sub(after) <= 4,
            "one tick discarded a wide memo: {before} -> {after}"
        );
        if work == job.work() && before > after {
            cleanup_ticks += 1;
        }
        match status {
            Progress::Pending => assert!(
                job.result().is_none(),
                "cleanup cannot advertise a complete result"
            ),
            Progress::Complete(result) => {
                assert!(a.evaluate(result, |_| true));
                assert!(!a.evaluate(result, |i| i != 300));
                assert!(!a.evaluate(result, |i| i != 151));
                assert!(peak > 600 && traced_large);
                assert!(
                    cleanup_ticks >= 100,
                    "memo cleanup must yield between entries"
                );
                assert_eq!(job.scratch_capacity(), 0);
                return;
            }
        }
    }
    panic!("large memo job did not finish");
}

#[test]
fn discard_large_memo_is_incremental_traceable_and_never_evaluates_more_work() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut a = Arena::default();
    let choices: Vec<_> = (0..300).map(|_| a.fresh_choice().1).collect();
    let mut all = Condition::TRUE;
    for &c in choices.iter().rev() {
        all = finish(&mut a, Operation::And(c, all));
    }
    let y = a.fresh_choice().1;
    let mut job = a.start(Operation::And(all, y));
    let mut stopped = false;
    for _ in 0..10_000 {
        let work = job.work();
        assert_eq!(job.tick(&mut a), Progress::Pending);
        if job.work() == work {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    let initial_roots = job.roots().count();
    assert!(initial_roots > 600, "cancel with a large memo still owned");
    let memo_entries = (initial_roots - 1) / 3; // Cleanup has no frames and one last result.
    let work = job.work();
    let mut calls = 0;
    loop {
        let mut gc = a.collect(traced_job(&job).into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        let before = job.roots().count();
        let done = job.discard_tick();
        calls += 1;
        assert_eq!(job.work(), work, "discard resumed BDD evaluation");
        assert!(job.result().is_none());
        assert!(before.saturating_sub(job.roots().count()) <= if calls == 1 { 4 } else { 3 });
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut a))).is_err());
        if done {
            break;
        }
        assert!(calls <= memo_entries);
    }
    assert_eq!(calls, memo_entries);
    assert_eq!(job.scratch_capacity(), 0);
    assert_eq!(job.roots().count(), 0);
    assert!(job.discard_tick());
    assert!(job.discard_tick());
    let mut gc = a.collect(traced_job(&job).into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert_eq!(a.node_count(), 0);
}

#[test]
fn discard_active_frames_and_completed_jobs_never_publish_a_result() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for prefix in 0..8 {
        let mut a = Arena::default();
        let x = a.fresh_choice().1;
        let y = a.fresh_choice().1;
        let mut job = a.start(Operation::And(x, y));
        for _ in 0..prefix {
            job.tick(&mut a);
        }
        let work = job.work();
        for step in 0..10 {
            let done = job.discard_tick();
            let mut gc = a.collect(traced_job(&job).into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            assert_eq!(job.work(), work);
            assert!(job.result().is_none());
            if done {
                break;
            }
            assert!(step < 9);
        }
        assert!(job.discard_tick());
        assert_eq!(job.scratch_capacity(), 0);
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut a))).is_err());
    }
}

#[test]
fn transform_substitutes_only_the_given_choices_and_preserves_canonical_results() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    let mut arena = Arena::default();
    let (xi, x) = arena.fresh_choice();
    let (yi, y) = arena.fresh_choice();
    let (_, z) = arena.fresh_choice();
    let xy = finish(&mut arena, Operation::And(x, y));
    let formula = finish(&mut arena, Operation::Or(xy, z));
    for (input, assignments, expected) in [
        (formula, BTreeMap::from([(xi, false)]), z),
        (formula, BTreeMap::from([(xi, true), (yi, false)]), z),
        (
            formula.not(),
            BTreeMap::from([(xi, true), (yi, true)]),
            Condition::FALSE,
        ),
        (formula, BTreeMap::new(), formula),
    ] {
        let mut job = arena.substitute(
            input,
            Arc::new(
                assignments
                    .iter()
                    .map(|(&key, &value)| {
                        (
                            key,
                            if value {
                                Condition::TRUE
                            } else {
                                Condition::FALSE
                            },
                        )
                    })
                    .collect(),
            ),
        );
        let result = loop {
            if let Progress::Complete(result) = job.tick(&mut arena) {
                break result;
            }
        };
        assert_eq!(result, expected);
        assert_eq!(job.scratch_capacity(), 0);
        for bits in 0..8 {
            let value = |i: u64| bits & (1 << i) != 0;
            assert_eq!(
                arena.evaluate(result, value),
                arena.evaluate(input, |i| assignments
                    .get(&i)
                    .copied()
                    .unwrap_or_else(|| value(i)))
            );
        }
    }
    assert_eq!(
        arena.fresh_choice().0,
        3,
        "transform must not create search choices"
    );
}

fn traced_transform(job: &chr::condition::Transform) -> Vec<Condition> {
    use chr::trace::{Cursor, Step, Trace};
    let mut cursor = Cursor::default();
    let mut roots = Vec::new();
    for _ in 0..(4 * job.roots().count() + 20) {
        match job.trace(&mut cursor) {
            Step::Root(c) => roots.push(c),
            Step::Pending => {}
            Step::Done => {
                let mut expected: Vec<_> = job.roots().collect();
                expected.sort();
                roots.sort();
                assert_eq!(roots, expected);
                return roots;
            }
        }
    }
    panic!("transform trace did not finish");
}

#[test]
fn transform_all_three_variable_functions_and_partial_assignments() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let vars: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let terms: Vec<_> = (0..8)
        .map(|bits| {
            vars.iter()
                .enumerate()
                .fold(Condition::TRUE, |term, (i, &v)| {
                    finish(
                        &mut a,
                        Operation::And(term, if bits & (1 << i) != 0 { v } else { v.not() }),
                    )
                })
        })
        .collect();
    for truth_table in 0..256 {
        let input = terms
            .iter()
            .enumerate()
            .filter(|(i, _)| truth_table & (1 << i) != 0)
            .fold(Condition::FALSE, |acc, (_, &term)| {
                finish(&mut a, Operation::Or(acc, term))
            });
        for pattern in 0..27 {
            let mut code = pattern;
            let bindings: BTreeMap<_, _> = (0..3)
                .filter_map(|i| {
                    let digit = code % 3;
                    code /= 3;
                    (digit != 0).then_some((i, digit == 2))
                })
                .collect();
            let mut job = a.substitute(
                input,
                Arc::new(
                    bindings
                        .iter()
                        .map(|(&key, &value)| {
                            (
                                key,
                                if value {
                                    Condition::TRUE
                                } else {
                                    Condition::FALSE
                                },
                            )
                        })
                        .collect(),
                ),
            );
            let result = (0..100)
                .find_map(|_| match job.tick(&mut a) {
                    Progress::Complete(c) => Some(c),
                    Progress::Pending => None,
                })
                .expect("bounded small cofactor");
            for bits in 0..8 {
                let substituted = (0..3).fold(bits, |bits, i| match bindings.get(&i) {
                    Some(true) => bits | (1 << i),
                    Some(false) => bits & !(1 << i),
                    None => bits,
                });
                assert_eq!(
                    a.evaluate(result, |i| bits & (1 << i) != 0),
                    truth_table & (1 << substituted) != 0
                );
            }
            assert_eq!(job.scratch_capacity(), 0);
        }
    }
}

#[test]
fn transform_survives_collection_and_cancellation_at_every_phase() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{collections::BTreeMap, sync::Arc};
    for cancel_at in 0..80 {
        let mut a = Arena::default();
        let vars: Vec<_> = (0..6).map(|_| a.fresh_choice().1).collect();
        let all = vars.iter().rev().fold(Condition::TRUE, |acc, &v| {
            finish(&mut a, Operation::And(v, acc))
        });
        let any = vars.iter().rev().fold(Condition::FALSE, |acc, &v| {
            finish(&mut a, Operation::Or(v, acc))
        });
        let input = finish(&mut a, Operation::Difference(any, all));
        let mut job = a.substitute(
            input.not(),
            Arc::new(BTreeMap::from([
                (1, Condition::TRUE),
                (4, Condition::FALSE),
            ])),
        );
        for step in 0..100 {
            let mut gc = a.collect(traced_transform(&job).into_iter());
            assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut a))).is_err());
            while !gc.tick(&mut a) {}
            drop(gc);
            if step == cancel_at {
                let work = job.work();
                let mut done = false;
                for _ in 0..100 {
                    done = job.discard_tick();
                    let mut gc = a.collect(traced_transform(&job).into_iter());
                    while !gc.tick(&mut a) {}
                    drop(gc);
                    assert_eq!(job.work(), work);
                    assert!(job.result().is_none());
                    if done {
                        break;
                    }
                }
                assert!(done);
                assert_eq!(job.scratch_capacity(), 0);
                assert_eq!(a.node_count(), 0);
                assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut a))).is_err());
                break;
            }
            if let Progress::Complete(result) = job.tick(&mut a) {
                // With one forced true and one forced false, neither all nor
                // none can hold, regardless of the remaining choices.
                assert_eq!(result, Condition::FALSE);
                assert_eq!(job.scratch_capacity(), 0);
                break;
            }
            assert!(step < 99);
        }
    }
}

#[test]
fn transform_deep_shared_dag_and_last_assignment_owner_cleanup_are_bounded() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let depth = 12_000;
    let vars: Vec<_> = (0..depth).map(|_| a.fresh_choice().1).collect();
    let mut parity = Condition::FALSE;
    for &v in vars.iter().rev() {
        let left = finish(&mut a, Operation::And(v, parity.not()));
        let right = finish(&mut a, Operation::And(v.not(), parity));
        parity = finish(&mut a, Operation::Or(left, right));
    }
    let mut job = a.substitute(
        parity,
        Arc::new(BTreeMap::from([(depth - 1, Condition::TRUE)])),
    );
    let result = (0..depth * 20)
        .find_map(|_| match job.tick(&mut a) {
            Progress::Complete(c) => Some(c),
            Progress::Pending => None,
        })
        .expect("shared DAG must take linear work");
    assert!(job.work() < depth * 10);
    assert!(a.evaluate(result, |_| false));
    assert!(!a.evaluate(result, |i| i == 0));
    assert_eq!(job.scratch_capacity(), 0);
    let assignments = Arc::new(
        (0..depth)
            .map(|i| (i, Condition::TRUE))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut sole = a.substitute(Condition::TRUE, assignments.clone());
    let mut shared = a.substitute(Condition::TRUE, assignments);
    assert!(!shared.discard_tick());
    assert!(shared.discard_tick());
    assert!(!sole.discard_tick());
    assert_eq!(sole.scratch_capacity(), depth as usize);
    for remaining in (0..depth).rev() {
        assert_eq!(sole.discard_tick(), remaining == 0);
        assert_eq!(sole.scratch_capacity(), remaining as usize);
    }
}

#[test]
fn transform_rejects_foreign_stale_and_unknown_operands() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let mut foreign = Arena::default();
    let x = a.fresh_choice().1;
    let y = foreign.fresh_choice().1;
    assert!(catch_unwind(AssertUnwindSafe(|| a.substitute(y, Arc::default()))).is_err());
    assert!(
        catch_unwind(AssertUnwindSafe(
            || a.substitute(x, Arc::new(BTreeMap::from([(1, Condition::TRUE)])))
        ))
        .is_err()
    );
    let mut job = a.substitute(x, Arc::new(BTreeMap::from([(0, Condition::TRUE)])));
    assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut foreign))).is_err());
    let mut gc = a.collect(std::iter::empty());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert!(catch_unwind(AssertUnwindSafe(|| a.substitute(x, Arc::default()))).is_err());
}

#[test]
fn transform_new_nonterminal_result_survives_every_collection_phase() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let x = a.fresh_choice().1;
    let (yi, y) = a.fresh_choice();
    let z = a.fresh_choice().1;
    let xy = finish(&mut a, Operation::And(x, y));
    let nz = finish(&mut a, Operation::And(x.not(), z));
    let input = finish(&mut a, Operation::Or(xy, nz));
    let mut job = a.substitute(input, Arc::new(BTreeMap::from([(yi, Condition::TRUE)])));
    for _ in 0..100 {
        let mut gc = a.collect(traced_transform(&job).into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        if let Progress::Complete(result) = job.tick(&mut a) {
            let mut gc = a.collect(traced_transform(&job).into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            for bits in 0..8 {
                assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), bits & 5 != 0);
            }
            assert_eq!(job.scratch_capacity(), 0);
            return;
        }
    }
    panic!("transform did not finish");
}

#[test]
fn simultaneous_images_preserve_truth_tables_and_order() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let vars: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let [x, y, z] = [vars[0], vars[1], vars[2]];
    let yz = finish(&mut a, Operation::And(y, z));
    let input = finish(&mut a, Operation::Or(x, yz)).not();
    let images = BTreeMap::from([(0, z.not()), (1, x), (2, y.not())]);
    let mut job = a.substitute(input, Arc::new(images.clone()));
    let result = (0..1000)
        .find_map(|_| match job.tick(&mut a) {
            Progress::Complete(c) => Some(c),
            Progress::Pending => None,
        })
        .expect("finite substitution");
    for bits in 0..8 {
        let value = |i| bits & (1 << i) != 0;
        assert_eq!(
            a.evaluate(result, value),
            a.evaluate(input, |i| a.evaluate(images[&i], value))
        );
    }
    let product = finish(&mut a, Operation::And(x, y.not()));
    let expected = finish(&mut a, Operation::Or(z.not(), product)).not();
    assert_eq!(
        result, expected,
        "canonical order despite images containing older choices"
    );
    assert_eq!(job.scratch_capacity(), 0);
}

#[test]
fn projection_quantifies_the_inclusive_cutoff_suffix() {
    let mut a = Arena::default();
    let x = a.fresh_choice().1;
    let y = a.fresh_choice().1;
    let z = a.fresh_choice().1;
    let yz = finish(&mut a, Operation::Difference(y, z));
    let input = finish(&mut a, Operation::And(x, yz));
    for cutoff in 0..=4 {
        let mut job = a.project_before(input, cutoff);
        let result = (0..1000)
            .find_map(|_| match job.tick(&mut a) {
                Progress::Complete(c) => Some(c),
                Progress::Pending => None,
            })
            .expect("finite projection");
        for bits in 0..8 {
            let expected = (0..8).any(|suffix| {
                a.evaluate(input, |i| {
                    (if i < cutoff { bits } else { suffix }) & (1 << i) != 0
                })
            });
            assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), expected);
        }
        assert_eq!(job.scratch_capacity(), 0);
    }
}

#[test]
fn transforms_match_every_three_choice_function_with_mixed_images_and_projection() {
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let vars: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let terms: Vec<_> = (0..8)
        .map(|bits| {
            vars.iter()
                .enumerate()
                .fold(Condition::TRUE, |term, (i, &v)| {
                    finish(
                        &mut a,
                        Operation::And(term, if bits & (1 << i) != 0 { v } else { v.not() }),
                    )
                })
        })
        .collect();
    let xy = finish(&mut a, Operation::Difference(vars[0], vars[1]));
    let yz = finish(&mut a, Operation::Or(vars[1], vars[2]));
    let maps = [
        BTreeMap::from([(0, vars[2]), (2, vars[0].not())]),
        BTreeMap::from([(0, yz), (1, xy), (2, vars[0])]),
        BTreeMap::from([
            (0, Condition::FALSE),
            (1, vars[0].not()),
            (2, Condition::TRUE),
        ]),
        BTreeMap::from([(2, xy)]), // Unchanged ancestors also need reordered reconstruction.
    ];
    for table in 0..256 {
        let input = terms
            .iter()
            .enumerate()
            .filter(|(i, _)| table & (1 << i) != 0)
            .fold(Condition::FALSE, |acc, (_, &term)| {
                finish(&mut a, Operation::Or(acc, term))
            });
        for images in &maps {
            let mut job = a.substitute(input, Arc::new(images.clone()));
            let result = (0..2000)
                .find_map(|_| match job.tick(&mut a) {
                    Progress::Complete(c) => Some(c),
                    _ => None,
                })
                .unwrap();
            let mut expected = Condition::FALSE;
            for (bits, &term) in terms.iter().enumerate() {
                let value = |i| bits & (1usize << i) != 0;
                let truth = a.evaluate(input, |i| {
                    images
                        .get(&i)
                        .map_or_else(|| value(i), |&c| a.evaluate(c, value))
                });
                assert_eq!(a.evaluate(result, value), truth);
                if truth {
                    expected = finish(&mut a, Operation::Or(expected, term));
                }
            }
            assert_eq!(result, expected, "canonical simultaneous substitution");
        }
        for cutoff in 0..=3 {
            let mut job = a.project_before(input, cutoff);
            let result = (0..1000)
                .find_map(|_| match job.tick(&mut a) {
                    Progress::Complete(c) => Some(c),
                    _ => None,
                })
                .unwrap();
            for bits in 0..8 {
                let expected = (0..8).any(|suffix| {
                    a.evaluate(input, |i| {
                        (if i < cutoff { bits } else { suffix }) & (1 << i) != 0
                    })
                });
                assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), expected);
            }
        }
    }
}

#[test]
fn image_composition_and_projection_survive_gc_and_discard_at_every_tick() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{collections::BTreeMap, sync::Arc};
    for project in [false, true] {
        let mut completed_at = None;
        for cancel_at in 0..1000 {
            let mut a = Arena::default();
            let vars: Vec<_> = (0..4).map(|_| a.fresh_choice().1).collect();
            let ab = finish(&mut a, Operation::And(vars[0], vars[1]));
            let cd = finish(&mut a, Operation::Or(vars[2], vars[3]));
            let input = finish(&mut a, Operation::Difference(cd, ab));
            let image = finish(&mut a, Operation::Difference(vars[0], vars[2]));
            let mut job = if project {
                a.project_before(input.not(), 2)
            } else {
                a.substitute(
                    input,
                    Arc::new(BTreeMap::from([
                        (1, image),
                        (2, vars[0].not()),
                        (3, vars[1]),
                    ])),
                )
            };
            let expected: Vec<_> = (0..16)
                .map(|bits| {
                    let value = |i| bits & (1 << i) != 0;
                    if project {
                        (0..16).any(|suffix| {
                            a.evaluate(input.not(), |i| {
                                (if i < 2 { bits } else { suffix }) & (1 << i) != 0
                            })
                        })
                    } else {
                        a.evaluate(input, |i| match i {
                            1 => a.evaluate(image, value),
                            2 => !value(0),
                            3 => value(1),
                            _ => value(i),
                        })
                    }
                })
                .collect();
            for step in 0..=cancel_at {
                let mut gc = a.collect(traced_transform(&job).into_iter());
                assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut a))).is_err());
                while !gc.tick(&mut a) {}
                drop(gc);
                if step == cancel_at {
                    let work = job.work();
                    let mut done = false;
                    for _ in 0..2000 {
                        done = job.discard_tick();
                        let mut gc = a.collect(traced_transform(&job).into_iter());
                        while !gc.tick(&mut a) {}
                        drop(gc);
                        assert_eq!(job.work(), work);
                        assert!(job.result().is_none());
                        if done {
                            break;
                        }
                    }
                    assert!(done);
                    assert_eq!(job.scratch_capacity(), 0);
                    assert_eq!(a.node_count(), 0);
                } else if let Progress::Complete(result) = job.tick(&mut a) {
                    for (bits, &expected) in expected.iter().enumerate() {
                        assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), expected);
                    }
                    completed_at = Some(step);
                    break;
                }
            }
            if completed_at.is_some() {
                break;
            }
        }
        assert!(completed_at.is_some(), "finite traversal and cleanup");
    }
}

#[test]
fn image_map_roots_survive_last_owner_drain_and_images_validate_on_use() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{collections::BTreeMap, sync::Arc};
    let mut a = Arena::default();
    let vars: Vec<_> = (0..64).map(|_| a.fresh_choice().1).collect();
    let images = Arc::new(
        vars.iter()
            .enumerate()
            .map(|(i, &v)| (i as u64, v.not()))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut job = a.substitute(Condition::TRUE, images);
    assert_eq!(traced_transform(&job).len(), 65);
    assert_eq!(job.tick(&mut a), Progress::Pending); // Move the sole Arc owner into drain.
    for remaining in (0..64).rev() {
        let mut gc = a.collect(traced_transform(&job).into_iter());
        while !gc.tick(&mut a) {}
        drop(gc);
        assert_eq!(
            job.tick(&mut a),
            if remaining == 0 {
                Progress::Complete(Condition::TRUE)
            } else {
                Progress::Pending
            }
        );
        assert_eq!(job.scratch_capacity(), remaining);
    }
    let foreign = Arena::default().fresh_choice().1;
    let mut invalid = a.substitute(vars[63], Arc::new(BTreeMap::from([(63, foreign)])));
    assert!(catch_unwind(AssertUnwindSafe(|| invalid.tick(&mut a))).is_err());
    // Unreferenced images are not scanned at construction or at an input leaf.
    let mut unused = a.substitute(Condition::FALSE, Arc::new(BTreeMap::from([(0, foreign)])));
    while unused.result().is_none() {
        unused.tick(&mut a);
    }
    assert_eq!(unused.result(), Some(Condition::FALSE));
    let stale = a.fresh_choice().1;
    let mut gc = a.collect([vars[63]].into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    let mut invalid = a.substitute(vars[63], Arc::new(BTreeMap::from([(63, stale)])));
    assert!(catch_unwind(AssertUnwindSafe(|| invalid.tick(&mut a))).is_err());
}
