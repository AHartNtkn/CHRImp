//! Generated inputs and semantic oracles. No engine scheduling assumptions in answer checks.
use super::{Counts, consume_tuple, expected_tuples, inputs, workload};
use std::collections::{BTreeMap, HashSet};

pub const CASES: &str = "bits-chain bits-star bits-chain-delayed bits-star-delayed answers alias-consume fair-loop fair-grow stream-fail rejected3 multiport multiport-hit multiport-probes-first simpagation repeated-alias raw-probes reach-chain prepare fanout";
pub type Rows = BTreeMap<String, Vec<Vec<u64>>>;
pub type Bindings = BTreeMap<String, u64>;

#[derive(Clone, Copy, Debug)]
pub enum Goal {
    Complete,
    Answers(usize),
    Applications(u64),
}

pub enum Oracle {
    Legacy(String, usize),
    Bits { n: usize, star: bool },
    Answers(usize),
    Alias(usize),
    Fair,
    Stream,
    Rejected(usize),
    Multiport { n: usize, probes: usize, hit: bool },
    Cells { copies: usize, depth: usize },
    AliasProbes { n: usize, probes: usize },
    Reach(usize),
    Fanout(usize),
    Prepare,
}
pub struct Workload {
    pub program: String,
    pub query: String,
    pub oracle: Oracle,
    pub answers: Option<usize>,
    pub apps: Option<u64>,
    pub goal: Goal,
    pub expected: Vec<Counts>,
}

pub fn make(case: &str, n: usize, rows: usize) -> Result<Workload, String> {
    let mut w = Workload {
        program: String::new(),
        query: String::new(),
        oracle: Oracle::Stream,
        answers: Some(1),
        apps: None,
        goal: Goal::Complete,
        expected: vec![],
    };
    match case {
        "bits-chain" | "bits-star" | "bits-chain-delayed" | "bits-star-delayed" => {
            let star = case.contains("star");
            w.oracle = Oracle::Bits { n, star };
            w.answers = Some(2);
            w.program = "enabled(),link(I,J),zero(I),one(J) ==> fail. enabled(),link(I,J),one(I),zero(J) ==> fail.".into();
            let mut query: Vec<_> = (0..n).map(|i| format!("(zero(V{i});one(V{i}))")).collect();
            query.extend((1..n).map(|i| format!("link(V{},V{i})", if star { 0 } else { i - 1 })));
            if case.ends_with("delayed") {
                // A causal 8n-application gate, not presumed left-to-right conjunction execution.
                for i in 0..8 * n {
                    w.program += &format!("delay{i}() <=> delay{}().", i + 1);
                }
                w.program += &format!("delay{}() <=> enabled().", 8 * n);
                query.push("delay0()".into());
            } else {
                query.push("enabled()".into());
            }
            w.query = query.join(",");
        }
        "answers" => {
            w.oracle = Oracle::Answers(rows);
            w.answers = Some(n);
            w.apps = Some(0);
            w.query = format!("({})", vec!["true"; n].join(";"));
            if rows > 0 {
                w.query += &format!(",{}", inputs("p", rows));
            }
        }
        "alias-consume" => {
            w.oracle = Oracle::Alias(n);
            w.answers = Some(n + 1);
            w.apps = Some((2 * n) as u64);
            w.program = "merge(X,Y) <=> X=Y. p(X) \\ q(X) <=> hit(X).".into();
            let mut query = vec!["p(A)".into(), inputs("q", n)];
            let mut arms: Vec<_> = (0..n).map(|i| format!("merge(A,V{i})")).collect();
            arms.push("true".into());
            query.push(format!("({})", arms.join(";")));
            w.query = query.join(",");
        }
        "fair-loop" | "fair-grow" => {
            w.oracle = Oracle::Fair;
            w.goal = Goal::Answers(1);
            w.program = if case == "fair-grow" {
                "loop(X) <=> (loop(X);loop(X))."
            } else {
                "loop(X) <=> loop(X)."
            }
            .into();
            for i in 0..rows {
                w.program += &format!("finite{i}() <=> finite{}().", i + 1);
            }
            w.program += &format!("finite{rows}() <=> answer().");
            let mut arms: Vec<_> = (0..n).map(|i| format!("loop(V{i})")).collect();
            arms.push("finite0()".into());
            w.query = arms.join(";");
        }
        "stream-fail" => {
            w.answers = Some(0);
            w.goal = Goal::Applications(n as u64);
            w.program = "loop() <=> (fail;loop()).".into();
            w.query = "loop()".into();
        }
        "rejected3" => {
            w.oracle = Oracle::Rejected(n);
            w.apps = Some(0);
            w.program = "p(X,Y),q(Y,Z),r(Z,X) ==> hit(X,Y,Z).".into();
            w.query = (0..n)
                .map(|i| format!("p(A{i},H),q(H,B{i}),r(B{i},C{i})"))
                .collect::<Vec<_>>()
                .join(",");
        }
        "multiport" | "multiport-hit" | "multiport-probes-first" => {
            let hit = case.ends_with("-hit");
            w.oracle = Oracle::Multiport {
                n,
                probes: rows,
                hit,
            };
            w.apps = Some(if hit { rows as u64 } else { 0 });
            w.program = "probe(X,Y,I),row(X,Y) ==> hit(I).".into();
            let mut query: Vec<_> = (0..n).map(|i| format!("row(A,C{i}),row(D{i},B)")).collect();
            query.extend((0..rows).map(|i| format!("probe(A,B,I{i})")));
            if hit {
                query.push("row(A,B)".into());
            }
            if case == "multiport-probes-first" {
                query.rotate_right(rows);
            }
            w.query = query.join(",");
        }
        "simpagation" => {
            if rows == 0 {
                return Err("simpagation requires positive --rows (depth)".into());
            }
            w.oracle = Oracle::Cells {
                copies: n,
                depth: rows,
            };
            w.apps = Some((rows * (n - 1)) as u64);
            w.program = "cell(K,V) \\ cell(K,W) <=> V=W.".into();
            w.query = (0..n)
                .flat_map(|i| {
                    (0..rows).map(move |d| {
                        let key = if d == 0 {
                            "Root".into()
                        } else {
                            format!("V{i}_{}", d - 1)
                        };
                        format!("cell({key},V{i}_{d})")
                    })
                })
                .collect::<Vec<_>>()
                .join(",");
        }
        "repeated-alias" | "raw-probes" => {
            let n = if case == "raw-probes" { 0 } else { n };
            w.oracle = Oracle::AliasProbes { n, probes: rows };
            w.apps = Some(rows as u64);
            w.program = "probe(X,I),target(X) ==> hit(I).".into();
            let mut query: Vec<_> = (0..n).map(|i| format!("V{i}=V{}", i + 1)).collect();
            query.push(format!("target(V{n})"));
            query.extend((0..rows).map(|i| format!("probe(V0,I{i})")));
            w.query = query.join(",");
        }
        "reach-chain" => {
            w.oracle = Oracle::Reach(n);
            w.apps = Some((n * (n + 1) / 2) as u64);
            w.program = "edge(X,Y) ==> reach(X,Y). reach(X,Y),edge(Y,Z) ==> reach(X,Z).".into();
            w.query = (0..n)
                .map(|i| format!("edge(V{i},V{})", i + 1))
                .collect::<Vec<_>>()
                .join(",");
        }
        "prepare" | "fanout" => {
            let fanout = case == "fanout";
            w.oracle = if fanout {
                Oracle::Fanout(n)
            } else {
                Oracle::Prepare
            };
            w.apps = Some(if fanout { n as u64 } else { 0 });
            w.query = "p(A)".into();
            for i in 0..n {
                let head = if fanout {
                    "p".into()
                } else {
                    format!("unused{i}")
                };
                w.program += &format!("{head}(X) ==> result{i}(X).");
            }
        }
        _ => {
            let (p, q, apps, expected) = workload(case, n)?;
            w.program = p;
            w.query = q;
            w.apps = Some(apps as u64);
            w.answers = Some(expected.len());
            w.expected = expected;
            w.oracle = Oracle::Legacy(case.into(), n);
        }
    }
    Ok(w)
}

