# CHR

Start the local browser notebook:

```sh
cargo run --release --offline -- --notebook
```

Open the printed URL. Program displays the editable rule diagrams; Query displays the editable query and its Run and Step controls, with answers underneath. Select a relation or compartment to edit it. Source text is available in the optional Source text section. History recording is optional and off by default. Saved answers remain in this browser; execution handles and recorded states last until released or the server stops. The default address uses port `7878`, preserving the browser storage origin across restarts. Use `--port PORT` to choose a different port; that address has separate browser storage.

Choose **Examples → Open example** to open one of four complete notebooks:

| Notebook | Queries | Coverage |
| --- | ---: | --- |
| [Arithmetic](examples/arithmetic.chrnb) | 12 | Forward addition, either missing operand, all decompositions of a sum, subtraction, partial answers, inconsistent inputs and successor cycles. |
| [Type-driven synthesis](examples/type-synthesis.chrnb) | 15 | Identity, constant, second selector, substitution, composition, argument swapping, duplication and application-to-a-function targets; principal type inference and rejection of self-application with finite simple types. |
| [Behavior-driven synthesis](examples/behavior-synthesis.chrnb) | 19 | The same combinator behaviors plus self-application, forward and underapplied evaluation, residual holes and the no-constant restriction. |
| [Lambda expressions](examples/lambda.chrnb) | 39 | Rewrite arms, recursive normalization, backward synthesis, structural consistency, disequality, normal-form constraints and shared versus distinct binder/argument wires. |

Each file contains its whole program and editable queries. Structure is expressed through ordinary relations: e.g. `app(Root,Function,Argument)`, `arrow(Type,Domain,Codomain)` and `succ(Number,Predecessor)`. The program supplies structural consistency and finite-structure constraints; these are not language built-ins. In the lambda notebook, binder and occurrence ports reference unscoped variable identities. Connections are intentional; equal printed lexical names are not the binding model.

The combinator notebooks use SK expressions, with unrestricted recursive search rather than a catalog of answers. `type_a`, `type_b` and `type_c` mark distinct rigid type parameters in synthesis targets; inference queries leave types open. Opaque test symbols in behavioral targets cannot occur inside a synthesized program because of `no_c`. An ignored argument may remain an unknown wire with a residual restriction. Pause and Resume continue enumeration; search need not exhaust. Unrestricted identity and constant-function synthesis produce validated answers. Composition, argument swapping and duplication remain expensive; the performance suite reports incomplete prefixes explicitly. Tests separately verify every target against a combinator checked by an independent SK reducer, plus unrestricted identity synthesis and continued constant-function enumeration; they do not claim prompt unrestricted synthesis of every target.

