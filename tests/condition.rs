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
