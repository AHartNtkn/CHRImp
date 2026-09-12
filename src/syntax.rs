//! Shared text/JSON syntax for the relational engine and graph editor.
//!
//! ASCII identifiers contain letters, digits and `_`: relations start lowercase,
//! variables start uppercase or `_`, and rule names start with any letter or `_`.
//! Arguments are variables only (including `_`, an ordinary named variable).
//! Relation/arity pairs are independent signatures. Body-only variables are allowed.
//!
//! Rules: `[name @] atoms <=> body.`, `atoms ==> body.`, or
//! `atoms \ atoms <=> body.`; both explicit head lists must be nonempty.
//! Bodies: variable equality `X = Y`, relation atoms, `true`, `fail`, parentheses,
//! comma conjunction and lower-precedence semicolon disjunction. Zero-port atoms
//! may omit parentheses; `true()` and `fail()` denote relations, not built-ins.
//! Queries accept an optional final period. `%` and `//` start line comments.
//!
//! Group boundaries preserve nested containers. For exact graph AST round trips,
//! `()` denotes empty And, `(body,)` singleton And, `(body;)` singleton Or.
//! Empty Or is invalid. Nesting is limited to 128 groups/containers.
//! Use `parse_query_json` / `parse_program_json` for untrusted notebook JSON.
//! They bound decoding before constructing a Body, then validate the model, and
//! support all 128 containers (including when `kind` follows `items`). Direct
//! serde decoding also bounds Body depth, but serde_json's default byte decoder
//! has a separate 128-JSON-level limit; use these entry points for full-depth
//! save/load. Owned hostile serde_json::Value trees have recursive Drop outside
//! this API; raw JSON avoids that cleanup hazard. Direct serde checks shape;
//! call validation before executing models obtained that way.
//! Source errors carry a zero-based UTF-8 byte offset and one-based line/character
//! column. AST errors instead carry a JSON field path, with no invented source position.
//! Formatters are total over the model; validate first for parseable output.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Atom {
    pub relation: String,
    pub args: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: Option<String>,
    pub kept: Vec<Atom>,
    pub removed: Vec<Atom>,
    pub body: Body,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    Atom { atom: Atom },
    Equal { left: String, right: String },
    And { items: Vec<Body> },
    Or { items: Vec<Body> },
    True,
    Fail,
}
impl<'de> Deserialize<'de> for Body {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::DeserializeSeed::deserialize(BodySeed(0), deserializer)
    }
}

// Carry semantic depth through items, including when items precedes kind. Never
// buffer an untrusted subtree as Value or internally tagged serde Content.
struct BodySeed(usize);
impl<'de> serde::de::DeserializeSeed<'de> for BodySeed {
    type Value = Body;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Body, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> serde::de::Visitor<'de> for BodySeed {
    type Value = Body;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a body object tagged with kind")
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Body, M::Error> {
        use serde::de::Error;
        let (mut kind, mut atom, mut left, mut right, mut items) = (None, None, None, None, None);
        let mut seen = 0u8;
        while let Some(field) = map.next_key::<String>()? {
            let bit = match field.as_str() {
                "kind" => 1,
                "atom" => 2,
                "left" => 4,
                "right" => 8,
                "items" => 16,
                _ => {
                    return Err(M::Error::unknown_field(
                        &field,
                        &["kind", "atom", "left", "right", "items"],
                    ));
                }
            };
            if seen & bit != 0 {
                return Err(M::Error::custom(format!("duplicate field `{field}`")));
            }
            seen |= bit;
            match field.as_str() {
                "kind" => kind = Some(map.next_value::<String>()?),
                "atom" => atom = Some(map.next_value::<Atom>()?),
                "left" => left = Some(map.next_value::<String>()?),
                "right" => right = Some(map.next_value::<String>()?),
                "items" => {
                    if self.0 >= MAX_NESTING {
                        return Err(M::Error::custom("body nesting exceeds 128 containers"));
                    }
                    items = Some(map.next_value_seed(ItemsSeed(self.0 + 1))?);
                }
                _ => unreachable!(), // field was checked above
            }
        }
        let kind = kind.ok_or_else(|| M::Error::missing_field("kind"))?;
        let allowed = match kind.as_str() {
            "atom" => 3,
            "equal" => 13,
            "and" | "or" => 17,
            "true" | "fail" => 1,
            _ => {
                return Err(M::Error::unknown_variant(
                    &kind,
                    &["atom", "equal", "and", "or", "true", "fail"],
                ));
            }
        };
        for (bit, field) in [(2, "atom"), (4, "left"), (8, "right"), (16, "items")] {
            if seen & bit != 0 && allowed & bit == 0 {
                return Err(M::Error::custom(format!(
                    "unexpected field `{field}` for `{kind}`"
                )));
            }
        }
        Ok(match kind.as_str() {
            "atom" => Body::Atom {
                atom: atom.ok_or_else(|| M::Error::missing_field("atom"))?,
            },
            "equal" => Body::Equal {
                left: left.ok_or_else(|| M::Error::missing_field("left"))?,
                right: right.ok_or_else(|| M::Error::missing_field("right"))?,
            },
            "and" | "or" => {
                let items = items.ok_or_else(|| M::Error::missing_field("items"))?;
                if kind == "and" {
                    Body::And { items }
                } else {
                    Body::Or { items }
                }
            }
            "true" => Body::True,
            "fail" => Body::Fail,
            _ => unreachable!(), // kind was checked above
        })
    }
}
struct ItemsSeed(usize);
impl<'de> serde::de::DeserializeSeed<'de> for ItemsSeed {
    type Value = Vec<Body>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> serde::de::Visitor<'de> for ItemsSeed {
    type Value = Vec<Body>;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an array of bodies")
    }
    fn visit_seq<S: serde::de::SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
        let mut items = Vec::new();
        while let Some(body) = seq.next_element_seed(BodySeed(self.0))? {
            items.push(body);
        }
        Ok(items)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: Option<usize>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub path: Option<String>,
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(offset), Some(line), Some(column)) = (self.offset, self.line, self.column) {
            write!(f, "{} at {line}:{column} (byte {offset})", self.message)
        } else {
            write!(
                f,
                "{} at {}",
                self.message,
                self.path.as_deref().unwrap_or("$")
            )
        }
    }
}
impl std::error::Error for ParseError {}
const MAX_NESTING: usize = 128;

