# Packaging verification

Offline builds and the small semantic runs passed. The older Git archive prerequisite remains unverified in this environment; the supplied repository path was unavailable during packaging.

- Current source: `git archive 294c010ebab311cf652d282398a3a2e38f85d4a5` succeeded from this repository.
- Older source: `/home/ahart/Documents/CHRLang` could not be opened. No accessible alternative repository containing the exact commit was found. To unblock a complete `reproduce.py` invocation, supply a local Git repository containing `4ed9c045dc4eccfca58fb25e39a4f13f46a1a223` via `--older-repo`.
- Build/semantic verification used the experiment's frozen older tree after verifying all 1,450 older source files against its recorded SHA-256 inventory. This checks the packaged manifests, locks and code against the recorded source; it does not replace verification of the older Git extraction. The production reproduction script has no snapshot fallback.
- Both harnesses built using `cargo build --offline --locked --release --bins`; search also built with `--features cow`. Existing standalone harnesses emit unused-code/import warnings; no build errors occurred.
- Ordinary qualification passed nonbinding, alias-wakeup, duplicate-occurrence, fresh-local and ordered-self-join fixtures. Sparse8 ran current and older Active with one measured repetition plus warmups, exact tuple/identity verification and tracing off.
- Search common-wide8 ran current/Active/Global in default and arena-cow builds, with the built-in semantic fixtures and separate traced references. Exact answer multisets, identity, ports and qualified application counts passed. Each build produced three measured rows.
- Bundled CSV reconstructed TABLES.md and summary.json byte-for-byte: 2,457 execution rows, 45 traced-reference rows and 1,638 fresh-preparation rows passed consistency checks. No historical matrix was rerun.
- Python scripts compile. Small-run and reconstruction outputs are retained in checks/. Original historical samples remain under results/.

Packaging work ran in `/tmp/chr-durable-validation-smoke`; this location is evidence provenance only. The reusable scripts accept a new output directory or use the system temporary directory. Both smoke builds/runs are complete; no build is left running.
