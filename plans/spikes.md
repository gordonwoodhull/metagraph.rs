# Implementability spikes — index and results

Eleven throwaway spikes validating the riskiest claims in milestones 1–4
before those milestones are implemented (nine from the original brief,
plus 10 and 11 added by consolidation-review decisions; model tier
chosen per task — spike 11 ran flash-tier over hand-written emission).
Method: subagents build each answer as a compiling scratch crate under `/tmp`
(the repo stays read-only), each maintaining a `REPORT.md` handoff in its
scratch root. The output of a spike is answers and target code shapes,
not code to keep; findings are folded back into the milestone plans as
they land.

## Status at a glance

| # | Spike | Milestone | Status |
|---|-------|-----------|--------|
| 1 | Compose chain, end to end | M2 | complete — folded in |
| 2 | Nested-store codegen by hand | M2 | descoped — answered by spike 1 |
| 3 | Edge-chain refinement, compiled | M2 | complete — folded in |
| 4 | Structural validation checks as plain functions | M2 | complete — folded in |
| 5 | `intrusive_list` through a `backing!`-style courier | M3 | complete — folded in |
| 6 | CSR-style two-phase backing by hand | M3 | complete — folded in |
| 7 | Flat-`Digraph` transaction by hand | M4 | complete — folded in |
| 8 | Generic read→write transform | M4 | complete — folded in |
| 9 | Removal + compaction + id-remap report | M4 | descoped (below) |
| 10 | Root-arena drop from flat codegen | M1/M2 | complete — folded in |
| 11 | Keyed lookup over path ids | M2 | complete — folded in |

## The spikes

**1. The compose chain, end to end.** Extend the review's minimal courier
bounce to a real composed pattern: two `Digraph` instances plus
`outer.Node <=> sub.Subgraph`-style colocation, marker re-minting, and
`implement!` on the composed result. Question: does fusion-class
computation + placement codegen work when real schemas flow through the
chain? *Result:* **yes** — the chain collected three member schemas
hop-by-hop, the final hop minted per-instance markers and emitted the
composed courier in the unchanged rule shape (so `implement!` needed no
changes), and the generated two-level store compiles and traverses,
including an unmodified generic `bfs` over composed handles (16 tests).
Findings folded into the plans: trait method/GAT names come from the
root's declaration, not the instantiation alias (E0407 otherwise — the
one real bug); placement is a per-instance decision expressed as
`Vec<Data>` fields on the host record, with path-shaped constructors,
ids, and hooks; generated type names needed a collision rule; the
composed schema needed its own courier grammar; the multi-root store
entry point resolved to one instance-prefixed accessor per unfused root.
Recorded limits (the milestone's frontier, in full): embedding depth 1;
intrusive arrows sourced at embedded classes or unfused roots rejected;
field backing into an embedded class rejected (its id needs a host path);
arena arrows into an embedded class hosted by another class rejected;
`<=>` transitivity exercised only on two-member classes; constructor
argument name collisions across instances of one fusion class silently
deduped (keeps first); further composition of a composed pattern untested.
One recorded limit was later retired by spike 10 (the flat/composed
root-arena asymmetry); the insert-op naming deviation was resolved back
to the `insert_<arrow>` convention (M1).

**2. Nested-store codegen by hand.** Hand-write the code
`__implement_gen!` should emit for a Hierarchy-like store, as the codegen
target spec. *Descoped:* the review already compile-verified index-path
handles in a nested sketch, and spike 1 then produced real nested code —
which *is* the target spec, empirically: embedded arenas as `Vec<Data>`
fields on the host record, path ids (`SubNodeId(OuterNodeId(0), idx)`),
nested-path hook addressing, per-instance prefixed names on collision,
one root accessor per unfused root.

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
belongs to multi-store layouts, which remain out of scope (the
multi-root store entry point itself was later resolved by spike 1 — one
instance-prefixed accessor per unfused root — so check 5's vocabulary
no longer tracks it). Check 6 (intrusive partition, runtime) was
designed and both
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
Question: what is the minimal bound set? *Result:* the claim holds — a
generic copy and a generic node-replacement transform compile and run at
two marker instantiations. A pure copy needs only associated ref types
with equality bounds (no GAT on the write side); the handle→ref mapping
enters exactly when the transform names old-store elements. New
load-bearing findings: handle keyability (`Copy + Eq + Hash`) is part of
the minimal bound set; signature-faithful copy must be written against
the backing's iteration-order annotation (collect-and-reverse under
intrusive_list); copy-into-empty and rewrite-deltas are distinct shapes
that must not be mixed. Folded into milestone-4.

