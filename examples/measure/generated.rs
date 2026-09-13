//! Seeded finite workloads; construct sources and semantic expectations before timing.
//!
//! Graph `n` is the vertex count; `rows` repeats each edge/relation (zero leaves
//! isolated vertices). Diamond is a chain of four-vertex diamonds sharing tips,
//! followed by a chain for leftover vertices. DAG edges always point forwards.
//! Boolean seed zero uses disagreement everywhere, including unsatisfiable odd
//! rings; other seeds mix agreement and disagreement. No implicit search occurs.
//! Duplicate heads use `n` copies in each of `rows` groups. Partial joins use
//! `n` distractors on each port and `rows` probes. These accept only `chain`.
use crate::families::{Bindings, Goal, Oracle, Rows, Workload};
use std::collections::HashSet;

pub const CASES: &str =
    "graph-walks proof-dag graph-bits duplicate-heads partial-join partial-join-hit";

pub enum Spec {
    Walks {
        vertices: usize,
        edges: Vec<(usize, usize)>,
    },
    Dag {
        vertices: usize,
        edges: Vec<(usize, usize)>,
        paths: Vec<(usize, usize, usize)>,
    },
    Bits {
        vertices: usize,
        relations: Vec<(usize, usize, bool)>,
    },
    Duplicates {
        copies: usize,
        groups: usize,
    },
    Partial {
        distractors: usize,
        probes: usize,
        hit: bool,
    },
}

fn product(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b)
        .ok_or_else(|| "generated dimensions overflow".into())
}

fn sum(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b)
        .ok_or_else(|| "generated dimensions overflow".into())
}

fn applications(n: usize) -> Result<u64, String> {
    u64::try_from(n).map_err(|_| "generated application count exceeds u64".into())
}

// Check representability of the source/oracle storage, not an arbitrary run budget.
fn storage(items: usize) -> Result<(), String> {
    if product(items, 128)? > isize::MAX as usize {
        return Err("generated dimensions exceed addressable storage".into());
    }
    Ok(())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64: deterministic on every target, including a zero seed.
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn shuffle<T>(&mut self, values: &mut [T]) {
        for i in (1..values.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            values.swap(i, j);
        }
    }
}