fn add(rows: &mut Rows, name: &str, ports: Vec<u64>) {
    rows.entry(name.into()).or_default().push(ports);
}
fn distinct(b: &Bindings) -> Result<(), String> {
    if b.values().collect::<HashSet<_>>().len() != b.len() {
        Err("unexpected variable alias".into())
    } else {
        Ok(())
    }
}
impl Oracle {
    /// Returns a semantic answer key when each allowed answer must occur once.
    pub fn check(
        &self,
        b: &Bindings,
        actual: &Rows,
        expected_counts: &mut Vec<Counts>,
    ) -> Result<Option<u64>, String> {
        let v = |name: &str| {
            b.get(name)
                .copied()
                .ok_or_else(|| format!("missing binding {name}"))
        };
        let mut expected = Rows::new();
        let mut key = None;
        match self {
            Self::Legacy(case, n) => {
                let counts: Counts = actual.iter().map(|(k, v)| (k.clone(), v.len())).collect();
                let i = expected_counts
                    .iter()
                    .position(|want| *want == counts)
                    .ok_or("unexpected answer counts")?;
                let mut tuples = expected_tuples(case, *n, b)?;
                for (name, rows) in actual {
                    for row in rows {
                        consume_tuple(&mut tuples, name, row)?;
                    }
                }
                if actual.keys().any(|name| !tuples[name].is_empty()) {
                    return Err("missing ordered tuples".into());
                }
                expected_counts.swap_remove(i);
                return Ok(None);
            }
            Self::Bits { n, star } => {
                distinct(b)?;
                let bit = if actual.contains_key("zero") {
                    "zero"
                } else {
                    "one"
                };
                key = Some(u64::from(bit == "one"));
                for i in 0..*n {
                    add(&mut expected, bit, vec![v(&format!("V{i}"))?]);
                }
                for i in 1..*n {
                    add(
                        &mut expected,
                        "link",
                        vec![
                            v(&format!("V{}", if *star { 0 } else { i - 1 }))?,
                            v(&format!("V{i}"))?,
                        ],
                    );
                }
                add(&mut expected, "enabled", vec![]);
            }
            Self::Answers(n) => {
                distinct(b)?;
                for i in 0..*n {
                    add(&mut expected, "p", vec![v(&format!("V{i}"))?]);
                }
            }
            Self::Alias(n) => {
                let a = v("A")?;
                let vars = (0..*n)
                    .map(|i| v(&format!("V{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                if vars.iter().collect::<HashSet<_>>().len() != *n {
                    return Err("sibling variables merged".into());
                }
                let selected = vars.iter().position(|x| *x == a);
                key = Some(selected.map_or(*n, |i| i) as u64);
                add(&mut expected, "p", vec![a]);
                for (i, &x) in vars.iter().enumerate() {
                    if Some(i) != selected {
                        add(&mut expected, "q", vec![x]);
                    }
                }
                if selected.is_some() {
                    add(&mut expected, "hit", vec![a]);
                }
            }
            Self::Fair => {
                distinct(b)?;
                add(&mut expected, "answer", vec![]);
                key = Some(0);
            }
            Self::Stream => return Err("continuing stream emitted an answer".into()),
            Self::Rejected(n) => {
                distinct(b)?;
                for i in 0..*n {
                    add(&mut expected, "p", vec![v(&format!("A{i}"))?, v("H")?]);
                    add(&mut expected, "q", vec![v("H")?, v(&format!("B{i}"))?]);
                    add(
                        &mut expected,
                        "r",
                        vec![v(&format!("B{i}"))?, v(&format!("C{i}"))?],
                    );
                }
            }
            Self::Multiport { n, probes, hit } => {
                distinct(b)?;
                for i in 0..*n {
                    add(&mut expected, "row", vec![v("A")?, v(&format!("C{i}"))?]);
                    add(&mut expected, "row", vec![v(&format!("D{i}"))?, v("B")?]);
                }
                if *hit {
                    add(&mut expected, "row", vec![v("A")?, v("B")?]);
                }
                for i in 0..*probes {
                    let id = v(&format!("I{i}"))?;
                    add(&mut expected, "probe", vec![v("A")?, v("B")?, id]);
                    if *hit {
                        add(&mut expected, "hit", vec![id]);
                    }
                }
            }
            Self::Cells { copies, depth } => {
                let mut levels = HashSet::from([v("Root")?]);
                let mut previous = v("Root")?;
                for d in 0..*depth {
                    let x = v(&format!("V0_{d}"))?;
                    if !levels.insert(x) {
                        return Err("chain depths aliased".into());
                    }
                    for i in 1..*copies {
                        if v(&format!("V{i}_{d}"))? != x {
                            return Err("corresponding chain levels differ".into());
                        }
                    }
                    add(&mut expected, "cell", vec![previous, x]);
                    previous = x;
                }
            }
            Self::AliasProbes { n, probes } => {
                let x = v("V0")?;
                let mut ids = HashSet::from([x]);
                for i in 1..=*n {
                    if v(&format!("V{i}"))? != x {
                        return Err("alias chain incomplete".into());
                    }
                }
                add(&mut expected, "target", vec![x]);
                for i in 0..*probes {
                    let id = v(&format!("I{i}"))?;
                    if !ids.insert(id) {
                        return Err("probe identities aliased".into());
                    }
                    add(&mut expected, "probe", vec![x, id]);
                    add(&mut expected, "hit", vec![id]);
                }
            }
            Self::Reach(n) => {
                distinct(b)?;
                for i in 0..*n {
                    add(
                        &mut expected,
                        "edge",
                        vec![v(&format!("V{i}"))?, v(&format!("V{}", i + 1))?],
                    );
                    for j in i + 1..=*n {
                        add(
                            &mut expected,
                            "reach",
                            vec![v(&format!("V{i}"))?, v(&format!("V{j}"))?],
                        );
                    }
                }
            }
            Self::Prepare | Self::Fanout(_) => {
                distinct(b)?;
                add(&mut expected, "p", vec![v("A")?]);
                if let Self::Fanout(n) = self {
                    for i in 0..*n {
                        add(&mut expected, &format!("result{i}"), vec![v("A")?]);
                    }
                }
            }
        }
        for rows in expected.values_mut() {
            rows.sort();
        }
        if &expected != actual {
            return Err(format!(
                "incorrect residual graph: expected {expected:?}, got {actual:?}"
            ));
        }
        Ok(key)
    }
}