These notebooks adapt the [arithmetic, SK, typing and lambda programs](https://github.com/AHartNtkn/CHRLang/blob/reference-interpreter/crates/chr-programs/src/lib.rs) into the variable-only language. Their runnable definitions are the `.chrnb` files themselves.

Use **New**, **Open**, **Save**, and **Save as** for `.chrnb` notebook files containing the program, named queries, and diagram positions. Browsers with file-system pickers save back to the chosen file; other browsers download a notebook file and reopen it through Open. Browser recovery also keeps the working notebook between visits.

Create, rename, duplicate, or switch queries above the query diagram. Shift-click relations or Shift-drag a selection rectangle to select a fragment; drag a selected relation to move the group. Copy, cut, paste, duplicate, and delete act on the selection. Pasted fragments keep their internal connections and receive fresh variables. Select a rule title to copy the whole rule; its arrow buttons change rule order. Find locates rules, relations, and named queries.

Export SVG saves a whole diagram, including offscreen content. Answer exports contain complete bindings, facts, ordered ports, and pending alternatives; choose one answer, all saved answers, or a complete answer SVG. Ctrl/Cmd+C/X/V/D, Z, Shift+Z (or Y), and Delete/Backspace operate on diagram selections; text fields keep native editing. Ctrl/Cmd+S saves, Shift+S saves as, O opens, and F focuses Find.

Normal execution runs independently of the browser. Closing the page or delaying answer reads does not stop computation; the runtime retains produced output until the execution is closed or the server stops. Reloading the same notebook restores its editor, saved inspection, and execution status. Pause explicitly suspends execution, and Resume continues it. One browser tab controls the notebook at a time; other tabs can browse saved answers. The CLI and notebook use the same execution driver and language engine.

Step pauses after one rule application in the selected alternative. Pause can suspend an unfinished step; Resume step continues that same application, including after reload. Choice and history selection stay fixed until the step finishes or is canceled.

The step’s inspection shows current relations and pending body expressions as graphs, with alternatives displayed in split boxes. Choice and history controls load one page at a time and preserve your selection while you browse. Recorded failure states show rejected alternatives for inspection; they are never emitted as successful answers.

Run a program with a relational query:

```sh
cargo run --release --offline -- examples/reachability.chr --query 'edge(A,B),edge(B,C)'
```

The proof example keeps both a direct path proof and a composed proof. `compose(AB,BC,R)` connects the two premises to a fresh proof variable `R`:

```sh
cargo run --release --offline -- examples/proofs.chr --query 'edge(A,B,AB),edge(B,C,BC),edge(A,C,AC)'
```

The synthesis example chooses explicitly among identity, negation, and constant Boolean programs. Two input/output examples leave `negate(P)` as the answer. With only `evaluate(P,A,B)`, both negation and constant-one survive as separate answers:

```sh
cargo run --release --offline -- examples/synthesis.chr --query 'synthesize(P),zero(A),one(B),evaluate(P,A,B),evaluate(P,B,A)'
```

These examples define their operations entirely through ordinary relations and rules. Open Source text and paste the file text and its query to inspect their graphs.

Relation arguments are variables. Relations encode structure; `X=Y` explicitly makes two variables the same. A rule head checks existing relationships without changing them.

```text
p(X) <=> q(X).              % replace p with q
p(X) ==> q(X).              % keep p and add q once per occurrence
p(X) \ q(X) <=> r(X).       % keep p, consume q, add r
choose(X) <=> (left(X);right(X)).
```

Names contain ASCII letters, digits, and underscores. Relation names start lowercase;
variables start uppercase or with `_`. `_` is an ordinary named variable: repeated
uses refer to the same variable within a query or rule. Each rule has its own variable
scope. Variables appearing only in a rule body are fresh for each application.

Rules end with a period and may have a unique name, such as
`rewrite @ p(X) <=> q(X).` Head relations are separated by commas; both sides of
`\` must contain at least one relation. A query's final period is optional.
`%` and `//` begin line comments.

In bodies and queries, `,` means conjunction and binds more tightly than `;`:
`a(X),b(X);c(X)` means `(a(X),b(X));c(X)`. Parentheses group expressions.
`true` succeeds without adding a relation; `fail` rejects the alternative.
A relation with no ports may be written `tag` or `tag()`; use `true()` or `fail()`
to name ordinary relations with those names. Different arities, such as `p(X)`
and `p(X,Y)`, are separate relation signatures.

The graph editor preserves expression groups in text: `()` is an empty conjunction,
`(a(X),)` a one-item conjunction, and `(a(X);)` a one-arm disjunction.
An empty disjunction is invalid. Expressions support up to 128 nested groups.

Only `;` creates search alternatives. Competing rule applications commit to a schedule. Each successful alternative returns its residual graph when all applicable work settles; equal graphs from separate alternatives remain separate answers. `fail` rejects its alternative.

Standard output is newline-delimited JSON. A `program` event supplies relation signatures and query-variable names. Each answer streams from `begin` through `end`, with variable mappings, distinct relation occurrences, and ports in argument order. An answer is flushed as soon as it finishes, including when another alternative continues indefinitely.

The engine reclaims completed choices during continuing execution when they no longer distinguish surviving alternatives. Suspended matches keep their progress across compaction. Explicitly held views and notebook choice selections retain the information needed for inspection.

Run the checks with `cargo test --offline`.

List performance probes with `cargo run --release --example measure -- --list`.
For example, `cargo run --release --example measure -- rejected3 128` measures an
unsuccessful three-head join. Use `answers 128 --rows 0` for complete delivery of
empty alternatives, or `notebook-type-i 1` for unrestricted synthesis through its first answer.
The suite checks exact results and reports first-answer latency, validation cost,
collection work, sampled storage, and cancellation/reclamation. Answer and application
prefixes are labeled separately from exhausted searches; a limit reached before the
requested result is incomplete. Lifecycle and runtime probes cover continuing execution,
retained history, and delayed answer reads. Memory counts are not byte or exact-peak measurements.
