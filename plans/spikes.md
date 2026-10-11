# Implementability spikes — index and results

Nine throwaway spikes validating the riskiest claims in milestones 2–4
before those milestones are implemented. Method: sonnet subagents build
each answer as a compiling scratch crate under `/tmp` (the repo stays
read-only), each maintaining a `REPORT.md` handoff in its scratch root.
The output of a spike is answers and target code shapes, not code to keep;
findings are folded back into the milestone plans as they land.

## Status at a glance

| # | Spike | Milestone | Status |
|---|-------|-----------|--------|
| 1 | Compose chain, end to end | M2 | **in progress** |
| 2 | Nested-store codegen by hand | M2 | descoped (below) |
| 3 | Edge-chain refinement, compiled | M2 | complete — folded in |
| 4 | Structural validation checks as plain functions | M2 | complete — folded in |
| 5 | `intrusive_list` through a `backing!`-style courier | M3 | complete — folded in |
| 6 | CSR-style two-phase backing by hand | M3 | complete — folded in |
| 7 | Flat-`Digraph` transaction by hand | M4 | complete — folded in |
| 8 | Generic read→write transform | M4 | **in progress** |
| 9 | Removal + compaction + id-remap report | M4 | descoped (below) |

## The spikes

**1. The compose chain, end to end.** Extend the review's minimal courier
bounce to a real composed pattern: two `Digraph` instances plus
`outer.Node <=> sub.Subgraph`-style colocation, marker re-minting, and
`implement!` on the composed result. Question: does fusion-class
computation + placement codegen work when real schemas flow through the
chain? *Status:* courier conversion to the target-parameterized rule done
(baseline tests green); the chain and composed codegen are under
construction. This is M2's principal implementability risk — the one
mechanism nothing else exercises.

**2. Nested-store codegen by hand.** Hand-write the code
`__implement_gen!` should emit for a Hierarchy-like store, as the codegen
target spec. *Descoped:* the review already compile-verified index-path
handles in a nested sketch, and spike 1's `implement!`-on-composed-result
produces real nested code empirically — a better answer to "what does
boring generated code look like" than a hand sketch. The question is
folded into spike 1's report.

**3. The edge-chain refinement sketch, compiled.** Chain pattern with
open `Elem`, intrusive `elems` arrow, reference fusion. *Result:* the
corrected sketch behaves — a chain walk is literally the `out_edges`
walk, and all structural checks pass under the one-root-store layout.
Found the intrusive-ownership hazard (silent truncation / divergent
traversal when one object joins two lists), which became validation
check 6, and the store-less-instance insert-op placement rule. Folded
into milestone-2.

**4. The structural validation checks as plain functions.** Over a
hand-written schema struct, no macros. Question: is the information
sufficient at the phase each check is assigned to? *Result:* mostly yes —
with two moves. Check 2 (no record contains itself) is vacuous at
compose time and moved to `implement!` time; check 5 (field homes) in its
non-vacuous form needs store-partition and reachability vocabulary that
turns out to be the same decision as the deferred multi-root entry-point
API. Check 6 (intrusive partition, runtime) was designed and both
corruption shapes reproduced. Folded into milestone-2.

**5. `intrusive_list` re-implemented through a `backing!`-style courier.**
Named iterator struct, user-ident hygiene across two macro layers.
Question: can arbitrary-Rust hook bodies survive the token channel
hygienically? *Result:* yes, with three caveats the grammar must enforce
(mixed-site spans are load-bearing; item paths resolve at the use site;
the hook-parameter binding contract must be written down). This was the
stated gate on fixing the `backing!` grammar. Folded into milestone-3.

**6. A CSR-style two-phase backing by hand.** Question: what do phase
constraints actually need to say? *Result:* a concrete annotation
vocabulary (`append_only`, `requires_compaction_at_boundary`,
`freeze_scope: per_store`, `read_precondition: post_compaction`,
`iteration_order`, `field_contributions: store_level_only`); iteration
order is now a per-backing contract annotation; M4's flush gained a
backing-finalization step. Left one open decision: how generated code
represents the build/frozen phase (type-state vs separate builder type
vs runtime flag). Folded into milestones 3 and 4.

**7. The flat-`Digraph` transaction, by hand.** `edit()`/`into_edit()`,
token-stamped refs with eager `Pending` checks, the change-set queue, the
flush pipeline including intrusive relinking. *Result:* the pipeline
produces the right store (Königsberg signature equality, 32 tests) — and
the spec had forgotten several things, most importantly that per-field
remapping of intrusive links silently truncates lists under removal;
chains must be rebuilt by walking old lists. Also pinned: the report
map shape, op-index attribution, the panic-vs-`FlushError` dividing line,
structural `validate()` for keyless backings, and `into_edit`'s
write-ordering constraint. Folded into milestone-4.

**8. The generic read→write transform.** Associated ref types and a
handle→ref mapping trait, compiled against spike 7's transaction.
Question: what is the minimal bound set? *In progress.* This is the only
proof of M4's central portability claim — a generic algorithm reading one
store and staging another without knowing either pattern.

**9. Removal + compaction + id-remap report.** Question: does the report
name ids users can act on? *Descoped:* spike 7's finished crate already
contains removal with compaction, dangler errors named in old ids,
old→new map round-trips, and survivor iteration-order checks (14 removal
tests) — spike 9's acceptance criteria were demonstrated there.

## Sequencing rules (from the original brief)

7 before 8 and 9; 1 before 2's codegen version; 4–6 independent. At most
two spike agents run concurrently.
