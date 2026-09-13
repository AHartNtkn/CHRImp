//! Prepared variable slots, relation signatures and reusable rule bodies.

pub(crate) mod constructors;
use crate::syntax::{self, Body, ParseError, Program};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Signature {
    pub name: String,
    pub arity: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    pub relation: usize,
    pub args: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Post(Atom),
    Equal(usize, usize),
    And(Vec<usize>),
    Or(Vec<usize>),
    True,
    Fail,
}

#[derive(Clone)]
pub struct RulePlan {
    pub name: Option<String>,
    /// Kept heads precede removed heads; each position needs a distinct occurrence.
    pub heads: Vec<Atom>,
    pub kept: usize,
    /// Slots before this boundary are supplied by nonbinding head matching.
    /// Remaining slots receive fresh variables for each application event.
    pub head_variables: usize,
    pub variables: Vec<String>,
    pub body: usize,
}

impl RulePlan {
    /// One head whose ordered ports impose no existing-identity comparisons.
    pub(crate) fn direct_anchor(&self) -> bool {
        self.heads.len() == 1 && self.heads[0].args.len() == self.head_variables
    }
}

/// A validated execution plan. Public consumers can inspect, but cannot mutate it.
///
/// ```compile_fail,E0616
/// use chr::{program::prepare, syntax::{parse_program, parse_query}};
/// let mut code = prepare(&parse_program("p(X) ==> q(X).").unwrap(),
///                        &parse_query("p(A)").unwrap()).unwrap();
/// code.rules.clear();
/// ```
///
/// Read accessors also preserve the plan's invariants when the caller owns it.
/// ```compile_fail,E0594
/// fn change(code: &mut chr::program::Prepared) {
///     code.rules()[0].kept = 0;
/// }
/// ```
#[derive(Clone)]
pub struct Prepared {
    pub(crate) constructors: Option<std::sync::Arc<constructors::Constructors>>,
    pub(crate) signatures: Vec<Signature>,
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) rules: Vec<RulePlan>,
    /// Relation signature -> (rule index, head position) activation targets.
    pub(crate) triggers: Vec<Vec<(usize, usize)>>,
    /// Merges can enable only heads participating in repeated-variable tests.
    pub(crate) merge_triggers: Vec<Vec<(usize, usize)>>,
    /// Whole-tuple lookup is useful only when other heads can bind every port.
    pub(crate) tuple_indexes: Vec<bool>,
    pub(crate) indexed_end: Vec<usize>,
    pub(crate) merge_indexed_end: Vec<usize>,
    pub(crate) query: usize,
    pub(crate) query_variables: Vec<String>,
}

impl Prepared {
    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn rules(&self) -> &[RulePlan] {
        &self.rules
    }

    pub fn triggers(&self) -> &[Vec<(usize, usize)>] {
        &self.triggers
    }

    pub fn query(&self) -> usize {
        self.query
    }

    pub fn query_variables(&self) -> &[String] {
        &self.query_variables
    }
}

#[derive(Default)]
struct Variables {
    names: Vec<String>,
    slots: HashMap<String, usize>,
}

impl Variables {
    fn slot(&mut self, name: &str) -> usize {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = self.names.len();
        self.names.push(name.to_owned());
        self.slots.insert(name.to_owned(), slot);
        slot
    }
}

struct Builder {
    signatures: Vec<Signature>,
    relations: HashMap<Signature, usize>,
    instructions: Vec<Instruction>,
}

impl Builder {
    fn atom(&mut self, atom: &syntax::Atom, variables: &mut Variables) -> Atom {
        let signature = Signature {
            name: atom.relation.clone(),
            arity: atom.args.len(),
        };
        let relation = *self.relations.entry(signature.clone()).or_insert_with(|| {
            let relation = self.signatures.len();
            self.signatures.push(signature);
            relation
        });
        Atom {
            relation,
            args: atom.args.iter().map(|name| variables.slot(name)).collect(),
        }
    }

    fn body(&mut self, body: &Body, variables: &mut Variables) -> usize {
        // Validation bounds nesting before preparation. Bodies are stored once;
        // runtime continuations hold instruction IDs rather than copied syntax.
        let instruction = match body {
            Body::Atom { atom } => Instruction::Post(self.atom(atom, variables)),
            Body::Equal { left, right } => {
                Instruction::Equal(variables.slot(left), variables.slot(right))
            }
            Body::And { items } => Instruction::And(
                items
                    .iter()
                    .map(|item| self.body(item, variables))
                    .collect(),
            ),
            Body::Or { items } => Instruction::Or(
                items
                    .iter()
                    .map(|item| self.body(item, variables))
                    .collect(),
            ),
            Body::True => Instruction::True,
            Body::Fail => Instruction::Fail,
        };
        let index = self.instructions.len();
        self.instructions.push(instruction);
        index
    }
}

