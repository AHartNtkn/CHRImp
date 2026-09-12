use chr::program::{Instruction, prepare};
use chr::syntax::{parse_program, parse_query};

#[test]
fn prepares_ordered_heads_and_fresh_body_slots_once() {
    let source = parse_program("join @ p(X,X) \\ q(Y,X) <=> X = Y, r(Z,X), s(Z,W).").unwrap();
    let code = prepare(&source, &parse_query("p(A,A), q(B,A)").unwrap()).unwrap();
    let rule = &code.rules[0];
    assert_eq!(rule.kept, 1);
    assert_eq!(rule.head_variables, 2);
    assert_eq!(rule.variables, ["X", "Y", "Z", "W"]);
    assert_eq!(rule.heads[0].args, [0, 0]);
    assert_eq!(rule.heads[1].args, [1, 0]);
    assert_eq!(code.query_variables, ["A", "B"]);
    let Instruction::And(items) = &code.instructions[rule.body] else {
        panic!("conjunction");
    };
    assert!(matches!(
        code.instructions[items[0]],
        Instruction::Equal(0, 1)
    ));
    let Instruction::Post(atom) = &code.instructions[items[1]] else {
        panic!("post");
    };
    assert_eq!(atom.args, [2, 0]);
    assert_eq!(code.signatures[atom.relation].name, "r");
    assert_eq!(code.signatures[atom.relation].arity, 2);
}

#[test]
fn dispatch_retains_every_head_position_and_relation_arity() {
    let source = parse_program("p(X), p(Y) ==> q(X,Y). p <=> p(X).").unwrap();
    let code = prepare(&source, &parse_query("p(A), p()").unwrap()).unwrap();
    let unary = code.rules[0].heads[0].relation;
    let nullary = code.rules[1].heads[0].relation;
    assert_ne!(unary, nullary);
    assert_eq!(code.triggers[unary], [(0, 0), (0, 1)]);
    assert_eq!(code.triggers[nullary], [(1, 0)]);
    assert_eq!(code.rules[0].kept, 2);
    assert_eq!(code.rules[1].kept, 0);
    assert_eq!(code.rules[1].head_variables, 0);
    assert_eq!(code.rules[1].variables, ["X"]);
}

#[test]
fn preparation_preserves_explicit_choice_structure_and_duplicate_arms() {
    let code = prepare(
        &parse_program("").unwrap(),
        &parse_query("p(X); (p(X); p(X))").unwrap(),
    )
    .unwrap();
    let Instruction::Or(arms) = &code.instructions[code.query] else {
        panic!("choice");
    };
    assert_eq!(arms.len(), 2);
    let Instruction::Or(nested) = &code.instructions[arms[1]] else {
        panic!("nested choice");
    };
    assert_eq!(nested.len(), 2);
    assert_eq!(code.instructions[arms[0]], code.instructions[nested[0]]);
    assert_eq!(code.instructions[nested[0]], code.instructions[nested[1]]);
    // Equal instructions do not collapse distinct explicit alternatives.
    assert_ne!(nested[0], nested[1]);
    assert_eq!(code.query_variables, ["X"]);
}

#[test]
fn preparation_rejects_invalid_graph_editor_models() {
    let source = parse_program("p(X) ==> q(X).").unwrap();
    let mut query = parse_query("p(X)").unwrap();
    let chr::syntax::Body::Atom { atom } = &mut query else {
        unreachable!()
    };
    atom.args[0] = "123".into();
    assert!(prepare(&source, &query).is_err());
    let mut invalid = source;
    invalid.rules[0].kept.clear();
    assert!(prepare(&invalid, &parse_query("true").unwrap()).is_err());
}
