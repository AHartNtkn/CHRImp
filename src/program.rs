//! Prepared variable slots, relation signatures and reusable rule bodies.

pub(crate) mod constructors;
use crate::syntax::{self, Body, ParseError, Program};
use std::collections::HashMap;

/// A contiguous range in a Prepared arena. Offsets use the native index width;
/// packing does not impose a smaller language/program size limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    start: usize,
    len: usize,
}
impl Span {
    pub fn len(self) -> usize {
        self.len
    }
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    fn slice<T>(self, arena: &[T]) -> &[T] {
        &arena[self.start..][..self.len]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Signature {
    pub name: String,
    pub arity: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    pub relation: usize,
    pub args: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Post(Atom),
    Equal(usize, usize),
    And(Span),
    Or(Span),
    True,
    Fail,
}

#[derive(Clone)]
pub struct RulePlan {
    pub name: Option<String>,
    /// Kept heads precede removed heads; each position needs a distinct occurrence.
    pub heads: Span,
    pub kept: usize,
    /// Slots before this boundary are supplied by nonbinding head matching.
    /// Remaining slots receive fresh variables for each application event.
    pub head_variables: usize,
    pub variables: Vec<String>,
    pub body: usize,
}

impl RulePlan {
    /// One head whose ordered ports impose no existing-identity comparisons.
    pub(crate) fn direct_anchor(&self, code: &Prepared) -> bool {
        self.heads.len() == 1 && code.heads(self)[0].args.len() == self.head_variables
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
    #[cfg(feature = "diagnostics")]
    preparation_diagnostics: PreparationDiagnostics,
    pub(crate) constructors: Option<std::sync::Arc<constructors::Constructors>>,
    pub(crate) graph_updates: Option<std::sync::Arc<[Option<crate::graph::UpdatePlan>]>>,
    pub(crate) signatures: Vec<Signature>,
    pub(crate) instructions: Box<[Instruction]>,
    operands: Box<[usize]>,
    heads: Box<[Atom]>,
    pub(crate) rules: Box<[RulePlan]>,
    /// Relation signature -> (rule index, head position) activation targets.
    pub(crate) triggers: Vec<Span>,
    /// Merges can enable only heads participating in repeated-variable tests.
    pub(crate) merge_triggers: Vec<Span>,
    trigger_records: Vec<(usize, usize)>,
    /// Whole-tuple lookup is useful only when other heads can bind every port.
    pub(crate) tuple_indexes: Vec<bool>,
    pub(crate) indexed_end: Vec<usize>,
    pub(crate) merge_indexed_end: Vec<usize>,
    pub(crate) query: usize,
    pub(crate) query_variables: Vec<String>,
}

#[cfg(feature = "diagnostics")]
#[derive(Clone, Default, serde::Serialize)]
pub struct PreparationDiagnostics {
    pub constructor_recognition_ns: u128,
    pub field_certificate_and_codegen_ns: u128,
    pub specialized_relations: usize,
    pub omitted_fields: usize,
    pub update_opcodes: usize,
    /// Requested plan buffers, excluding Arc header and allocator overhead.
    pub plan_payload_bytes: usize,
}

impl Prepared {
    /// Requested storage of the changed representation, including all buffer
    /// capacity, padding, and the entire Prepared header. Excludes unchanged
    /// signature/name buffers and constructor/update plans; allocator phase
    /// accounting separately measures the complete retained preparation heap.
    #[cfg(feature = "diagnostics")]
    pub fn representation_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(self.instructions.as_ref())
            + std::mem::size_of_val(self.operands.as_ref())
            + std::mem::size_of_val(self.heads.as_ref())
            + std::mem::size_of_val(self.rules.as_ref())
            + self.triggers.capacity() * std::mem::size_of::<Span>()
            + self.merge_triggers.capacity() * std::mem::size_of::<Span>()
            + self.trigger_records.capacity() * std::mem::size_of::<(usize, usize)>()
    }

    #[cfg(feature = "diagnostics")]
    pub fn preparation_diagnostics(&self) -> &PreparationDiagnostics {
        &self.preparation_diagnostics
    }
    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn rules(&self) -> &[RulePlan] {
        &self.rules
    }

    pub fn operands(&self, span: Span) -> &[usize] {
        span.slice(&self.operands)
    }

    pub fn args(&self, atom: &Atom) -> &[usize] {
        self.operands(atom.args)
    }

    pub fn heads(&self, rule: &RulePlan) -> &[Atom] {
        rule.heads.slice(&self.heads)
    }

    pub fn triggers(&self) -> impl ExactSizeIterator<Item = &[(usize, usize)]> {
        self.triggers.iter().map(|&span| self.targets(span))
    }

    pub(crate) fn targets(&self, span: Span) -> &[(usize, usize)] {
        span.slice(&self.trigger_records)
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
    operands: Vec<usize>,
    heads: Vec<Atom>,
}

#[derive(Default)]
struct Layout {
    instructions: usize,
    operands: usize,
    heads: usize,
}
impl Layout {
    fn body(&mut self, body: &Body) {
        self.instructions += 1;
        match body {
            Body::Atom { atom } => self.operands += atom.args.len(),
            Body::And { items } | Body::Or { items } => {
                self.operands += items.len();
                for item in items {
                    self.body(item);
                }
            }
            _ => {}
        }
    }
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
        let args = Span {
            start: self.operands.len(),
            len: atom.args.len(),
        };
        self.operands
            .extend(atom.args.iter().map(|name| variables.slot(name)));
        Atom { relation, args }
    }

    fn body(&mut self, body: &Body, variables: &mut Variables) -> usize {
        // Validation bounds nesting before preparation. Bodies are stored once;
        // runtime continuations hold instruction IDs rather than copied syntax.
        let instruction = match body {
            Body::Atom { atom } => Instruction::Post(self.atom(atom, variables)),
            Body::Equal { left, right } => {
                Instruction::Equal(variables.slot(left), variables.slot(right))
            }
            Body::And { items } | Body::Or { items } => {
                // Reserve the parent's slots before recursively appending children.
                // No temporary child vector and no references survive arena growth.
                let span = Span {
                    start: self.operands.len(),
                    len: items.len(),
                };
                self.operands.resize(span.start + span.len, 0);
                for (offset, item) in items.iter().enumerate() {
                    let child = self.body(item, variables);
                    self.operands[span.start + offset] = child;
                }
                if matches!(body, Body::And { .. }) {
                    Instruction::And(span)
                } else {
                    Instruction::Or(span)
                }
            }
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
    // One bounded-depth sizing walk replaces geometric arena growth/copying.
    let mut layout = Layout::default();
    for rule in &program.rules {
        for atom in rule.kept.iter().chain(&rule.removed) {
            layout.heads += 1;
            layout.operands += atom.args.len();
        }
        layout.body(&rule.body);
    }
    layout.body(query);
    let mut builder = Builder {
        signatures: Vec::new(),
        relations: HashMap::new(),
        instructions: Vec::with_capacity(layout.instructions),
        operands: Vec::with_capacity(layout.operands),
        heads: Vec::with_capacity(layout.heads),
    };
    let mut rules = Vec::with_capacity(program.rules.len());
    for rule in &program.rules {
        let mut variables = Variables::default();
        let heads = Span {
            start: builder.heads.len(),
            len: rule.kept.len() + rule.removed.len(),
        };
        for atom in rule.kept.iter().chain(&rule.removed) {
            let atom = builder.atom(atom, &mut variables);
            builder.heads.push(atom);
        }
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
    let mut merge_heads = vec![false; builder.heads.len()];
    let mut tuple_indexes = vec![false; builder.signatures.len()];
    for rule in &rules {
        let mut uses = vec![0; rule.head_variables];
        for head in rule.heads.slice(&builder.heads) {
            for &slot in head.args.slice(&builder.operands) {
                uses[slot] += 1;
            }
        }
        let mut local_uses = vec![0; rule.head_variables];
        for (h, head) in rule.heads.slice(&builder.heads).iter().enumerate() {
            let args = head.args.slice(&builder.operands);
            for &slot in args {
                local_uses[slot] += 1;
            }
            if args.len() >= 2 && args.iter().all(|&slot| uses[slot] > local_uses[slot]) {
                tuple_indexes[head.relation] = true;
            }
            for &slot in args {
                local_uses[slot] = 0;
            }
            merge_heads[rule.heads.start + h] = args.iter().any(|&slot| uses[slot] > 1);
        }
    }
    let mut code = Prepared {
        #[cfg(feature = "diagnostics")]
        preparation_diagnostics: PreparationDiagnostics::default(),
        constructors: None,
        graph_updates: None,
        signatures: builder.signatures,
        instructions: builder.instructions.into_boxed_slice(),
        operands: builder.operands.into_boxed_slice(),
        heads: builder.heads.into_boxed_slice(),
        rules: rules.into_boxed_slice(),
        triggers: Vec::new(),
        merge_triggers: Vec::new(),
        trigger_records: Vec::new(),
        tuple_indexes,
        indexed_end: Vec::new(),
        merge_indexed_end: Vec::new(),
        query,
        query_variables: variables.names,
    };
    #[cfg(feature = "diagnostics")]
    let start = std::time::Instant::now();
    if let Ok(plan) = constructors::Constructors::recognize(&code) {
        code.constructors = Some(std::sync::Arc::new(plan));
    }
    #[cfg(feature = "diagnostics")]
    {
        code.preparation_diagnostics.constructor_recognition_ns = start.elapsed().as_nanos();
    }
    #[cfg(feature = "diagnostics")]
    let start = std::time::Instant::now();
    code.graph_updates = crate::graph::UpdatePlan::prepare(
        &code.signatures,
        &code.tuple_indexes,
        code.constructors
            .as_ref()
            .map(|plan| plan.field_indexes(&code)),
    );
    #[cfg(feature = "diagnostics")]
    {
        let d = &mut code.preparation_diagnostics;
        d.field_certificate_and_codegen_ns = start.elapsed().as_nanos();
        (
            d.specialized_relations,
            d.omitted_fields,
            d.update_opcodes,
            d.plan_payload_bytes,
        ) = crate::graph::UpdatePlan::statistics(code.graph_updates.as_ref());
    }
    // Count, prefix, and fill directly in final storage. Preserve source rule /
    // ordered-head activation order, including duplicate heads and merge tests.
    let relations = code.signatures.len();
    code.triggers = vec![Span { start: 0, len: 0 }; relations];
    code.merge_triggers = code.triggers.clone();
    code.indexed_end = vec![0; relations];
    code.merge_indexed_end = vec![0; relations];
    for (r, rule) in code.rules.iter().enumerate() {
        if code
            .constructors
            .as_ref()
            .is_some_and(|c| c.rules.contains(&r))
        {
            continue;
        }
        for (h, head) in rule.heads.slice(&code.heads).iter().enumerate() {
            code.triggers[head.relation].len += 1;
            code.merge_triggers[head.relation].len +=
                usize::from(merge_heads[rule.heads.start + h]);
        }
    }
    let mut total = 0;
    for span in code.triggers.iter_mut().chain(&mut code.merge_triggers) {
        span.start = total;
        total += span.len;
        span.len = 0;
    }
    code.trigger_records = vec![(0, 0); total];
    for (r, rule) in code.rules.iter().enumerate() {
        if code
            .constructors
            .as_ref()
            .is_some_and(|c| c.rules.contains(&r))
        {
            continue;
        }
        let indexed = !rule.direct_anchor(&code);
        for (h, head) in rule.heads.slice(&code.heads).iter().enumerate() {
            let span = &mut code.triggers[head.relation];
            code.trigger_records[span.start + span.len] = (r, h);
            span.len += 1;
            if indexed {
                code.indexed_end[head.relation] = span.len;
            }
            if merge_heads[rule.heads.start + h] {
                let span = &mut code.merge_triggers[head.relation];
                code.trigger_records[span.start + span.len] = (r, h);
                span.len += 1;
                if indexed {
                    code.merge_indexed_end[head.relation] = span.len;
                }
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tuple_plan_tests {
    use super::*;

    fn decode_atom(code: &Prepared, atom: &Atom, names: &[String]) -> syntax::Atom {
        syntax::Atom {
            relation: code.signatures[atom.relation].name.clone(),
            args: code.args(atom).iter().map(|&s| names[s].clone()).collect(),
        }
    }

    fn decode(code: &Prepared, id: usize, names: &[String], visited: &mut [bool]) -> Body {
        assert!(
            !std::mem::replace(&mut visited[id], true),
            "distinct source positions must remain distinct instructions"
        );
        match &code.instructions[id] {
            Instruction::Post(atom) => Body::Atom {
                atom: decode_atom(code, atom, names),
            },
            Instruction::Equal(x, y) => Body::Equal {
                left: names[*x].clone(),
                right: names[*y].clone(),
            },
            Instruction::And(span) | Instruction::Or(span) => {
                let items = code
                    .operands(*span)
                    .iter()
                    .map(|&child| {
                        assert!(child < id, "postorder instruction IDs");
                        decode(code, child, names, visited)
                    })
                    .collect();
                if matches!(code.instructions[id], Instruction::And(_)) {
                    Body::And { items }
                } else {
                    Body::Or { items }
                }
            }
            Instruction::True => Body::True,
            Instruction::Fail => Body::Fail,
        }
    }

    #[test]
    fn packed_arenas_roundtrip_every_position_without_gaps_or_aliasing() {
        for (width, depth) in [(0, 0), (1, 1), (9, 7), (65, 3), (2, 120)] {
            let leaf = syntax::parse_query("p(X,X), (q(Y); q(Y); fail), X=Y, nullary()").unwrap();
            let mut body = Body::And {
                items: vec![leaf; width],
            };
            for d in 0..depth {
                body = if d % 2 == 0 {
                    Body::And { items: vec![body] }
                } else {
                    Body::Or { items: vec![body] }
                };
            }
            let mut program =
                syntax::parse_program("r @ h(X,X), h(Y,X) ==> true. s @ z() <=> true.").unwrap();
            program.rules[0].body = body.clone();
            let original = prepare(&program, &body).unwrap();
            let code = original.clone();
            drop(original);
            let mut visited = vec![false; code.instructions.len()];
            for (source, rule) in program.rules.iter().zip(code.rules()) {
                assert_eq!(
                    decode(&code, rule.body, &rule.variables, &mut visited),
                    source.body
                );
                let heads: Vec<_> = code
                    .heads(rule)
                    .iter()
                    .map(|a| decode_atom(&code, a, &rule.variables))
                    .collect();
                assert_eq!(
                    heads,
                    source
                        .kept
                        .iter()
                        .chain(&source.removed)
                        .cloned()
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(
                decode(&code, code.query, &code.query_variables, &mut visited),
                body
            );
            assert!(visited.iter().all(|&x| x));
            let mut operands = vec![0; code.operands.len()];
            let mut mark = |s: Span| {
                for count in &mut operands[s.start..][..s.len] {
                    *count += 1;
                }
            };
            for head in &code.heads {
                mark(head.args);
            }
            for instruction in &code.instructions {
                match instruction {
                    Instruction::Post(atom) => mark(atom.args),
                    Instruction::And(span) | Instruction::Or(span) => mark(*span),
                    _ => {}
                }
            }
            assert!(operands.iter().all(|&n| n == 1));
            let h = code.heads(&code.rules[0])[0].relation;
            assert_eq!(code.targets(code.triggers[h]), [(0, 0), (0, 1)]);
            assert_eq!(code.targets(code.merge_triggers[h]), [(0, 0), (0, 1)]);
            assert_eq!(code.indexed_end[h], 2);
            assert_eq!(code.merge_indexed_end[h], 2);
            let z = code.heads(&code.rules[1])[0].relation;
            assert_eq!(code.targets(code.triggers[z]), [(1, 0)]);
            assert!(code.targets(code.merge_triggers[z]).is_empty());
            assert_eq!(code.indexed_end[z], 0);
        }
    }

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
