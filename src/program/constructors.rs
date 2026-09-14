//! Whole-program applicability and source provenance for constructor lowering.
use super::{Instruction, Prepared, RulePlan};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct ConstructorChoice {
    pub key: usize,
    pub family: usize,
    pub arms: BTreeMap<usize, usize>,
    pub rejection: BTreeMap<usize, Vec<FailureConsumer>>,
}

/// A two-head terminal consumer, specialized against a proposed leading post.
/// Tests read existing identity only; local consumer slots bind to row ports.
#[derive(Debug)]
pub(crate) struct FailureConsumer {
    pub rule: usize,
    pub relation: usize,
    pub tests: Vec<(usize, ConsumerValue)>,
}
#[derive(Debug)]
pub(crate) enum ConsumerValue {
    Body(usize),
    Port(usize),
}
fn failure_consumers(
    code: &Prepared,
    terminal: &BTreeSet<usize>,
    atom: &super::Atom,
    families: &BTreeMap<usize, usize>,
) -> Vec<FailureConsumer> {
    let mut out = vec![];
    for &rule in terminal {
        let r = &code.rules[rule];
        if code.heads(r).len() != 2 {
            continue;
        }
        let Some(position) = code
            .heads(r)
            .iter()
            .position(|h| h.relation == atom.relation)
        else {
            continue;
        };
        let constructor = &code.heads(r)[position];
        // Repeated constructor-head slots impose additional existing-identity
        // tests. Such consumers continue through ordinary source execution.
        if !distinct(code.args(constructor)) {
            continue;
        }
        let other = &code.heads(r)[1 - position];
        // On surviving support the marker remains, or its consumption keeps
        // an incompatible constructor at the SAME key. Constructor admission
        // already proves those attachments persist through coalescence/merges.
        // RHS posts and descendant keys are not witnesses: they leave a gap.
        let key_port = code
            .args(other)
            .iter()
            .position(|&v| v == code.args(constructor)[0]);
        if code.rules.iter().any(|consumer| {
            !matches!(code.instructions[consumer.body], Instruction::Fail)
                && code.heads(consumer)[consumer.kept..]
                    .iter()
                    .filter(|h| h.relation == other.relation)
                    .any(|marker| {
                        !key_port.is_some_and(|port| {
                            code.heads(consumer)[..consumer.kept].iter().any(|kept| {
                                families.get(&kept.relation) == families.get(&atom.relation)
                                    && kept.relation != atom.relation
                                    && code.args(kept)[0] == code.args(marker)[port]
                            })
                        })
                    })
        }) {
            continue;
        }
        let mut slots: BTreeMap<usize, ConsumerValue> = code
            .args(constructor)
            .iter()
            .copied()
            .zip(code.args(atom).iter().copied().map(ConsumerValue::Body))
            .collect();
        let mut tests = vec![];
        for (port, &slot) in code.args(other).iter().enumerate() {
            match slots.get(&slot) {
                Some(ConsumerValue::Body(s)) => tests.push((port, ConsumerValue::Body(*s))),
                Some(ConsumerValue::Port(p)) => tests.push((port, ConsumerValue::Port(*p))),
                None => {
                    slots.insert(slot, ConsumerValue::Port(port));
                }
            }
        }
        out.push(FailureConsumer {
            rule,
            relation: other.relation,
            tests,
        });
    }
    out
}

