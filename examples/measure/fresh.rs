//! Fresh production, identity contraction and occurrence consumption interact.
//!
//! A step consumes its input and posts cell_d(K,V), mark_d(I,V), step_{d+1}(V,I).
//! V is body-only, hence fresh for each application (program::RulePlan's
//! head_variables boundary). I identifies a copy and never participates in
//! equality. In fresh-contract, cell_d(K,V) \\ cell_d(K,W) consumes one cell
//! and equates V/W. Those equalities make the NEXT level's previously distinct
//! keys equal, enabling its nonbinding head match. Groups never share keys.
//!
//! The finite normal form follows by induction on levels: each group has one
//! cell at each level after contraction, while every copy's marker points to
//! that level's common value. Fresh-unmerged runs identical production without
//! contraction and must retain disjoint fresh chains for all copies. One copy
//! is an equivalent control; depth zero is a consumption-only control.
//!
//! No survivor occurrence or numeric variable representative is prescribed.
//! The oracle reads existential witnesses from markers, checks every identity
//! constraint, then compares the entire expected residual multiset. It does not
//! execute rules to obtain expectations. Output/validator size is O(groups *
//! copies * depth); markers deliberately preserve evidence of each producer.
//!
//! Query orders grouped/interleaved/reverse perturb admission of independent
//! producers. This is an explicit order control, not a random-input claim.
//! All use the shared Workload runner (baseline/detail, deadlines and cleanup).
use crate::families::{Bindings, Goal, Oracle, Rows, Workload};
use std::collections::{BTreeMap, BTreeSet};

pub const CASES: &str = "fresh-contract fresh-unmerged";
pub const ORDERS: &str = "grouped interleaved reverse";

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub groups: usize,
    pub copies: usize,
    pub depth: usize,
    pub contract: bool,
}
fn product(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b)
        .ok_or_else(|| "fresh dimensions overflow".into())
}
fn sum(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b)
        .ok_or_else(|| "fresh dimensions overflow".into())
}
impl Spec {
    fn validate(self) -> Result<(), String> {
        if self.groups == 0 || self.copies == 0 {
            return Err("fresh groups and copies must be positive; depth may be zero".into());
        }
        let producers = product(self.groups, self.copies)?;
        let stages = sum(self.depth, 1)?;
        // Include generated source and the oracle's observable markers/edges.
        let items = sum(sum(product(producers, stages)?, self.groups)?, stages)?;
        if product(items, 256)? > isize::MAX as usize {
            return Err("fresh workload exceeds addressable storage".into());
        }
        Ok(())
    }
    fn applications(self) -> Result<u64, String> {
        let production = product(product(self.groups, self.copies)?, sum(self.depth, 1)?)?;
        let contraction = if self.contract {
            product(product(self.groups, self.depth)?, self.copies - 1)?
        } else {
            0
        };
        u64::try_from(sum(production, contraction)?)
            .map_err(|_| "fresh applications exceed u64".into())
    }
}

