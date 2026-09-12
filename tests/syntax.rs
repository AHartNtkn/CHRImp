use chr::syntax::*;
use serde_json::json;

fn atom(relation: &str, args: &[&str]) -> Atom {
    Atom {
        relation: relation.into(),
        args: args.iter().map(|s| (*s).into()).collect(),
    }
}
fn post(relation: &str, args: &[&str]) -> Body {
    Body::Atom {
        atom: atom(relation, args),
    }
}

#[test]
fn parses_head_modes_and_body_precedence() {
    let p = parse_program("% comment\ns @ p(X), q(X,Y) <=> r(Y,_Fresh9), X = Y; fail.\n p(X) ==> true. // comment\nk @ p(X) \\ q(Y) <=> (a(X); b(Y)), c(_).").unwrap();
    assert_eq!(p.rules.len(), 3);
    assert_eq!(p.rules[0].name.as_deref(), Some("s"));
    assert!(p.rules[0].kept.is_empty());
    assert_eq!(
        p.rules[0].removed,
        vec![atom("p", &["X"]), atom("q", &["X", "Y"])]
    );
    assert_eq!(
        p.rules[0].body,
        Body::Or {
            items: vec![
                Body::And {
                    items: vec![
                        post("r", &["Y", "_Fresh9"]),
                        Body::Equal {
                            left: "X".into(),
                            right: "Y".into()
                        }
                    ]
                },
                Body::Fail
            ]
        }
    );
    assert_eq!(p.rules[1].kept, vec![atom("p", &["X"])]);
    assert!(p.rules[1].removed.is_empty());
    assert_eq!(p.rules[2].kept, vec![atom("p", &["X"])]);
    assert_eq!(p.rules[2].removed, vec![atom("q", &["Y"])]);
    assert_eq!(parse_program(&format_program(&p)).unwrap(), p);
}

#[test]
fn structural_roundtrip_for_graph_created_containers_and_json() {
    let bodies = vec![
        Body::True,
        Body::Fail,
        post("true", &[]),
        post("fail", &["_"]),
        Body::Equal {
            left: "_fresh_123".into(),
            right: "X9".into(),
        },
        Body::And { items: vec![] },
        Body::And {
            items: vec![Body::True],
        },
        Body::Or {
            items: vec![Body::Fail],
        },
        Body::Or {
            items: vec![
                Body::True,
                Body::Or {
                    items: vec![Body::Fail, post("p", &["X"])],
                },
            ],
        },
        Body::And {
            items: vec![
                Body::And {
                    items: vec![post("p", &["X"]), Body::True],
                },
                Body::Fail,
            ],
        },
    ];
    for body in bodies {
        validate_query(&body).unwrap();
        let encoded = serde_json::to_value(&body).unwrap();
        assert!(encoded["kind"].is_string());
        assert_eq!(serde_json::from_value::<Body>(encoded).unwrap(), body);
        assert_eq!(parse_query(&format_query(&body)).unwrap(), body);
        let p = Program {
            rules: vec![Rule {
                name: Some("rule_1".into()),
                kept: vec![atom("p", &[])],
                removed: vec![atom("p", &["X"])],
                body,
            }],
        };
        validate_program(&p).unwrap();
        assert_eq!(parse_program(&format_program(&p)).unwrap(), p);
        assert_eq!(
            serde_json::from_str::<Program>(&serde_json::to_string(&p).unwrap()).unwrap(),
            p
        );
    }
    assert_eq!(
        serde_json::to_value(Body::Equal {
            left: "X".into(),
            right: "Y".into()
        })
        .unwrap(),
        json!({"kind":"equal","left":"X","right":"Y"})
    );
}

#[test]
fn accepts_signatures_locals_comments_and_optional_query_period() {
    assert!(parse_program("").unwrap().rules.is_empty());
    assert!(parse_program("p <=> q(Local). p(X) ==> p(X,Y).").is_ok());
    assert_eq!(
        parse_query("p, p(X), p(X,Y). % λ\n").unwrap(),
        Body::And {
            items: vec![post("p", &[]), post("p", &["X"]), post("p", &["X", "Y"])]
        }
    );
    assert_eq!(parse_query("true // done").unwrap(), Body::True);
    assert_eq!(
        parse_query("X = _Y").unwrap(),
        Body::Equal {
            left: "X".into(),
            right: "_Y".into()
        }
    );
}