#[derive(Clone, Debug)]
pub(crate) struct Constructors {
    pub(crate) relations: BTreeSet<usize>,
    pub(crate) families: BTreeMap<usize, usize>,
    pub(crate) rules: BTreeSet<usize>,
    pub(crate) terminal: BTreeSet<usize>,
    pub(crate) consistency: BTreeMap<usize, usize>,
    pub(crate) clashes: BTreeMap<(usize, usize), usize>,
    pub(crate) choices: BTreeMap<usize, Arc<ConstructorChoice>>,
}
fn equalities(code: &Prepared, i: usize, out: &mut Vec<(usize, usize)>) -> bool {
    match &code.instructions[i] {
        Instruction::True => true,
        Instruction::Equal(x, y) => {
            out.push(((*x).min(*y), (*x).max(*y)));
            true
        }
        Instruction::And(items) => code
            .operands(*items)
            .iter()
            .all(|&i| equalities(code, i, out)),
        _ => false,
    }
}
fn distinct(xs: &[usize]) -> bool {
    xs.iter().copied().collect::<BTreeSet<_>>().len() == xs.len()
}
fn pair(code: &Prepared, rule: &RulePlan) -> bool {
    if code.heads(rule).len() != 2 {
        return false;
    }
    let a = code.args(&code.heads(rule)[0]);
    let b = code.args(&code.heads(rule)[1]);
    !a.is_empty()
        && !b.is_empty()
        && a[0] == b[0]
        && distinct(a)
        && distinct(b)
        && a[1..].iter().all(|x| !b[1..].contains(x))
}
impl Constructors {
    /// The attachment executor implements consistency/clashes directly. A field
    /// used exactly once in every source head is a payload read, never an index
    /// key: matching only looks up already bound variables. Keep the key port,
    /// and conservatively retain every field compared anywhere in the program,
    /// including source rules removed from ordinary activation. Unknown and
    /// interfering programs never receive this representation certificate.
    /// `Some` also certifies occurrence storage, including unary relations with
    /// no payload-only fields. `None` retains the complete generic update path.
    pub(crate) fn field_indexes(&self, code: &Prepared) -> Vec<Option<Vec<bool>>> {
        let mut ports: Vec<Vec<bool>> = code
            .signatures
            .iter()
            .enumerate()
            .map(|(r, s)| {
                (0..s.arity)
                    .map(|p| p == 0 || !self.relations.contains(&r))
                    .collect()
            })
            .collect();
        for rule in &code.rules {
            let mut uses = vec![0; rule.head_variables];
            for h in code.heads(rule) {
                for &slot in code.args(h) {
                    uses[slot] += 1;
                }
            }
            for h in code.heads(rule) {
                for (port, &slot) in code.args(h).iter().enumerate() {
                    ports[h.relation][port] |= uses[slot] > 1;
                }
            }
        }
        ports
            .into_iter()
            .enumerate()
            .map(|(relation, ports)| self.relations.contains(&relation).then_some(ports))
            .collect()
    }
    /// Derive an exact finite normalization subsystem from rule structure.
    /// A direct coalescence is one legal simpagation followed by its field
    /// equalities; an incompatible overlap is one source failure. Giving these
    /// rules priority is a committed schedule choice. They create no identities
    /// or choices, and each firing consumes an occurrence on its support.
    ///
    /// Remaining consumers may keep one constructor while consuming controls.
    /// The whole-program checks exclude multiplicity/history observers and
    /// nonterminal constructor consumption, so an attachment stays valid until
    /// coalescence or terminal failure. This is compiler applicability, not a
    /// restriction on the language.
    pub fn recognize(code: &Prepared) -> Result<Self, String> {
        let mut by_relation = BTreeMap::new();
        let mut rules = BTreeSet::new();
        for (i, r) in code.rules.iter().enumerate() {
            if r.kept != 1
                || !pair(code, r)
                || code.heads(r)[0].relation != code.heads(r)[1].relation
            {
                continue;
            }
            let mut actual = vec![];
            if !equalities(code, r.body, &mut actual) {
                return Err("unsupported consistency body".into());
            }
            let mut expected: Vec<_> = code.args(&code.heads(r)[0])[1..]
                .iter()
                .zip(&code.args(&code.heads(r)[1])[1..])
                .map(|(&a, &b)| (a.min(b), a.max(b)))
                .collect();
            actual.sort();
            expected.sort();
            if actual != expected || r.head_variables != r.variables.len() {
                return Err("consistency must equate exactly corresponding fields".into());
            }
            if by_relation.insert(code.heads(r)[0].relation, i).is_some() {
                return Err("duplicate consistency definitions".into());
            }
            rules.insert(i);
        }
        if by_relation.is_empty() {
            return Err("no exact constructor consistency subsystem".into());
        }
        let relations: BTreeSet<_> = by_relation.keys().copied().collect();
        let mut clashes = BTreeMap::new();
        for (i, r) in code.rules.iter().enumerate() {
            if r.kept != 0
                || !pair(code, r)
                || !matches!(code.instructions[r.body], Instruction::Fail)
            {
                continue;
            }
            let (a, b) = (code.heads(r)[0].relation, code.heads(r)[1].relation);
            if a != b && relations.contains(&a) && relations.contains(&b) {
                if clashes.insert((a.min(b), a.max(b)), i).is_some() {
                    return Err("duplicate constructor clash".into());
                }
                rules.insert(i);
            }
        }
        // Clash-connected components are exclusive families only when every
        // pair has a source clash. Isolated consistency relations form their
        // own families; identity sharing between families remains unrestricted.
        let mut adjacency = vec![Vec::new(); code.signatures.len()];
        for &(left, right) in clashes.keys() {
            adjacency[left].push(right);
            adjacency[right].push(left);
        }
        let mut families = BTreeMap::new();
        for &relation in &relations {
            if families.contains_key(&relation) {
                continue;
            }
            let mut component = BTreeSet::from([relation]);
            let mut pending = vec![relation];
            while let Some(member) = pending.pop() {
                for &neighbor in &adjacency[member] {
                    if component.insert(neighbor) {
                        pending.push(neighbor);
                    }
                }
            }
            for &left in &component {
                for &right in
                    component.range((std::ops::Bound::Excluded(left), std::ops::Bound::Unbounded))
                {
                    if !clashes.contains_key(&(left, right)) {
                        return Err("incomplete constructor family clash coverage".into());
                    }
                }
                families.insert(left, relation);
            }
        }
        let mut terminal = BTreeSet::new();
        for (i, r) in code.rules.iter().enumerate() {
            if rules.contains(&i) {
                continue;
            }
            let uses: Vec<_> = code
                .heads(r)
                .iter()
                .enumerate()
                .filter(|(_, h)| relations.contains(&h.relation))
                .collect();
            if uses.len() > 1 {
                return Err("consumer observes multiple constructor occurrences".into());
            }
            if let Some(&(position, _)) = uses.first() {
                if r.kept == code.heads(r).len() {
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
            let items = code.operands(*items);
            if items.len() < 2 {
                continue;
            }
            let mut key = None;
            let mut family = None;
            let mut arms = BTreeMap::new();
            let mut rejection = BTreeMap::new();
            let supported = items.iter().all(|&arm| {
                let leading = match &code.instructions[arm] {
                    Instruction::And(items) => {
                        code.operands(*items).first().copied().unwrap_or(arm)
                    }
                    _ => arm,
                };
                let Instruction::Post(atom) = &code.instructions[leading] else {
                    return false;
                };
                if !relations.contains(&atom.relation) {
                    return false;
                }
                let current_family = families[&atom.relation];
                if family.is_some_and(|family| family != current_family) {
                    return false;
                }
                family = Some(current_family);
                let Some(&root) = code.args(atom).first() else {
                    return false;
                };
                if key.is_some_and(|key| key != root) {
                    return false;
                }
                key = Some(root);
                let consumers = failure_consumers(code, &terminal, atom, &families);
                if !consumers.is_empty() {
                    rejection.insert(arm, consumers);
                }
                arms.insert(atom.relation, arm).is_none()
            });
            if supported {
                choices.insert(
                    i,
                    Arc::new(ConstructorChoice {
                        key: key.unwrap(),
                        family: family.unwrap(),
                        arms,
                        rejection,
                    }),
                );
            }
        }
        Ok(Self {
            relations,
            families,
            rules,
            terminal,
            consistency: by_relation,
            clashes,
            choices,
        })
    }
}
