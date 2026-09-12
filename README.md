# CHR

Start the local browser notebook:

```sh
cargo run --offline -- --notebook
```

Open the printed URL. Edit programs and queries as text or graphs, then run or step through execution. History recording is optional and off by default. Saved answers remain in this browser; execution handles and recorded states last until released or the server stops. Use `--port PORT` to choose a port.

Step pauses after one rule application in the selected alternative. Its inspection shows current relations and pending body expressions; open a conjunction or disjunction to inspect its contents. Choice and history controls load one page at a time and preserve your selection while you browse. Recorded failure states show rejected alternatives for inspection; they are never emitted as successful answers.

Run a program with a relational query:

```sh
cargo run --offline -- examples/reachability.chr --query 'edge(A,B),edge(B,C)'
```

Relation arguments are variables. Relations encode structure; `X=Y` explicitly makes two variables the same. A rule head checks existing relationships without changing them.

```text
p(X) <=> q(X).              % replace p with q
p(X) ==> q(X).              % keep p and add q once per occurrence
p(X) \ q(X) <=> r(X).       % keep p, consume q, add r
choose(X) <=> (left(X);right(X)).
```

Only `;` creates search alternatives. Competing rule applications commit to a schedule. Each successful alternative returns its residual graph when all applicable work settles; equal graphs from separate alternatives remain separate answers. `fail` rejects its alternative.

Standard output is newline-delimited JSON. A `program` event supplies relation signatures and query-variable names. Each answer streams from `begin` through `end`, with variable mappings, distinct relation occurrences, and ports in argument order. An answer is flushed as soon as it finishes, including when another alternative continues indefinitely.

The engine reclaims completed choices during continuing execution when they no longer distinguish surviving alternatives. Suspended matches keep their progress across compaction. Explicitly held views and notebook choice selections retain the information needed for inspection.

Run the checks with `cargo test --offline`.

Measure complete engine runs with `cargo run --release --example measure -- --list`,
then choose a case and size, for example `cargo run --release --example measure -- dense 32`.
The runner checks exact ordered tuples and answer multiplicity. It reports preparation,
execution and delivery, disposal, and sampled memory separately; execution timings include
the output checks. A run that reaches its work or time limit is reported as incomplete.