#[test]
fn rejects_malformed_source_with_locations() {
    for source in [
        "p(1)",
        "p(\"x\")",
        "p('x')",
        "p(q(X))",
        "p(x)",
        "P(X)",
        "p(X,)",
        "X = 2",
        "X",
        "",
        "p(X);",
        "p(X),",
        "(p(X)",
        "true fail",
        "p(X)..",
        "p(λ)",
    ] {
        let e = parse_query(source).expect_err(source);
        assert!(e.offset.is_some(), "{source}: {e}");
        assert!(e.line.unwrap() >= 1 && e.column.unwrap() >= 1);
    }
    for source in [
        "<=> true.",
        "\\ p(X) <=> true.",
        "p(X) \\ <=> true.",
        "p(X) \\ q(Y) ==> true.",
        "p(X) <=> true",
        "p(X) ==> .",
        "p(X), <=> true.",
        "r @ p(X) ==> true. r @ q(X) ==> true.",
    ] {
        assert!(parse_program(source).is_err(), "{source}");
    }
    let source = "% λ\np(X, 42)";
    let e = parse_query(source).unwrap_err();
    assert_eq!(
        (e.offset, e.line, e.column),
        (Some(source.find('4').unwrap()), Some(2), Some(6))
    );
    assert!(e.to_string().contains("2:6"));
}

#[test]
fn validates_deserialized_ast_at_exact_field_paths() {
    for (value, path) in [
        (
            json!({"kind":"atom","atom":{"relation":"P","args":[]}}),
            "$.atom.relation",
        ),
        (
            json!({"kind":"atom","atom":{"relation":"p","args":["1"]}}),
            "$.atom.args[0]",
        ),
        (json!({"kind":"equal","left":"X","right":"f(Y)"}), "$.right"),
        (json!({"kind":"or","items":[]}), "$"),
        (
            json!({"kind":"and","items":[{"kind":"equal","left":"x","right":"Y"}]}),
            "$.items[0].left",
        ),
    ] {
        let body: Body = serde_json::from_value(value).unwrap();
        let e = validate_query(&body).unwrap_err();
        assert_eq!(e.path.as_deref(), Some(path));
        assert!(e.offset.is_none());
        assert!(e.to_string().contains(path));
    }
    for value in [
        json!({"kind":"bogus"}),
        json!({"kind":"true","items":[]}),
        json!({"kind":"atom"}),
        json!({"kind":"equal","left":12,"right":"X"}),
    ] {
        assert!(serde_json::from_value::<Body>(value).is_err());
    }
    for (value, path) in [
        (
            json!({"rules":[{"name":null,"kept":[],"removed":[],"body":{"kind":"true"}}]}),
            "$.rules[0]",
        ),
        (
            json!({"rules":[{"name":"","kept":[{"relation":"p","args":[]}],"removed":[],"body":{"kind":"true"}}]}),
            "$.rules[0].name",
        ),
        (
            json!({"rules":[{"name":null,"kept":[{"relation":"p","args":["bad"]}],"removed":[],"body":{"kind":"true"}}]}),
            "$.rules[0].kept[0].args[0]",
        ),
    ] {
        let p: Program = serde_json::from_value(value).unwrap();
        assert_eq!(
            validate_program(&p).unwrap_err().path.as_deref(),
            Some(path)
        );
    }
    let r = Rule {
        name: Some("n".into()),
        kept: vec![atom("p", &[])],
        removed: vec![],
        body: Body::True,
    };
    let e = validate_program(&Program {
        rules: vec![r.clone(), r],
    })
    .unwrap_err();
    assert_eq!(e.path.as_deref(), Some("$.rules[1].name"));
}

#[test]
fn deeply_nested_input_returns_error_without_stack_overflow() {
    let source = format!("{}true{}", "(".repeat(2000), ")".repeat(2000));
    assert!(parse_query(&source).is_err());
}

#[test]
fn nesting_limit_is_consistent_between_source_and_ast() {
    // Each group creates both an Or and an And in the resulting AST.
    let source = format!("{}true{}", "(true; true, ".repeat(70), ")".repeat(70));
    assert!(parse_query(&source).is_err());
    assert!(parse_program(&format!("p ==> {source}.")).is_err());
    let mut body = Body::True;
    for _ in 0..128 {
        body = Body::And { items: vec![body] };
    }
    validate_query(&body).unwrap();
    assert_eq!(parse_query(&format_query(&body)).unwrap(), body);
    body = Body::Or { items: vec![body] };
    assert!(validate_query(&body).is_err());
}