/// SIZE=groups, --rows=copies, --depth=depth, --order=grouped|interleaved|reverse.
/// PM adds Oracle::Fresh(Spec) and delegates its check to Spec::check.
pub fn make(
    case: &str,
    groups: usize,
    copies: usize,
    depth: usize,
    order: &str,
) -> Result<Workload, String> {
    let contract = match case {
        "fresh-contract" => true,
        "fresh-unmerged" => false,
        _ => return Err(format!("unknown fresh case {case}")),
    };
    if !ORDERS.split_whitespace().any(|o| o == order) {
        return Err(format!("unknown fresh input order {order}"));
    }
    let spec = Spec {
        groups,
        copies,
        depth,
        contract,
    };
    spec.validate()?;
    let apps = spec.applications()?;
    let mut program = String::new();
    for d in 0..depth {
        program += &format!(
            "step{d}(K,I) <=> cell{d}(K,V),mark{d}(I,V),step{}(V,I).",
            d + 1
        );
        if contract {
            program += &format!("cell{d}(K,V) \\ cell{d}(K,W) <=> V=W.");
        }
    }
    program += &format!("step{depth}(K,I) <=> end(I,K).");
    let mut query = (0..groups)
        .map(|g| format!("group(G{g})"))
        .collect::<Vec<_>>();
    let producer = |g: usize, c: usize| format!("step0(G{g},I{g}_{c})");
    if order == "interleaved" {
        for c in 0..copies {
            for g in 0..groups {
                query.push(producer(g, c));
            }
        }
    } else {
        for g in 0..groups {
            for c in 0..copies {
                query.push(producer(g, c));
            }
        }
        if order == "reverse" {
            query.reverse();
        }
    }
    Ok(Workload {
        program,
        query: query.join(","),
        oracle: Oracle::Fresh(spec),
        answers: Some(1),
        apps: Some(apps),
        goal: Goal::Complete,
        expected: vec![],
    })
}
fn add(rows: &mut Rows, name: &str, tuple: Vec<u64>) {
    rows.entry(name.into()).or_default().push(tuple);
}
impl Spec {
    pub fn check(&self, bindings: &Bindings, actual: &Rows) -> Result<Option<u64>, String> {
        self.validate()?;
        let binding_count = sum(self.groups, product(self.groups, self.copies)?)?;
        if bindings.len() != binding_count {
            return Err("incorrect fresh query binding count".into());
        }
        let mut identities: BTreeSet<_> = bindings.values().copied().collect();
        if identities.len() != bindings.len() {
            return Err("fresh query identities aliased".into());
        }
        let binding = |name: &str| {
            bindings
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing binding {name}"))
        };
        let roots = (0..self.groups)
            .map(|g| binding(&format!("G{g}")))
            .collect::<Result<Vec<_>, _>>()?;
        let mut copies = Vec::new();
        let mut expected = Rows::new();
        let mut previous = Vec::new();
        for (g, &root) in roots.iter().enumerate() {
            add(&mut expected, "group", vec![root]);
            for c in 0..self.copies {
                copies.push(binding(&format!("I{g}_{c}"))?);
                previous.push(root);
            }
        }
        for d in 0..self.depth {
            let name = format!("mark{d}");
            let mut witnesses = BTreeMap::new();
            for tuple in actual.get(&name).into_iter().flatten() {
                let [copy, value] = tuple.as_slice() else {
                    return Err("fresh marker arity".into());
                };
                if witnesses.insert(*copy, *value).is_some() {
                    return Err("duplicate fresh producer marker".into());
                }
            }
            if witnesses.len() != copies.len() {
                return Err("missing or extra fresh producer markers".into());
            }
            let mut next = Vec::with_capacity(copies.len());
            for (index, &copy) in copies.iter().enumerate() {
                let value = *witnesses.get(&copy).ok_or("unknown/missing marker copy")?;
                let first = index.is_multiple_of(self.copies);
                if self.contract && !first {
                    if value != next[index - index % self.copies] {
                        return Err("corresponding fresh levels were not contracted".into());
                    }
                } else if !identities.insert(value) {
                    return Err(
                        "fresh level aliases an input, another group/copy, or another level".into(),
                    );
                }
                add(&mut expected, &name, vec![copy, value]);
                if !self.contract || first {
                    add(
                        &mut expected,
                        &format!("cell{d}"),
                        vec![previous[index], value],
                    );
                }
                next.push(value);
            }
            previous = next;
        }
        for (copy, tail) in copies.into_iter().zip(previous) {
            add(&mut expected, "end", vec![copy, tail]);
        }
        for tuples in expected.values_mut() {
            tuples.sort_unstable();
        }
        let mut actual = actual.clone();
        for tuples in actual.values_mut() {
            tuples.sort_unstable();
        }
        if expected != actual {
            return Err("incorrect fresh final graph or occurrence multiplicity".into());
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Hand-calculable two-copy, two-level contracted chain.
    fn contracted() -> (Spec, Bindings, Rows) {
        let spec = Spec {
            groups: 1,
            copies: 2,
            depth: 2,
            contract: true,
        };
        let b = Bindings::from([("G0".into(), 10), ("I0_0".into(), 20), ("I0_1".into(), 21)]);
        let rows = Rows::from([
            ("group".into(), vec![vec![10]]),
            ("cell0".into(), vec![vec![10, 100]]),
            ("cell1".into(), vec![vec![100, 101]]),
            ("mark0".into(), vec![vec![20, 100], vec![21, 100]]),
            ("mark1".into(), vec![vec![20, 101], vec![21, 101]]),
            ("end".into(), vec![vec![20, 101], vec![21, 101]]),
        ]);
        (spec, b, rows)
    }
    #[test]
    fn contraction_oracle_checks_identity_consumption_and_exact_multisets() {
        let (spec, b, rows) = contracted();
        assert!(spec.check(&b, &rows).is_ok());
        assert!(
            Spec {
                contract: false,
                ..spec
            }
            .check(&b, &rows)
            .is_err()
        );
        for mutation in 0..6 {
            let mut bad = rows.clone();
            match mutation {
                0 => bad.get_mut("cell0").unwrap().push(vec![10, 100]),
                1 => bad.get_mut("mark1").unwrap()[1][1] = 102,
                2 => {
                    bad.get_mut("mark0").unwrap().pop();
                }
                3 => {
                    bad.insert("step2".into(), vec![vec![101, 20]]);
                }
                4 | 5 => {
                    // Consistently corrupt every reference: graph shape still fits,
                    // but freshness/depth separation must reject the alias.
                    for tuples in bad.values_mut() {
                        for tuple in tuples {
                            for v in tuple {
                                if *v == 101 {
                                    *v = if mutation == 4 { 100 } else { 10 };
                                }
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
            assert!(spec.check(&b, &bad).is_err(), "mutation={mutation}");
        }
        let mut renamed = rows;
        for tuples in renamed.values_mut() {
            tuples.reverse();
            for tuple in tuples {
                for v in tuple {
                    if *v >= 100 {
                        *v += 1000;
                    }
                }
            }
        }
        assert!(spec.check(&b, &renamed).is_ok());
    }
    #[test]
    fn contracted_groups_cannot_share_fresh_levels() {
        let (mut spec, mut bindings, mut rows) = contracted();
        spec.groups = 2;
        bindings.extend([("G1".into(), 11), ("I1_0".into(), 22), ("I1_1".into(), 23)]);
        for (name, tuples) in rows.clone() {
            for mut tuple in tuples {
                for value in &mut tuple {
                    *value = match *value {
                        10 => 11,
                        20 => 22,
                        21 => 23,
                        v => v,
                    };
                }
                add(&mut rows, &name, tuple);
            }
        }
        // Each group's graph is locally correct, but both use nodes 100/101.
        assert!(spec.check(&bindings, &rows).is_err());
    }

    #[test]
    fn unmerged_control_requires_disjoint_produced_chains() {
        let spec = Spec {
            groups: 1,
            copies: 2,
            depth: 1,
            contract: false,
        };
        let b = contracted().1;
        let rows = Rows::from([
            ("group".into(), vec![vec![10]]),
            ("cell0".into(), vec![vec![10, 100], vec![10, 101]]),
            ("mark0".into(), vec![vec![20, 100], vec![21, 101]]),
            ("end".into(), vec![vec![20, 100], vec![21, 101]]),
        ]);
        assert!(spec.check(&b, &rows).is_ok());
        assert!(
            Spec {
                contract: true,
                ..spec
            }
            .check(&b, &rows)
            .is_err()
        );
    }
    #[test]
    fn independent_dimensions_and_orders_execute_against_the_oracle() {
        for case in CASES.split_whitespace() {
            for order in ORDERS.split_whitespace() {
                for (groups, copies, depth) in [(1, 1, 0), (1, 3, 2), (2, 1, 3), (2, 3, 3)] {
                    let w = make(case, groups, copies, depth, order).unwrap();
                    assert!(
                        crate::run_workload(
                            w,
                            case,
                            groups,
                            copies,
                            None,
                            2_000_000,
                            Duration::from_secs(10)
                        )
                        .unwrap(),
                        "{case} {order}"
                    );
                }
            }
        }
    }
    #[test]
    fn invalid_dimensions_and_orders_are_rejected() {
        for (groups, copies, depth) in
            [(0, 1, 1), (1, 0, 1), (usize::MAX, 2, 1), (1, 1, usize::MAX)]
        {
            assert!(make("fresh-contract", groups, copies, depth, "grouped").is_err());
        }
        assert!(make("fresh-contract", 1, 1, 1, "random").is_err());
        let a = make("fresh-contract", 2, 3, 2, "grouped").unwrap();
        let b = make("fresh-contract", 2, 3, 2, "interleaved").unwrap();
        assert_eq!(a.program, b.program);
        assert_ne!(a.query, b.query);
        assert_eq!(a.apps, b.apps);
        assert_eq!(a.apps, Some(26)); // 18 producer steps + 8 contractions.
    }
}
