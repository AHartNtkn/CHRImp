# Rehearse 007: bounded adaptive tail pages

**Recommendation: KEEP the implemented partial slice.** Baseline `d9a103a2f739b602c90ec1f98a57767a123d3909`.
Implementation `7becf10` on `codex/opt/adaptive-pages`, worktree
`/tmp/chrimp-opt-round007-adaptive-pages`. This is the implementor's recommendation;
the campaign landing branch and campaign instructions/history are unchanged.

## Implemented scope

The generic persistent Store packs two through eight sorted scalar entries from
one aligned eight-key interval in the final key word. Pages share the first
three words, so exact three-word prefix dependency witnesses survive unrelated
updates. Sparse boundaries retain ordinary crit-bit branches. Page COW preserves
immutable old roots and weak witnesses; scalar and batch edits preserve no-op
identity, last-write-wins behavior, exact get/range/count and distinct keys.
Removing all but one entry restores a leaf. Crossing an interval creates a
crit-bit boundary; neighboring eligible leaves merge into a page. There is no
global repacking pass.

This is **not** arbitrary high-fanout prefix paging, general B-tree splitting,
or a reduction in joins or logical rule work. Pages do not cross three-word
prefixes, and most sparse indexes are unchanged. Entries use Vec storage with
at most eight logical entries and ordinary bounded capacity growth, not an
inline fixed-size array. Cursors retain their page position in the existing
traversal stack. Filters retain a resumable Page frame and stage changes through
a traced persistent result root. Collection emits one scalar payload per tick,
including during archive registration; stale/frozen/foreign checks precede work.
Page destruction contains no recursive child references. Existing deferred
branch release still frees at most two records per release tick.

## Evaluation and actual gains

The unchanged maintained `measure`/`perf.py` path completed **250 validated
samples: five per side at 25 points**, listed in `run.py`. All requested selected
workloads ran, including fresh-contract 64/256 in grouped/interleaved/reverse
orders, graph-bits 7, duplicate-heads 4, partial-join-hit, rewrite 128, behavior
I/S, lambda 2, fairness, pending snapshots/cancel, runtime replay and conditional
retained readers. No rwLog runs. `measurements.json` preserves every metric,
varying range, missing field and first-sample endpoint; `raw-samples.json.gz`
preserves all samples and configurations. Values below are medians.

| Workload | Sampled graph peak, baseline → pages | Graph record allocations | Cleanup ticks |
|---|---:|---:|---:|
| wide-rewrite 4, arity 64 | 2,107 → 1,778 (-15.61%) | 13,336 → 12,824 | 1,098 → 842 |
| wide-rewrite 16, arity 64 | 8,590 → 6,363 (-25.93%) | 64,352 → 62,304 | 4,242 → 3,218 (-24.14%) |
| lambda 2 first answer | 273,441 → 257,903 (-5.68%) | 631,552 → 612,601 (-3.00%) | 539,339 → 529,740 |
| fair-grow 4, finite chain 8 | 7,170 → 5,610 (-21.76%) | 63,730 → 60,546 | 25,945 → 25,165 |
| duplicate-heads 4, groups 3 | 551 → 497 (-9.80%) | 1,483 → 1,469 | 377 → 350 |
| partial-join-hit 32, probes 4 | 1,167 → 1,055 (-9.60%) | 3,241 → 3,188 | see raw records |

Wide16 still performs 64 applications, delivers one answer with 32 facts,
2,048 ordered ports and 3,138 scalar events. Lambda still reaches its independently
validated first answer after 5,339 applications and 83 scalar events. Fair-grow
still delivers its finite sibling after 1,029 applications. Source per-rule
counter medians agree across all 25 paired points. Fresh64/256 perform exactly
2,048/8,192 applications in every order and validate all fresh identities,
intended contractions and residual multiplicities. Separate successful alternatives
remain separate, including all 256 empty answers.

