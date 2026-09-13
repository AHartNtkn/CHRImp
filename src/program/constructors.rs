//! Experimental whole-program derivation of constructor consistency.
use super::{Instruction, Prepared, RulePlan};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct ConstructorChoice {
    pub key: usize,
    pub arms: BTreeMap<usize, usize>,
}

#[derive(Clone, Debug)]
pub struct Constructors {
    pub(crate) relations: BTreeSet<usize>,
    pub(crate) rules: BTreeSet<usize>,
    pub(crate) terminal: BTreeSet<usize>,
    pub(crate) choices: BTreeMap<usize, Arc<ConstructorChoice>>,
}
fn equalities(code: &Prepared, i: usize, out: &mut Vec<(usize, usize)>) -> bool {
    match &code.instructions[i] {
        Instruction::True => true,
        Instruction::Equal(x, y) => {
            out.push(((*x).min(*y), (*x).max(*y)));
            true
        }
        Instruction::And(items) => items.iter().all(|&i| equalities(code, i, out)),
        _ => false,
    }
}
fn distinct(xs: &[usize]) -> bool {
    xs.iter().copied().collect::<BTreeSet<_>>().len() == xs.len()
}
fn pair(rule: &RulePlan) -> bool {
    if rule.heads.len() != 2 {
        return false;
    }
    let a = &rule.heads[0].args;
    let b = &rule.heads[1].args;
    !a.is_empty()
        && !b.is_empty()
        && a[0] == b[0]
        && distinct(a)
        && distinct(b)
        && a[1..].iter().all(|x| !b[1..].contains(x))
}
impl Constructors {
    /// Derive an exact finite normalization subsystem from rule structure.
    /// A direct coalescence is one legal simpagation followed by its field
    /// equalities; an incompatible overlap is one source failure. Giving these
    /// rules priority is a committed schedule choice. They create no identities
    /// or choices, and each firing consumes an occurrence on its support.
    ///
    /// Remaining consumers may keep one constructor while consuming controls.
    /// The whole-program checks exclude multiplicity/history observers and
    /// nonterminal constructor consumption, so an attachment stays valid until
    /// coalescence or terminal failure. This is experimental admission, not a
    /// restriction on the language.
    pub fn recognize(code: &Prepared) -> Result<Self, String> {
        let mut by_relation = BTreeMap::new();
        let mut rules = BTreeSet::new();
        for (i, r) in code.rules.iter().enumerate() {
            if r.kept != 1 || !pair(r) || r.heads[0].relation != r.heads[1].relation {
                continue;
            }
            let mut actual = vec![];
            if !equalities(code, r.body, &mut actual) {
                return Err("unsupported consistency body".into());
            }
            let mut expected: Vec<_> = r.heads[0].args[1..]
                .iter()
                .zip(&r.heads[1].args[1..])
                .map(|(&a, &b)| (a.min(b), a.max(b)))
                .collect();
            actual.sort();
            expected.sort();
            if actual != expected || r.head_variables != r.variables.len() {
                return Err("consistency must equate exactly corresponding fields".into());
            }
            if by_relation.insert(r.heads[0].relation, i).is_some() {
                return Err("duplicate consistency definitions".into());
            }
            rules.insert(i);
        }
        if by_relation.is_empty() {
            return Err("no exact constructor consistency subsystem".into());
        }
        let relations: BTreeSet<_> = by_relation.keys().copied().collect();
        let mut clashes = BTreeSet::new();
        for (i, r) in code.rules.iter().enumerate() {
            if r.kept != 0 || !pair(r) || !matches!(code.instructions[r.body], Instruction::Fail) {
                continue;
            }
            let (a, b) = (r.heads[0].relation, r.heads[1].relation);
            if a != b && relations.contains(&a) && relations.contains(&b) {
                if !clashes.insert((a.min(b), a.max(b))) {
                    return Err("duplicate constructor clash".into());
                }
                rules.insert(i);
            }
        }
        if clashes.len() != relations.len() * (relations.len() - 1) / 2 {
            return Err("incomplete constructor clash coverage".into());
        }
        let mut terminal = BTreeSet::new();
        for (i, r) in code.rules.iter().enumerate() {
            if rules.contains(&i) {
                continue;
            }
            let uses: Vec<_> = r
                .heads
                .iter()
                .enumerate()
                .filter(|(_, h)| relations.contains(&h.relation))
                .collect();
            if uses.len() > 1 {
                return Err("consumer observes multiple constructor occurrences".into());
            }
            if let Some(&(position, _)) = uses.first() {
                if r.kept == r.heads.len() {
                    return Err("propagation observes constructor occurrence history".into());
                }
                if position >= r.kept {
                    if !matches!(code.instructions[r.body], Instruction::Fail) {
                        return Err(
                            "consumer changes constructor liveness without terminal failure".into(),
                        );
                    }
                    terminal.insert(i);
                }
            }
        }
        // Distinct leading tags at one syntactic key have at most one viable
        // arm on known support. The admitted clash rules justify finite failure
        // of the others. Keep each complete source arm: its fields can impose
        // additional equalities/failure, and its occurrence must still be posted.
        // Unsupported shapes (including repeated tags) stay ordinary disjunctions.
        let mut choices = BTreeMap::new();
        for (i, instruction) in code.instructions.iter().enumerate() {
            let Instruction::Or(items) = instruction else {
                continue;
            };
            if items.len() < 2 {
                continue;
            }
            let mut key = None;
            let mut arms = BTreeMap::new();
            let supported = items.iter().all(|&arm| {
                let leading = match &code.instructions[arm] {
                    Instruction::And(items) => items.first().copied().unwrap_or(arm),
                    _ => arm,
                };
                let Instruction::Post(atom) = &code.instructions[leading] else {
                    return false;
                };
                if !relations.contains(&atom.relation) {
                    return false;
                }
                let Some(&root) = atom.args.first() else {
                    return false;
                };
                if key.is_some_and(|key| key != root) {
                    return false;
                }
                key = Some(root);
                arms.insert(atom.relation, arm).is_none()
            });
            if supported {
                choices.insert(
                    i,
                    Arc::new(ConstructorChoice {
                        key: key.unwrap(),
                        arms,
                    }),
                );
            }
        }
        Ok(Self {
            relations,
            rules,
            terminal,
            choices,
        })
    }
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
    pub(crate) fn consumer_code(&self, code: &Prepared) -> Prepared {
        let mut result = code.clone();
        for ts in result
            .triggers
            .iter_mut()
            .chain(result.merge_triggers.iter_mut())
        {
            ts.retain(|(r, _)| !self.rules.contains(r));
        }
        let ends = |ts: &Vec<Vec<(usize, usize)>>| {
            ts.iter()
                .map(|ts| {
                    ts.iter()
                        .rposition(|(r, _)| !result.rules[*r].direct_anchor())
                        .map_or(0, |i| i + 1)
                })
                .collect()
        };
        result.indexed_end = ends(&result.triggers);
        result.merge_indexed_end = ends(&result.merge_triggers);
        result
    }
}
