//! Prepared variable slots, relation signatures and reusable rule bodies.

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

pub struct Prepared {
    pub signatures: Vec<Signature>,
    pub instructions: Vec<Instruction>,
    pub rules: Vec<RulePlan>,
    /// Relation signature -> (rule index, head position) activation targets.
    pub triggers: Vec<Vec<(usize, usize)>>,
    pub query: usize,
    pub query_variables: Vec<String>,
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
    for (r, rule) in rules.iter().enumerate() {
        for (h, head) in rule.heads.iter().enumerate() {
            triggers[head.relation].push((r, h));
        }
    }
    Ok(Prepared {
        signatures: builder.signatures,
        instructions: builder.instructions,
        rules,
        triggers,
        query,
        query_variables: variables.names,
    })
}
