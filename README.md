# CHR

Start the local browser notebook:

```sh
cargo run --offline -- --notebook
```

Open the printed URL. Edit programs and queries as text or graphs, then run or step through execution. History recording is optional and off by default. Saved answers remain in this browser; execution handles and recorded states last until released or the server stops. Use `--port PORT` to choose a port.

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

Run the checks with `cargo test --offline`.