The removed work/storage is concrete: dense entries no longer require separate
leaf/branch records, and fewer child records reach deferred release. The focused
256-entry dense test rejects the baseline's 511 records with a <128-record
bound and validates unchanged snapshots and every tested range against BTreeMap.
Cleanup ticks accompany this structural evidence; lower source ticks alone are
not an acceptance argument. Pages return entries separately from node visits,
so their entry work must be included before interpreting cursor diagnostics.

## Displaced costs and acceptance assessment

| Workload | Execution requested bytes | Whole-process requested peak |
|---|---:|---:|
| wide4 | 1,959,208 → 2,079,560 (+6.14%) | 409,821 → 393,987 (-3.86%) |
| wide16 | 9,471,488 → 10,091,152 (+6.54%) | 1,417,779 → 1,369,955 (-3.37%) |
| lambda2 | 4,903,956,416 → 4,910,781,192 (+0.14%) | 163,933,213 → 162,937,653 (-0.61%) |
| behavior I | 13,918,860 → 14,005,532 (+0.62%) | 1,962,332 → 1,962,858 (+0.03%) |
| behavior S | 13,387,164 → 13,468,308 (+0.61%) | 1,719,223 → 1,718,623 (-0.03%) |

Wide16 execution allocation calls increase 69,750 → 73,746 (+5.73%). Graph
mutation visits increase 68,601 → 72,697 (+5.97%): unique visits stay 29,206,
copies grow 39,395 → 43,491. Its page counters charge 3,072 page record
allocations, 6,144 copied entries, 6,144 writes, 1,536 sparse-boundary splits,
3,072 merges and zero page-cursor returns. Each sorted write may additionally
shift up to seven entries. Lambda graph mutation visits grow 512,697 → 522,098
(+1.83%), with 25,397 copied entries and 160,553 page-cursor returns. Thus this
slice does not claim reduced total mutation work or allocation traffic.

The maintained comparator qualifies both wide16 storage/cleanup decreases and
allocation increases. Lambda's graph allocations, graph/byte peaks and cleanup
decrease, and its small allocation-byte increase, are qualified; allocation-call
change remains uncertain. See both `comparison-*.json` files. Runtime is diagnostic
only; compilation and other work overlapped some runs, and there is no timing-based
acceptance or speedup claim. Object peaks are sampled; requested-byte accounting
excludes allocator fragmentation and is not RSS.

Under the supplied independent-dimensions acceptance rule, the bounded extra
page copying/shift and allocation costs do **not** outweigh the demonstrated
representation, retention and release gains. Wide16 removes 2,227 sampled live
records and 1,024 cleanup steps at the same completed output; lambda removes
18,951 record allocations and 15,538 sampled live records with a 0.14% execution
byte cost. Added implementation complexity is the bounded page handlers and
their cursor/filter/collector continuations, not a second tree or GC authority.
These are useful partial gains despite the absence of a join-work improvement.
The allocation increases are real costs, not hidden or dismissed by a total-byte
gate. No significant semantic-work, progress or scaling regression was found
in the measured controls; this is not a claim about every workload.

Fresh64/256 graph peaks stay 22,016/88,064: no fresh-workload storage gain is
claimed. Their execution-byte overhead stays roughly 0.35–0.40% and requested
peak overhead roughly 0.44–0.51%, across sizes/orders. Collection work redistributes:
fresh64 grouped graph-collection steps grow 49,572 → 55,821 while parked scanning
falls 31,729 → 26,385 and variable seeding 4,036 → 2,759. Individual collection
phase counts are not independent semantic obligations or universally deterministic.
Rewrite128 and empty-answer controls have unchanged graph work and small traversal/
diagnostic storage overhead. Duplicate-heads execution bytes/calls grow 1.18%/1.28%.

## Full lifetime charges and limits