fn identifier(s: &str, start: impl Fn(u8) -> bool) -> bool {
    s.bytes().next().is_some_and(start) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn variable(s: &str) -> bool {
    identifier(s, |c| c.is_ascii_uppercase() || c == b'_')
}
fn relation(s: &str) -> bool {
    identifier(s, |c| c.is_ascii_lowercase())
}
fn name(s: &str) -> bool {
    identifier(s, |c| c.is_ascii_alphabetic() || c == b'_')
}
fn invalid(path: &str, message: &str) -> ParseError {
    ParseError {
        message: message.into(),
        offset: None,
        line: None,
        column: None,
        path: Some(path.into()),
    }
}
fn validate_atom(atom: &Atom, path: &str) -> Result<(), ParseError> {
    if !relation(&atom.relation) {
        return Err(invalid(
            &format!("{path}.relation"),
            "expected lowercase-start relation identifier",
        ));
    }
    for (i, arg) in atom.args.iter().enumerate() {
        if !variable(arg) {
            return Err(invalid(
                &format!("{path}.args[{i}]"),
                "expected uppercase or underscore-start variable identifier",
            ));
        }
    }
    Ok(())
}
fn validate_body(body: &Body, path: &str) -> Result<(), ParseError> {
    let mut pending = vec![(body, path.to_owned(), 0)];
    while let Some((body, path, depth)) = pending.pop() {
        match body {
            Body::Atom { atom } => validate_atom(atom, &format!("{path}.atom"))?,
            Body::Equal { left, right } => {
                for (field, value) in [("left", left), ("right", right)] {
                    if !variable(value) {
                        return Err(invalid(
                            &format!("{path}.{field}"),
                            "expected uppercase or underscore-start variable identifier",
                        ));
                    }
                }
            }
            Body::And { items } | Body::Or { items } => {
                if depth >= MAX_NESTING {
                    return Err(invalid(&path, "body nesting exceeds 128 containers"));
                }
                if matches!(body, Body::Or { .. }) && items.is_empty() {
                    return Err(invalid(&path, "disjunction must contain at least one body"));
                }
                for (i, item) in items.iter().enumerate().rev() {
                    pending.push((item, format!("{path}.items[{i}]"), depth + 1));
                }
            }
            Body::True | Body::Fail => {}
        }
    }
    Ok(())
}
pub fn validate_query(body: &Body) -> Result<(), ParseError> {
    validate_body(body, "$")
}
pub fn validate_program(program: &Program) -> Result<(), ParseError> {
    let mut names = std::collections::HashSet::new();
    for (i, rule) in program.rules.iter().enumerate() {
        let path = format!("$.rules[{i}]");
        if let Some(n) = &rule.name {
            if !name(n) {
                return Err(invalid(
                    &format!("{path}.name"),
                    "expected nonempty rule identifier",
                ));
            }
            if !names.insert(n) {
                return Err(invalid(&format!("{path}.name"), "duplicate rule name"));
            }
        }
        if rule.kept.is_empty() && rule.removed.is_empty() {
            return Err(invalid(&path, "rule must have at least one head atom"));
        }
        for (field, atoms) in [("kept", &rule.kept), ("removed", &rule.removed)] {
            for (j, atom) in atoms.iter().enumerate() {
                validate_atom(atom, &format!("{path}.{field}[{j}]"))?;
            }
        }
        validate_body(&rule.body, &format!("{path}.body"))?;
    }
    Ok(())
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
}
impl<'a> Parser<'a> {
    fn error(&self, message: &str) -> ParseError {
        let prefix = &self.source[..self.pos];
        ParseError {
            message: message.into(),
            offset: Some(self.pos),
            line: Some(prefix.bytes().filter(|b| *b == b'\n').count() + 1),
            column: Some(prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1),
            path: None,
        }
    }
    fn skip(&mut self) {
        loop {
            while let Some(c) = self.source[self.pos..]
                .chars()
                .next()
                .filter(|c| c.is_whitespace())
            {
                self.pos += c.len_utf8();
            }
            let tail = &self.source[self.pos..];
            if tail.starts_with('%') || tail.starts_with("//") {
                self.pos += tail.find('\n').unwrap_or(tail.len());
            } else {
                break;
            }
        }
    }
    fn at(&mut self, token: &str) -> bool {
        self.skip();
        self.source[self.pos..].starts_with(token)
    }
    fn eat(&mut self, token: &str) -> bool {
        if self.at(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, token: &str) -> Result<(), ParseError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{token}`")))
        }
    }
    fn id(&mut self, valid: fn(&str) -> bool, message: &str) -> Result<String, ParseError> {
        self.skip();
        let start = self.pos;
        let len = self.source[start..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        let id = &self.source[start..start + len];
        if !valid(id) {
            return Err(self.error(message));
        }
        self.pos += len;
        Ok(id.into())
    }
    fn var(&mut self) -> Result<String, ParseError> {
        self.id(
            variable,
            "expected uppercase or underscore-start variable identifier",
        )
    }
    fn atom(&mut self) -> Result<Atom, ParseError> {
        let relation = self.id(relation, "expected lowercase-start relation identifier")?;
        self.atom_args(relation)
    }
    fn atom_args(&mut self, relation: String) -> Result<Atom, ParseError> {
        let mut args = Vec::new();
        if self.eat("(") && !self.eat(")") {
            loop {
                args.push(self.var()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        Ok(Atom { relation, args })
    }
    fn heads(&mut self) -> Result<Vec<Atom>, ParseError> {
        let mut atoms = vec![self.atom()?];
        while self.eat(",") {
            atoms.push(self.atom()?);
        }
        Ok(atoms)
    }
    fn body(&mut self, depth: usize) -> Result<Body, ParseError> {
        let first = self.and(depth)?;
        if !self.eat(";") {
            return Ok(first);
        }
        let mut items = vec![first];
        // A trailing separator denotes a singleton only inside a group.
        if !(depth > 0 && self.at(")")) {
            loop {
                items.push(self.and(depth)?);
                if !self.eat(";") {
                    break;
                }
            }
        }
        Ok(Body::Or { items })
    }
    fn and(&mut self, depth: usize) -> Result<Body, ParseError> {
        let first = self.primary(depth)?;
        if !self.eat(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        if !(depth > 0 && self.at(")")) {
            loop {
                items.push(self.primary(depth)?);
                if !self.eat(",") {
                    break;
                }
            }
        }
        Ok(Body::And { items })
    }
    fn primary(&mut self, depth: usize) -> Result<Body, ParseError> {
        if self.at("(") {
            if depth >= MAX_NESTING {
                return Err(self.error("body nesting exceeds 128 groups"));
            }
            self.eat("(");
            if self.eat(")") {
                return Ok(Body::And { items: vec![] });
            }
            let body = self.body(depth + 1)?;
            self.expect(")")?;
            return Ok(body);
        }
        self.skip();
        if self.source[self.pos..]
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == b'_')
        {
            let left = self.var()?;
            self.expect("=")?;
            return Ok(Body::Equal {
                left,
                right: self.var()?,
            });
        }
        let relation = self.id(
            relation,
            "expected body atom, equality, true, fail or parenthesized body",
        )?;
        if !self.at("(") {
            match relation.as_str() {
                "true" => return Ok(Body::True),
                "fail" => return Ok(Body::Fail),
                _ => {}
            }
        }
        Ok(Body::Atom {
            atom: self.atom_args(relation)?,
        })
    }
    fn end(&mut self) -> Result<(), ParseError> {
        self.skip();
        if self.pos == self.source.len() {
            Ok(())
        } else {
            Err(self.error("unexpected trailing input"))
        }
    }
}
pub fn parse_query(source: &str) -> Result<Body, ParseError> {
    let mut p = Parser { source, pos: 0 };
    let body = p.body(0)?;
    validate_query(&body).map_err(|e| p.error(&e.message))?;
    p.eat(".");
    p.end()?;
    Ok(body)
}
pub fn parse_program(source: &str) -> Result<Program, ParseError> {
    let mut p = Parser { source, pos: 0 };
    let mut rules = Vec::new();
    let mut names = std::collections::HashSet::new();
    p.skip();
    while p.pos < source.len() {
        let start = p.pos;
        let candidate = p.id(name, "expected rule head or name")?;
        let name = if p.eat("@") {
            if !names.insert(candidate.clone()) {
                p.pos = start;
                return Err(p.error("duplicate rule name"));
            }
            Some(candidate)
        } else {
            p.pos = start;
            None
        };
        let heads = p.heads()?;
        let (kept, removed) = if p.eat("\\") {
            let removed = p.heads()?;
            p.expect("<=>")?;
            (heads, removed)
        } else if p.eat("==>") {
            (heads, vec![])
        } else {
            p.expect("<=>")?;
            (vec![], heads)
        };
        let body = p.body(0)?;
        validate_query(&body).map_err(|e| p.error(&e.message))?;
        p.expect(".")?;
        rules.push(Rule {
            name,
            kept,
            removed,
            body,
        });
        p.skip();
    }
    Ok(Program { rules })
}
fn format_atom(atom: &Atom) -> String {
    format!("{}({})", atom.relation, atom.args.join(", "))
}
pub fn format_query(body: &Body) -> String {
    enum Part<'a> {
        Body(&'a Body),
        Text(&'static str),
    }
    let mut output = String::new();
    let mut pending = vec![Part::Body(body)];
    while let Some(part) = pending.pop() {
        match part {
            Part::Text(text) => output.push_str(text),
            Part::Body(body) => match body {
                Body::Atom { atom } => output.push_str(&format_atom(atom)),
                Body::Equal { left, right } => {
                    output.push_str(left);
                    output.push_str(" = ");
                    output.push_str(right);
                }
                Body::True => output.push_str("true"),
                Body::Fail => output.push_str("fail"),
                Body::And { items } | Body::Or { items } => {
                    let separator = if matches!(body, Body::And { .. }) {
                        ","
                    } else {
                        ";"
                    };
                    output.push('(');
                    pending.push(Part::Text(")"));
                    if items.len() == 1 {
                        pending.push(Part::Text(separator));
                    }
                    for (i, item) in items.iter().enumerate().rev() {
                        pending.push(Part::Body(item));
                        if i > 0 {
                            pending.push(Part::Text(" "));
                            pending.push(Part::Text(separator));
                        }
                    }
                }
            },
        }
    }
    output
}
pub fn format_program(program: &Program) -> String {
    fn heads(atoms: &[Atom]) -> String {
        atoms.iter().map(format_atom).collect::<Vec<_>>().join(", ")
    }
    program
        .rules
        .iter()
        .map(|rule| {
            let name = rule
                .name
                .as_ref()
                .map(|n| format!("{n} @ "))
                .unwrap_or_default();
            let head = if rule.removed.is_empty() {
                format!("{} ==>", heads(&rule.kept))
            } else if rule.kept.is_empty() {
                format!("{} <=>", heads(&rule.removed))
            } else {
                format!("{} \\ {} <=>", heads(&rule.kept), heads(&rule.removed))
            };
            format!("{name}{head} {}.", format_query(&rule.body))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Decode and validate notebook query JSON at the full language depth limit.
pub fn parse_query_json(source: &str) -> Result<Body, ParseError> {
    let body = decode_json(source)?;
    validate_query(&body)?;
    Ok(body)
}
/// Decode and validate notebook program JSON at the full language depth limit.
pub fn parse_program_json(source: &str) -> Result<Program, ParseError> {
    let program = decode_json(source)?;
    validate_program(&program)?;
    Ok(program)
}
fn decode_json<T: serde::de::DeserializeOwned>(source: &str) -> Result<T, ParseError> {
    let mut decoder = serde_json::Deserializer::from_str(source);
    // BodySeed enforces semantic depth before descending; all other fields have
    // fixed nonrecursive shapes and reject unexpected fields without reading them.
    decoder.disable_recursion_limit();
    let decoded = T::deserialize(&mut decoder).and_then(|value| decoder.end().map(|()| value));
    decoded.map_err(|error| {
        let line_start = source
            .split_inclusive('\n')
            .take(error.line().saturating_sub(1))
            .map(str::len)
            .sum::<usize>();
        let mut pos = (line_start + error.column().saturating_sub(1)).min(source.len());
        while !source.is_char_boundary(pos) {
            pos -= 1;
        }
        Parser { source, pos }.error(&error.to_string())
    })
}