/// Validate either editor's model, intern relation/arity signatures, assign
/// per-rule variable slots, and prepare reusable body/activation plans.
pub fn prepare(program: &Program, query: &Body) -> Result<Prepared, ParseError> {
    syntax::validate_program(program)?;
    syntax::validate_query(query)?;
    let mut builder = Builder {
        signatures: Vec::new(),
        relations: HashMap::new(),
        instructions: Vec::new(),
    };
    let mut rules = Vec::with_capacity(program.rules.len());
    for rule in &program.rules {
        let mut variables = Variables::default();
        let heads = rule
            .kept
            .iter()
            .chain(&rule.removed)
            .map(|atom| builder.atom(atom, &mut variables))
            .collect();
        let head_variables = variables.names.len();
        let body = builder.body(&rule.body, &mut variables);
        rules.push(RulePlan {
            name: rule.name.clone(),
            heads,
            kept: rule.kept.len(),
            head_variables,
            variables: variables.names,
            body,
        });
    }
    let mut variables = Variables::default();
    let query = builder.body(query, &mut variables);
    let mut triggers = vec![Vec::new(); builder.signatures.len()];
    let mut merge_triggers = vec![Vec::new(); builder.signatures.len()];
    let mut tuple_indexes = vec![false; builder.signatures.len()];
    for (r, rule) in rules.iter().enumerate() {
        let mut uses = vec![0; rule.head_variables];
        for head in &rule.heads {
            for &slot in &head.args {
                uses[slot] += 1;
            }
        }
        let mut local_uses = vec![0; rule.head_variables];
        for (h, head) in rule.heads.iter().enumerate() {
            for &slot in &head.args {
                local_uses[slot] += 1;
            }
            if head.args.len() >= 2 && head.args.iter().all(|&slot| uses[slot] > local_uses[slot]) {
                tuple_indexes[head.relation] = true;
            }
            for &slot in &head.args {
                local_uses[slot] = 0;
            }
            triggers[head.relation].push((r, h));
            if head.args.iter().any(|&slot| uses[slot] > 1) {
                merge_triggers[head.relation].push((r, h));
            }
        }
    }
    let mut code = Prepared {
        constructors: None,
        signatures: builder.signatures,
        instructions: builder.instructions,
        rules,
        triggers,
        merge_triggers,
        tuple_indexes,
        indexed_end: Vec::new(),
        merge_indexed_end: Vec::new(),
        query,
        query_variables: variables.names,
    };
    if let Ok(plan) = constructors::Constructors::recognize(&code) {
        plan.lower_triggers(&mut code);
        code.constructors = Some(std::sync::Arc::new(plan));
    }
    let indexed_end = |targets: &Vec<Vec<(usize, usize)>>| {
        targets
            .iter()
            .map(|ts| {
                ts.iter()
                    .rposition(|(r, _)| !code.rules[*r].direct_anchor())
                    .map_or(0, |i| i + 1)
            })
            .collect()
    };
    code.indexed_end = indexed_end(&code.triggers);
    code.merge_indexed_end = indexed_end(&code.merge_triggers);
    Ok(code)
}

#[cfg(test)]
mod tuple_plan_tests {
    use super::*;
    #[test]
    fn tuple_indexes_require_every_variable_from_other_heads() {
        let p = syntax::parse_program(
            "single(X,X) ==> output(X,X). \
            left(X),right(Y),both(X,Y) ==> answer(X,Y). \
            one(X),partial(X,Y) ==> true. \
            duplicate(X,X),one(X) ==> true.",
        )
        .unwrap();
        let code = prepare(&p, &syntax::parse_query("query_only(A,B)").unwrap()).unwrap();
        let enabled: Vec<_> = code
            .signatures
            .iter()
            .zip(&code.tuple_indexes)
            .filter(|(_, on)| **on)
            .map(|(s, _)| s.name.as_str())
            .collect();
        assert_eq!(enabled, ["both", "duplicate"]);
    }
}