Preparation, engine admission, delivery/validation, retention, cancellation,
cleanup and final destruction are included by the maintained runners. Core setup
adds 144 requested bytes for the three Store statistics blocks; cleanup commonly
adds another 144 when replacing stores. Page/filter/cursor metadata increases
other executing allocations. Validator bytes are unchanged on the displayed
core/notebook cases. Delivery is inside source work plus validator attribution,
not an uncharged zero-cost phase. Final report serialization adds `other` traffic;
the new page diagnostic fields also cost report storage.

All four conditional reader cases use four owners/siblings, width 64 and 64
continued applications. Their exact retained graphs and choice pins validate.
Archive and inspector projection performs zero extra source applications; held
output deliberately resumes its source while delivering the finite answer.
Archives/inspectors themselves do not gain page storage in this slice. Their
execution-byte overhead is 0.04–0.78%; projection adds 432 bytes per run. Held
output execution bytes grow 0.14%, graph allocations fall 3,336 → 3,130, and
requested peak grows 0.87%. Runtime replay validates unchanged output and final
spool release; execution/cleanup byte overhead is 0.61%/0.62%.

Pending snapshot/cancel 64 validates the exact unposted prefix and all retained
syntax after cancellation. Source allocation calls are unchanged; engine bytes
grow 2,048 and requested peaks about 2,598 bytes. All store tests reclaim zero
records after final owners disappear. Engine lifecycle oracles reclaim graph,
condition, history, pending, inspection and release payloads; an initialized
coordinate sentinel and harness infrastructure can remain until engine/process
destruction. Process requested bytes are therefore not asserted zero at report
emission. Final owner/destruction checks stay enabled.

## Validation and reproduction

Builds were separate from test executions. Native all-target binaries passed
470 tests plus two socket tests; diagnostics binaries passed 496 plus two socket
tests. Every test binary invocation used `timeout --kill-after=5s 60s`; no test
timed out. The three maintained observation-mode equivalence checks passed.
Production clippy with `-D warnings`, fmt and diff checks pass. All-target clippy
passes with only `clippy::overly_complex_bool_expr` allowed: the existing Boolean
truth-table oracle in `tests/condition.rs:33` intentionally triggers that lint.
Initial localhost tests were denied by the sandbox and both native/diagnostics
variants passed with localhost access. No product fix was needed for them.

The dense storage test was run before production edits and failed at its storage
bound; its semantic/filter companion passed on baseline. Existing binary-tree
node-count assertions were updated for packing. The prior allocation-bound test
subtracted live nodes from cumulative allocations; it now compares allocation
counters. The filter per-tick check likewise measures allocations rather than
unsigned subtraction of live counts, which may decrease during a page release.

`run.py` initially used the unsupported order spelling `reversed`; all five
baseline invocations rejected that input before execution. `invalid-order.json`
records that harness error. Corrected `reverse` cohorts completed on both sides;
the 250-sample archive includes only the validated, correctly configured cohorts.
No unsuccessful or incomplete semantic run is relabeled successful.

Commands: `cargo build --offline --release --features diagnostics --example measure`;
save baseline binary before editing; run `python3 <this-directory>/run.py BASELINE
CANDIDATE OUTPUT`, then `python3 <this-directory>/collect.py OUTPUT`.
The two comparator JSON files preserve their exact metric selection. Test build
commands were `cargo test --offline --release --all-targets --no-run` with and
without `--features diagnostics`; compiled binaries were then invoked separately.
Toolchain: rustc 1.95.0-nightly (18d13b533), x86_64 Linux. Baseline executable SHA256
`e2a8ec20779b29ebf95c37f00574ac1d7183ad796b162c236f02b18b8d348116`;
measured candidate SHA256
`4723392db84d6b79a69b1e96bb081e1ad6be669f3983ad7ec5ee68916fe6813f`.
Measurements preceded the implementation commit; subsequent production edits
only collapsed an equivalent nested `if` for clippy. Test/documentation changes
do not affect the measured binary. No unimplemented ambition is included in KEEP.