#[test]
fn deep_json_is_rejected_without_aborting() {
    // A subprocess contains any process-aborting regression at the raw boundary.
    if std::env::var_os("CHR_SYNTAX_DEEP_JSON").is_some() {
        let source = format!(
            "{}{{\"kind\":\"true\"}}{}",
            r#"{"items":["#.repeat(3000),
            r#"],"kind":"and"}"#.repeat(3000)
        );
        let error = parse_query_json(&source).unwrap_err();
        assert!(error.message.contains("body nesting"), "{error}");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "deep_json_is_rejected_without_aborting",
            "--nocapture",
        ])
        .env("CHR_SYNTAX_DEEP_JSON", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn direct_serde_bounds_borrowed_values_before_constructing_body() {
    let mut value = json!({"kind":"true"});
    for _ in 0..3000 {
        let mut fields = serde_json::Map::new();
        fields.insert("kind".into(), "and".into());
        fields.insert("items".into(), serde_json::Value::Array(vec![value]));
        value = serde_json::Value::Object(fields);
    }
    let result = <Body as serde::Deserialize>::deserialize(&value);
    // Value itself has recursive Drop; that is independent of syntax decoding.
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(fields) => pending.extend(fields.into_values()),
            _ => {}
        }
    }
    assert!(result.unwrap_err().to_string().contains("body nesting"));
}

#[test]
fn json_boundary_roundtrips_the_full_validated_depth() {
    for depth in [64, 128] {
        let mut body = Body::True;
        for _ in 0..depth {
            body = Body::And { items: vec![body] };
        }
        validate_query(&body).unwrap();
        assert_eq!(parse_query(&format_query(&body)).unwrap(), body);
        assert_eq!(
            parse_query_json(&serde_json::to_string(&body).unwrap()).unwrap(),
            body
        );
        let program = Program {
            rules: vec![Rule {
                name: None,
                kept: vec![atom("p", &[])],
                removed: vec![],
                body,
            }],
        };
        validate_program(&program).unwrap();
        assert_eq!(
            parse_program_json(&serde_json::to_string(&program).unwrap()).unwrap(),
            program
        );
    }
}

#[test]
fn json_boundary_checks_shape_order_depth_and_semantics() {
    assert_eq!(
        parse_query_json(r#"{"items":[{"right":"Y","left":"X","kind":"equal"}],"kind":"or"}"#)
            .unwrap(),
        Body::Or {
            items: vec![Body::Equal {
                left: "X".into(),
                right: "Y".into()
            }]
        }
    );
    for source in [
        r#"{"kind":"true","kind":"fail"}"#,
        r#"{"kind":"true","extra":[]}"#,
        r#"{"kind":"true","items":[]}"#,
        r#"{"items":[],"kind":"true"}"#,
        r#"{"kind":"and","items":[],"items":[]}"#,
        r#"{"kind":"equal","left":"X","right":"Y","atom":{"relation":"p","args":[]}}"#,
        r#"{"kind":"and"}"#,
        r#"{"kind":"atom","atom":null}"#,
        r#"{"kind":"unknown"}"#,
        r#"{"kind":"true"} {}"#,
    ] {
        assert!(parse_query_json(source).is_err(), "{source}");
    }
    assert_eq!(
        parse_query_json(r#"{"kind":"equal","left":"x","right":"Y"}"#)
            .unwrap_err()
            .path
            .as_deref(),
        Some("$.left")
    );
    assert!(
        parse_program_json(r#"{"rules":[{"kept":[],"removed":[],"body":{"kind":"true"}}]}"#)
            .is_err()
    );
    for depth in [129, 3000] {
        for (prefix, suffix) in [
            (r#"{"kind":"and","items":["#, "]}"),
            (r#"{"items":["#, r#"],"kind":"and"}"#),
        ] {
            let source = format!(
                "{}{{\"kind\":\"true\"}}{}",
                prefix.repeat(depth),
                suffix.repeat(depth)
            );
            let error = parse_query_json(&source).unwrap_err();
            assert!(error.message.contains("128"), "{error}");
            assert!(error.offset.is_some());
            let source = format!(
                r#"{{"rules":[{{"kept":[{{"relation":"p","args":[]}}],"removed":[],"body":{source}}}]}}"#
            );
            assert!(parse_program_json(&source).is_err());
        }
    }
}