**9. Removal + compaction + id-remap report.** Question: does the report
name ids users can act on? *Descoped:* spike 7's finished crate already
contains removal with compaction, dangler errors named in old ids,
old→new map round-trips, and survivor iteration-order checks (14 removal
tests) — spike 9's acceptance criteria were demonstrated there.

**10. Root-arena drop from flat codegen.** Question: is the uniform
"root-marked entity owns no arena, record, or id — only its handle" rule
cheap to adopt on the flat path (composed codegen already followed it)?
Built on spike 1's workspace (copied to /tmp/spike10-root-arena). *Result:*
yes — a ~38-line diff confined to flat codegen (the entity-kit loop, the
store's arena-field list, the per-entity count functions); 16/16 tests
green unmodified; nothing load-bearing (ids are per-entity arena lengths,
and the root record was never constructed). Sole API casualty:
`graph_count()`, which always returned 0 and was unused. Also assessed:
flat/composed unification of entity emission is *not* a small follow-up
(flat is per-entity with bare `u32` ids; composed is per-fusion-class
with placement and compound ids) — the shareable pieces are the
handle/iterator emission templates and the near-verbatim duplicated
backing-binding validation loop. Folded into M1's naming rule and M2's
placement rule.

**11. Keyed lookup over path ids.** `hash_index` targeting an embedded
class stores target ids the way field backing does — exactly the
configuration spike 1's codegen rejects for fields, since the target's id
is a host-anchored path (`SubNodeId(OuterNodeId(h), i)`). Question: can
the keyed backing's map, its insert hook, and the keyed read method
address embedded targets through path ids, and does lookup hold up
through the nesting? *Result:* **yes** — hand-written emission of the
Hierarchy store with the keyed arrow bound, 5 tests green: path ids
derive `Copy`/`Eq`/`Hash` unchanged and work as map values; the map sits
on the host record beside the embedded arena; the insert hook takes
`&mut` store state (the arena append and map insert cannot share one
`&mut` host borrow — M1's hook contract already implies this shape);
`<->` collapses the key type to the partner's plain id. Key uniqueness
looked free with `HashMap` (corrected below: a plain insert silently
overwrites, which made duplicate-key rejection an insert-time rule in
M2/M4); validate() checks key existence and value-in-arena. One
codegen rule pinned: handles hand-write `Eq`/`Hash`
forwarding to (store pointer, id) — a derive would bound
`Store: Eq`/`Hash`, and id-only forwarding would equate handles across
stores (the spike's first cut said id-only; corrected against the
reference `arena.rs`, whose `std::ptr::eq` + id shape is what spike 8
made load-bearing). Macro-side keyed grammar and codegen remain
unverified. Folded into milestone-2; the Hierarchy keyed bindings are
un-gated. A consolidation-review probe (/tmp/m2-probe-edge-induced, 5
tests) later verified the other configuration — an unfused-root self: the
map sits at store level (M1's contribution contract), the insert op
drops the self argument, the root-generic associated-`Key` trait
dispatches at two key sorts, `add_subedge(me)` induces through id
getters, and a plain HashMap duplicate insert silently overwrites —
which made duplicate-key rejection an insert-time rule in M2/M4.

## Probes (review-session evidence outside the numbered spikes)

| probe | workspace | result |
|---|---|---|
| Combined ctor-collision rule (M2 review 4) | queued | One fused record, one shared-arg constructor (targets agree via `<->`), one instance-prefixed constructor (targets differ), asserting each field routes from the right argument. Covers the sharing half of the combined rule, which is plan-pinned only; the prefixing half is already covered by /tmp/m3-probe-ctor-merge |

## Sequencing rules (from the original brief)

7 before 8 and 9; 1 before 2's codegen version; 4–6 independent; 11
before any M2 Hierarchy test binds the keyed arrows. At most two spike
agents run concurrently.