fn graph(n: usize, shape: &str, dag: bool, rng: &mut Rng) -> Result<Vec<(usize, usize)>, String> {
    if !matches!(
        shape,
        "chain" | "ring" | "star" | "diamond" | "dense" | "random"
    ) || (dag && shape == "ring")
    {
        return Err(format!("inapplicable graph shape {shape:?}"));
    }
    storage(n)?;
    let mut edges = Vec::new();
    match shape {
        "chain" => edges.extend((1..n).map(|i| (i - 1, i))),
        "ring" => edges.extend((0..n).map(|i| (i, (i + 1) % n))),
        "star" => edges.extend((1..n).map(|i| (0, i))),
        "diamond" => {
            let mut i = 0;
            while n - i > 3 {
                edges.extend([(i, i + 1), (i, i + 2), (i + 1, i + 3), (i + 2, i + 3)]);
                i += 3;
            }
            edges.extend((i + 1..n).map(|j| (j - 1, j)));
        }
        "dense" | "random" => {
            storage(product(n, n)?)?;
            for x in 0..n {
                for y in 0..n {
                    if (!dag || x < y) && (shape == "dense" || rng.next().is_multiple_of(4)) {
                        edges.push((x, y));
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(edges)
}

/// Dynamic programming counts every nonempty edge-occurrence path, independently
/// of the rule engine. Forward vertex order is a topological order by construction.
fn path_counts(n: usize, edges: &[(usize, usize)]) -> Result<Vec<(usize, usize, usize)>, String> {
    storage(product(n, n)?)?;
    let mut outgoing = vec![Vec::new(); n];
    for &(x, y) in edges {
        if x >= y || y >= n {
            return Err("proof-dag requires strictly forward edges".into());
        }
        outgoing[x].push(y);
    }
    let mut paths = Vec::new();
    let mut total = 0;
    for start in 0..n {
        let mut count = vec![0usize; n];
        count[start] = 1; // Empty prefix seeds extensions but is never emitted.
        for x in start..n {
            for &y in &outgoing[x] {
                count[y] = sum(count[y], count[x])?;
            }
        }
        for (end, &copies) in count.iter().enumerate().skip(start + 1) {
            if copies != 0 {
                total = sum(total, copies)?;
                paths.push((start, end, copies));
            }
        }
    }
    storage(total)?;
    Ok(paths)
}

fn valid_assignment(mask: u64, relations: &[(usize, usize, bool)]) -> bool {
    relations
        .iter()
        .all(|&(x, y, different)| (((mask >> x) ^ (mask >> y)) & 1 != 0) == different)
}

pub fn make(case: &str, n: usize, rows: usize, seed: u64, shape: &str) -> Result<Workload, String> {
    storage(sum(n, rows)?)?;
    let mut rng = Rng(seed);
    let mut query = Vec::<String>::new();
    let (program, spec, answers, apps) = match case {
        "graph-walks" | "proof-dag" | "graph-bits" => {
            let bits = case == "graph-bits";
            if bits && n > 16 {
                return Err("graph-bits requires at most 16 variables".into());
            }
            let base = graph(n, shape, case == "proof-dag", &mut rng)?;
            storage(sum(n, product(base.len(), rows)?)?)?;
            query.extend((0..n).map(|i| format!("vertex(V{i})")));
            if bits {
                let relations: Vec<_> = base
                    .into_iter()
                    .flat_map(|(x, y)| {
                        let different = seed == 0 || rng.next() & 1 != 0;
                        std::iter::repeat_n((x, y, different), rows)
                    })
                    .collect();
                query.extend((0..n).map(|i| format!("(zero(V{i});one(V{i}))")));
                query.extend(relations.iter().map(|&(x, y, different)| {
                    format!(
                        "{}(V{x},V{y})",
                        if different { "disagree" } else { "agree" }
                    )
                }));
                let answers = (0..(1u64 << n))
                    .filter(|&mask| valid_assignment(mask, &relations))
                    .count();
                (
                    "agree(X,Y),zero(X),one(Y) ==> fail. agree(X,Y),one(X),zero(Y) ==> fail. disagree(X,X) ==> fail. disagree(X,Y),zero(X),zero(Y) ==> fail. disagree(X,Y),one(X),one(Y) ==> fail.",
                    Spec::Bits {
                        vertices: n,
                        relations,
                    },
                    answers,
                    None,
                )
            } else {
                let edges: Vec<_> = base
                    .into_iter()
                    .flat_map(|edge| std::iter::repeat_n(edge, rows))
                    .collect();
                query.extend(edges.iter().map(|(x, y)| format!("edge(V{x},V{y})")));
                if case == "proof-dag" {
                    let paths = path_counts(n, &edges)?;
                    let count = paths
                        .iter()
                        .try_fold(0usize, |total, &(_, _, copies)| sum(total, copies))?;
                    (
                        "edge(X,Y) ==> reach(X,Y). reach(X,Y),edge(Y,Z) ==> reach(X,Z).",
                        Spec::Dag {
                            vertices: n,
                            edges,
                            paths,
                        },
                        1,
                        Some(applications(count)?),
                    )
                } else {
                    // Count by degrees, independently of the oracle's ordered
                    // occurrence-pair enumeration. A self-loop cannot join itself.
                    let mut incoming = vec![0usize; n];
                    let mut outgoing = vec![0usize; n];
                    let mut loops = 0;
                    for &(x, y) in &edges {
                        outgoing[x] = sum(outgoing[x], 1)?;
                        incoming[y] = sum(incoming[y], 1)?;
                        if x == y {
                            loops += 1;
                        }
                    }
                    let mut count = 0;
                    for i in 0..n {
                        count = sum(count, product(incoming[i], outgoing[i])?)?;
                    }
                    count -= loops;
                    storage(count)?;
                    (
                        "edge(X,Y),edge(Y,Z) ==> path(X,Y,Z).",
                        Spec::Walks { vertices: n, edges },
                        1,
                        Some(applications(count)?),
                    )
                }
            }
        }
        "duplicate-heads" | "partial-join" | "partial-join-hit" => {
            if shape != "chain" {
                return Err(format!("{case} supports only shape chain"));
            }
            if case == "duplicate-heads" {
                let count = product(product(n, n.saturating_sub(1))?, rows)?;
                storage(sum(product(sum(n, 1)?, rows)?, count)?)?;
                for g in 0..rows {
                    query.push(format!("group(G{g})"));
                    query.extend(std::iter::repeat_n(format!("p(G{g})"), n));
                }
                (
                    "p(G),p(G) ==> witness(G,Z).",
                    Spec::Duplicates {
                        copies: n,
                        groups: rows,
                    },
                    1,
                    Some(applications(count)?),
                )
            } else {
                storage(sum(product(n, 2)?, sum(rows, 3)?)?)?;
                let hit = case == "partial-join-hit";
                query.extend(["anchor(A)".into(), "anchor(B)".into()]);
                for i in 0..n {
                    query.push(format!("row(A,C{i},L{i})"));
                    query.push(format!("row(D{i},B,R{i})"));
                }
                query.extend((0..rows).map(|i| format!("probe(A,B,I{i})")));
                if hit {
                    query.push("row(A,B,H)".into());
                }
                (
                    "probe(X,Y,I),row(X,Y,Z) ==> hit(I,Z).",
                    Spec::Partial {
                        distractors: n,
                        probes: rows,
                        hit,
                    },
                    1,
                    Some(applications(if hit { rows } else { 0 })?),
                )
            }
        }
        _ => return Err(format!("unknown generated case {case:?}; choose {CASES}")),
    };
    if seed != 0 {
        rng.shuffle(&mut query);
    }
    Ok(Workload {
        program: program.into(),
        query: if query.is_empty() {
            "true".into()
        } else {
            query.join(",")
        },
        oracle: Oracle::Generated(spec),
        answers: Some(answers),
        apps,
        goal: Goal::Complete,
        expected: vec![],
    })
}

fn add(rows: &mut Rows, name: &str, tuple: Vec<u64>) {
    rows.entry(name.into()).or_default().push(tuple);
}

fn multiset(mut expected: Rows, actual: &Rows) -> Result<(), String> {
    for tuples in expected.values_mut() {
        tuples.sort_unstable();
    }
    // Accept arbitrary output order, including direct use without the shared reader.
    let mut actual = actual.clone();
    for tuples in actual.values_mut() {
        tuples.sort_unstable();
    }
    if expected != actual {
        return Err("incorrect generated residual tuple multiset".into());
    }
    Ok(())
}

impl Spec {
    /// Only explicit Boolean alternatives have unique semantic answer keys.
    pub fn check(&self, b: &Bindings, actual: &Rows) -> Result<Option<u64>, String> {
        if b.values().collect::<HashSet<_>>().len() != b.len() {
            return Err("unexpected query-variable alias".into());
        }
        let v = |name: &str| {
            b.get(name)
                .copied()
                .ok_or_else(|| format!("missing binding {name}"))
        };
        let mut expected = Rows::new();
        let mut key = None;
        match self {
            Self::Walks { vertices, edges }
            | Self::Dag {
                vertices, edges, ..
            } => {
                let ids = (0..*vertices)
                    .map(|i| v(&format!("V{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                for &id in &ids {
                    add(&mut expected, "vertex", vec![id]);
                }
                for &(x, y) in edges {
                    add(&mut expected, "edge", vec![ids[x], ids[y]]);
                }
                if let Self::Dag { paths, .. } = self {
                    for &(x, y, copies) in paths {
                        for _ in 0..copies {
                            add(&mut expected, "reach", vec![ids[x], ids[y]]);
                        }
                    }
                } else {
                    // Occurrences, not vertex tuples, are the head identities.
                    for (i, &(x, y)) in edges.iter().enumerate() {
                        for (j, &(from, z)) in edges.iter().enumerate() {
                            if i != j && y == from {
                                add(&mut expected, "path", vec![ids[x], ids[y], ids[z]]);
                            }
                        }
                    }
                }
            }
            Self::Bits {
                vertices,
                relations,
            } => {
                if *vertices > 16 {
                    return Err("Boolean oracle exceeds 16 variables".into());
                }
                let ids = (0..*vertices)
                    .map(|i| v(&format!("V{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut mask = 0u64;
                for (i, &id) in ids.iter().enumerate() {
                    add(&mut expected, "vertex", vec![id]);
                    let zero = actual
                        .get("zero")
                        .is_some_and(|tuples| tuples.contains(&vec![id]));
                    let one = actual
                        .get("one")
                        .is_some_and(|tuples| tuples.contains(&vec![id]));
                    if zero == one {
                        return Err("expected exactly one Boolean value per vertex".into());
                    }
                    if one {
                        mask |= 1 << i;
                    }
                    add(&mut expected, if one { "one" } else { "zero" }, vec![id]);
                }
                if !valid_assignment(mask, relations) {
                    return Err("assignment violates graph relations".into());
                }
                for &(x, y, different) in relations {
                    add(
                        &mut expected,
                        if different { "disagree" } else { "agree" },
                        vec![ids[x], ids[y]],
                    );
                }
                key = Some(mask);
            }
            Self::Duplicates { copies, groups } => {
                let per_group = product(*copies, copies.saturating_sub(1))?;
                let mut fresh: HashSet<_> = b.values().copied().collect();
                let ids = (0..*groups)
                    .map(|g| v(&format!("G{g}")))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut counts = std::collections::BTreeMap::new();
                for &id in &ids {
                    add(&mut expected, "group", vec![id]);
                    for _ in 0..*copies {
                        add(&mut expected, "p", vec![id]);
                    }
                    counts.insert(id, 0usize);
                }
                for tuple in actual.get("witness").into_iter().flatten() {
                    let [group, local] = tuple.as_slice() else {
                        return Err("witness requires two ports".into());
                    };
                    let count = counts.get_mut(group).ok_or("unknown witness group")?;
                    *count = sum(*count, 1)?;
                    if !fresh.insert(*local) {
                        return Err("witness local is not fresh and unique".into());
                    }
                    add(&mut expected, "witness", tuple.clone());
                }
                if counts.values().any(|&count| count != per_group) {
                    return Err("incorrect witness multiplicity".into());
                }
            }
            Self::Partial {
                distractors,
                probes,
                hit,
            } => {
                let (a, b) = (v("A")?, v("B")?);
                add(&mut expected, "anchor", vec![a]);
                add(&mut expected, "anchor", vec![b]);
                for i in 0..*distractors {
                    add(
                        &mut expected,
                        "row",
                        vec![a, v(&format!("C{i}"))?, v(&format!("L{i}"))?],
                    );
                    add(
                        &mut expected,
                        "row",
                        vec![v(&format!("D{i}"))?, b, v(&format!("R{i}"))?],
                    );
                }
                let payload = if *hit { Some(v("H")?) } else { None };
                if let Some(z) = payload {
                    add(&mut expected, "row", vec![a, b, z]);
                }
                for i in 0..*probes {
                    let id = v(&format!("I{i}"))?;
                    add(&mut expected, "probe", vec![a, b, id]);
                    if let Some(z) = payload {
                        add(&mut expected, "hit", vec![id, z]);
                    }
                }
            }
        }
        multiset(expected, actual)?;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(w: &Workload) -> &Spec {
        match &w.oracle {
            Oracle::Generated(s) => s,
            _ => panic!("generated oracle required"),
        }
    }

    fn vertices(n: usize) -> Bindings {
        (0..n).map(|i| (format!("V{i}"), i as u64 + 10)).collect()
    }

    fn graph_rows(n: usize, edges: &[(u64, u64)]) -> Rows {
        let mut r = Rows::new();
        for i in 0..n {
            add(&mut r, "vertex", vec![i as u64 + 10]);
        }
        for &(x, y) in edges {
            add(&mut r, "edge", vec![x, y]);
        }
        r
    }

    #[test]
    fn walks_check_order_multiplicity_and_aliases() {
        let w = make("graph-walks", 3, 1, 0, "chain").unwrap();
        assert_eq!(w.apps, Some(1));
        let b = vertices(3);
        let mut r = graph_rows(3, &[(10, 11), (11, 12)]);
        add(&mut r, "path", vec![10, 11, 12]);
        assert_eq!(spec(&w).check(&b, &r).unwrap(), None);
        for bad in [vec![12, 11, 10], vec![10, 12, 11], vec![10, 11]] {
            let mut bad_rows = r.clone();
            bad_rows.insert("path".into(), vec![bad]);
            assert!(spec(&w).check(&b, &bad_rows).is_err());
        }
        let mut missing = r.clone();
        missing.remove("path");
        assert!(spec(&w).check(&b, &missing).is_err());
        add(&mut r, "path", vec![10, 11, 12]);
        assert!(spec(&w).check(&b, &r).is_err());
        let mut aliased = b.clone();
        aliased.insert("V1".into(), 10);
        assert!(spec(&w).check(&aliased, &missing).is_err());
        let mut absent = b;
        absent.remove("V2");
        assert!(spec(&w).check(&absent, &missing).is_err());
    }

    #[test]
    fn self_loop_requires_two_distinct_occurrences() {
        for (copies, apps) in [(1, 0), (2, 2), (3, 6)] {
            let w = make("graph-walks", 1, copies, 0, "ring").unwrap();
            assert_eq!(w.apps, Some(apps));
            let mut r = graph_rows(1, &vec![(10, 10); copies]);
            for _ in 0..apps {
                add(&mut r, "path", vec![10, 10, 10]);
            }
            assert!(spec(&w).check(&vertices(1), &r).is_ok());
        }
    }

    #[test]
    fn diamond_counts_proofs_instead_of_reachable_pairs() {
        let w = make("proof-dag", 4, 1, 0, "diamond").unwrap();
        assert_eq!(w.apps, Some(6));
        let mut r = graph_rows(4, &[(10, 11), (10, 12), (11, 13), (12, 13)]);
        for pair in [[10, 11], [10, 12], [11, 13], [12, 13], [10, 13], [10, 13]] {
            add(&mut r, "reach", pair.to_vec());
        }
        assert!(spec(&w).check(&vertices(4), &r).is_ok());
        r.get_mut("reach").unwrap().pop();
        assert!(spec(&w).check(&vertices(4), &r).is_err());
        assert_eq!(
            make("proof-dag", 4, 2, 0, "diamond").unwrap().apps,
            Some(16)
        );
    }

    #[test]
    fn boolean_oracle_enumerates_disconnected_and_unsatisfiable_states() {
        // Independent hand-built relation: V0 != V1, with V2 isolated.
        let s = Spec::Bits {
            vertices: 3,
            relations: vec![(0, 1, true)],
        };
        for mask in 0u64..8 {
            let mut r = Rows::new();
            for i in 0..3 {
                add(&mut r, "vertex", vec![10 + i]);
                add(
                    &mut r,
                    if mask & (1 << i) == 0 { "zero" } else { "one" },
                    vec![10 + i],
                );
            }
            add(&mut r, "disagree", vec![10, 11]);
            let valid = matches!(mask, 1 | 2 | 5 | 6);
            assert_eq!(s.check(&vertices(3), &r).is_ok(), valid);
            if valid {
                assert_eq!(s.check(&vertices(3), &r).unwrap(), Some(mask));
                add(&mut r, "zero", vec![10]);
                assert!(s.check(&vertices(3), &r).is_err());
            }
        }
        assert_eq!(
            make("graph-bits", 3, 1, 0, "ring").unwrap().answers,
            Some(0)
        );
        assert_eq!(
            make("graph-bits", 3, 0, 0, "chain").unwrap().answers,
            Some(8)
        );
        assert_eq!(
            make("graph-bits", 4, 1, 0, "ring").unwrap().answers,
            Some(2)
        );
        // Agreement must reject opposite values, independently of disagreement.
        let same = Spec::Bits {
            vertices: 2,
            relations: vec![(0, 1, false)],
        };
        for (left, right, valid) in [
            ("zero", "zero", true),
            ("one", "one", true),
            ("zero", "one", false),
            ("one", "zero", false),
        ] {
            let mut r = Rows::from([
                ("vertex".into(), vec![vec![10], vec![11]]),
                ("agree".into(), vec![vec![10, 11]]),
            ]);
            add(&mut r, left, vec![10]);
            add(&mut r, right, vec![11]);
            assert_eq!(same.check(&vertices(2), &r).is_ok(), valid);
        }
    }

    #[test]
    fn duplicate_heads_require_fresh_distinct_witnesses_in_each_group() {
        let w = make("duplicate-heads", 2, 2, 0, "chain").unwrap();
        assert_eq!(w.apps, Some(4));
        let b = Bindings::from([("G0".into(), 10), ("G1".into(), 11)]);
        let mut r = Rows::from([
            ("group".into(), vec![vec![10], vec![11]]),
            ("p".into(), vec![vec![10], vec![10], vec![11], vec![11]]),
            (
                "witness".into(),
                vec![vec![10, 20], vec![10, 21], vec![11, 22], vec![11, 23]],
            ),
        ]);
        assert_eq!(spec(&w).check(&b, &r).unwrap(), None);
        for bad in [vec![10, 20], vec![11, 21], vec![10, 10], vec![21, 10]] {
            r.get_mut("witness").unwrap()[1] = bad;
            assert!(spec(&w).check(&b, &r).is_err());
        }
    }

    #[test]
    fn partial_join_checks_unbound_payload_and_exact_probe() {
        let b: Bindings = ["A", "B", "C0", "D0", "L0", "R0", "I0", "H"]
            .into_iter()
            .enumerate()
            .map(|(i, s)| (s.into(), i as u64))
            .collect();
        for hit in [false, true] {
            let w = make(
                if hit {
                    "partial-join-hit"
                } else {
                    "partial-join"
                },
                1,
                1,
                0,
                "chain",
            )
            .unwrap();
            let mut bindings = b.clone();
            let mut r = Rows::from([
                ("anchor".into(), vec![vec![0], vec![1]]),
                ("row".into(), vec![vec![0, 2, 4], vec![3, 1, 5]]),
                ("probe".into(), vec![vec![0, 1, 6]]),
            ]);
            if hit {
                add(&mut r, "row", vec![0, 1, 7]);
                add(&mut r, "hit", vec![6, 7]);
            } else {
                bindings.remove("H");
            }
            assert!(spec(&w).check(&bindings, &r).is_ok());
            r.insert("hit".into(), vec![vec![6, 4]]);
            assert!(spec(&w).check(&bindings, &r).is_err());
        }
    }

    #[test]
    fn checked_dimensions_and_inapplicable_shapes() {
        for case in CASES.split_whitespace() {
            assert!(make(case, 2, 1, 0, "unknown").is_err());
            assert!(make(case, usize::MAX, 2, 0, "chain").is_err());
        }
        assert!(make("proof-dag", 3, 1, 0, "ring").is_err());
        assert!(make("duplicate-heads", 3, 1, 0, "star").is_err());
        assert!(make("partial-join", 3, 1, 0, "random").is_err());
        assert!(make("graph-bits", 17, 1, 0, "chain").is_err());
        assert!(make("proof-dag", 66, 1, 0, "dense").is_err());
    }

    #[test]
    fn seeds_reproduce_source_and_preserve_analytic_topologies() {
        for case in CASES.split_whitespace() {
            let a = make(case, 4, 2, 72, "chain").unwrap();
            let b = make(case, 4, 2, 72, "chain").unwrap();
            assert_eq!((a.program, a.query), (b.program, b.query));
        }
        let a = make("graph-walks", 8, 1, 1, "random").unwrap();
        let b = make("graph-walks", 8, 1, 2, "random").unwrap();
        assert_ne!(a.query, b.query);
        assert_eq!(
            make("graph-walks", 4, 1, 72, "chain").unwrap().apps,
            Some(2)
        );
    }

    #[test]
    fn generated_sources_execute_against_the_oracles() {
        use chr::{
            engine::Engine,
            program::prepare,
            syntax::{parse_program, parse_query},
        };
        use std::sync::Arc;
        for case in CASES.split_whitespace() {
            let shapes: &[&str] = if case.starts_with("graph-") {
                &["chain", "ring", "star", "diamond", "dense", "random"]
            } else if case == "proof-dag" {
                &["chain", "star", "diamond", "dense", "random"]
            } else {
                &["chain"]
            };
            for &shape in shapes {
                for (n, rows, seed) in [(0, 0, 0), (1, 2, 0), (3, 0, 0), (4, 1, 0), (4, 2, 17)] {
                    let mut w = make(case, n, rows, seed, shape).unwrap();
                    let mut e = Engine::new(Arc::new(
                        prepare(
                            &parse_program(&w.program).unwrap(),
                            &parse_query(&w.query).unwrap(),
                        )
                        .unwrap(),
                    ));
                    let mut reader = crate::Reader::default();
                    for _ in 0..2_000_000 {
                        e.advance(1);
                        while let Some(event) = e.take_output() {
                            reader.push(event, &e, &mut w).unwrap_or_else(|error| {
                                panic!("{case}/{shape}/{n}/{rows}/{seed}: {error}")
                            });
                        }
                        if e.delivery_done() {
                            break;
                        }
                    }
                    assert!(e.delivery_done(), "{case}/{shape}/{n}/{rows}/{seed}");
                    assert_eq!(Some(reader.answers), w.answers, "{case}/{shape}");
                    if let Some(apps) = w.apps {
                        assert_eq!(e.applications(), apps, "{case}/{shape}");
                    }
                }
            }
        }
    }
}
