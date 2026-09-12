# CHR

Start the local browser notebook:

```sh
cargo run --release --offline -- --notebook
```

Open the printed URL. Program displays the editable rule diagrams; Query displays the editable query and its Run and Step controls, with answers underneath. Select a relation or compartment to edit it. Source text is available in the optional Source text section. History recording is optional and off by default. Saved answers remain in this browser; execution handles and recorded states last until released or the server stops. Use `--port PORT` to choose a port.

Reloading the same notebook restores its editor, saved inspection, and paused execution while the server remains running. Resume continues that execution. One browser tab controls the notebook at a time; other tabs can browse saved answers.

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

Measure complete engine runs with `cargo run --release --example measure -- --list`,
then choose a case and size, for example `cargo run --release --example measure -- dense 32`.
The runner checks exact ordered tuples and answer multiplicity. It reports preparation,
execution and delivery, disposal, and sampled memory separately; execution timings include
the output checks. A run that reaches its work or time limit is reported as incomplete.
