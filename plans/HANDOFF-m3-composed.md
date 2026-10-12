# HANDOFF — M3 continuation: steps 7–9

**Status when written:** step 6 is COMPLETE on `worktree-m3-implement`
(worktree `.claude/worktrees/m3-implement`, branched at `9c3569c` = main).
Steps 2–5 were complete at tip `64de74a`; step 6 lands on top (see the
commit message). Full workspace green: 33 suites incl. trybuild;
`validate_failures` now runs 11 tests (was 6). Steps 3–6 ticked in the
plan; the milestone gate's fixed-table item is ticked. Nothing merged;
the user merges.

**Scope:** steps 7–9 per `plans/milestone-3-backings-and-stdlib.md` (the
plan is the contract; this file is the working state and the traps).

---

## What step 6 left behind

- `tests/validate_failures.rs` gained `mod corrupt` — corruption below
  the op boundary, the only driver of the uniform range pre-pass and its
  early return. **One `implement!` per submodule**: two `implement!`s in
  one module collide on shared class names (`OuterNodeId`, …) — the class
  items emit per module. Three submodules:
  - `corrupt::refinement` — `CorruptRefStore` (the Refinement
    composition); damages `inner_edge_arena[0].next_elems` and pins the
    link message + the early return (no `shared membership`/`repeats`
    text in the same report).
  - `corrupt::hierarchy` — `HierIntrusiveStore`: the same
    `hier_comp::Hierarchy` composition with `sub.subedges:
    intrusive_list` instead of `arena`. This is the
    **intrusive-into-embedded driver** the port had never run: path-id
    links, records nested in the host's arena, the walk through the
    fused handle (`<OuterNode<'_> as
    EdgesOf<markers::sub::Subedges>>::subedges(&a)`), plus the
    path-aware pre-pass fixtures (dead host component with an
    in-bounds-looking leaf; in-bounds host with a dead leaf). The
    emission worked unmodified — no engine fixes this step.
  - `corrupt::relaxed` — `CorruptRootStore` (own `Relaxed`
    composition over `neg::RootMany`); damages the store-level head
    `first_cs` and pins the `head of the store` message variant.
- **Field-name correction to the step-6 design note**: `subedges`' link
  field is `next_subedges`, not `next_sub` — `link_stem` strips one
  trailing `_edges` and `subedges` has no underscore, so the stem is the
  full arrow name. Same for the head: `first_subedges`.
- `src/lib.rs` crate docs: the canonical-inventory paragraph is now the
  documented contract — per-class inventory (`field`/`arena`/
  `intrusive_list`/`hash_index`) plus the pre-pass's scope (heads and
  links, record-level and store-level, path-aware host-anchored ids) and
  its report-everything-then-return-early semantics.

## Step 7 — constraint predicates + trybuild error tests

Contract: the plan's "Constraints and applicability" and its evaluation
table.

- **Predicate evaluation with the pinned templates.** Cardinality exists
  (both codegens, byte-pinned). To add: `existence` (a primitive `many`
  arrow bound to a `membership: existence` backing is rejected — today
  nothing reads `Clause::MembershipExistence`; also the duplicate case, a
  second existence binding on one target naming both arrows),
  `hosts_arena` and `direct_records` upgraded from commit 3's
  plain-worded rejections to the table's template style (error names the
  arrow, the computed placement facts — B's arena location, A's self
  case, endpoint placements — and quotes the backing's tokens). The
  evaluation hooks are already in codegen_composed's applicability loop;
  flat has the orchestrator check only — bring flat up to parity.
- **`ctor_arg`/`on_add` orchestrator-rule rejection**: exists in both
  codegens; the trybuild fixture (`impl_unfused_root_field`) is composed
  — a flat fixture is owed.
- **Trybuild fixtures for step 1's grammar rejections** (unknown clause,
  bad store label, closed annotation sets, repetition): today they are
  unit tests in `backing.rs` via `parse_str`; add compilefail fixtures
  with real declaration-site spans (one `backing!` per rejection in
  `tests/compilefail/`, `.stderr` via `TRYBUILD=overwrite`).
- **The open fixture debts (from the prior handoff):**
  1. backing **re-export routing** rejection — mirror the pattern side's
     `tests/downstream/reexporter`: a middle crate re-exports the
     declaring module, the implement site depends only on the middle
     crate → the resolver's clean direct-dependency error. The mechanism
     fires in `GenInput::BackingStep` via `declaring_crate_root`.
  2. **Reserved-set shadowing** (hygiene caveat 3's grammar rule): the
     engine rewrites reserved heads unconditionally, so a body defining a
     local named `rec`/`handle`/`maps`/… is silently captured. Add a
     validation pass — e.g. `check_reserved_bindings(body) ->
     syn::Result<()>` in `roles.rs` rejecting `let`/`fn`/`struct`
     bindings of the reserved idents with the closed set quoted — called
     once per authored body before `roles::rewrite` (or per declaration
     at collection, cheaper). A trybuild fixture pins it.
  3. `hosts_arena`/`direct_records`/`existence` parse but are not
     evaluated — this step's main work (above).
- Also tick step 4's remaining box (the hygiene caveats as grammar
  rules) when the shadowing rejection lands — mixed_site scaffolds and
  absolutized paths are already emitted; shadowing is the missing third.

## Step 8 — `csr_adjacency` + type-state

- The declaration rides in `src/backings.rs` (bodies may be refined when
  emission lands — say so in the commit). `arena_len(A)` appears in its
  validate body and is NOT yet a role the engine rewrites — either grow
  the engine (a `records`- sibling role) or refine the body to the
  `b_arena_of`/`records` vocabulary; decide once and keep the closed
  vocabulary honest.
- `on store build`/`on store frozen` partition; the store type parameter
  emitted only when a phased backing is bound; the store parameter only
  in phased signatures; `compact` at the single `freeze()` in binding
  order; the three compile-fail proofs (insert-after-freeze,
  frozen-as-builder, no accumulators on the frozen type);
  applicability rejections (entity arrow, unfused-root self); the
  fused-root emission attempted against the frontier note — emit it or
  record exactly what breaks.
- The pre-pass applicability checks already reject phased declarations
  cleanly (flat and composed); removing that rejection is step 8's first
  act.

## Step 9 — stdlib promotion + doc pass

- Promote M2's discovered roots into the stdlib (names as M2 resolved
  them; criterion = M2 suites unchanged modulo import paths): `EdgesOf`,
  the `main_*` one-arrows, the keyed pair. `m2-fixtures` keeps local
  declarations re-pointing at the stdlib, or the fixtures thin out —
  whichever the criterion prefers.
- The doc pass to M1 step-11's standard (generated-code doc comments
  carry the arrow that produced each item — the port already emits
  per-contribution docs naming `inst.arrow`; audit flat too).
- The doc-table prose in `macros/src/lib.rs` (the four built-in names in
  `implement!`'s docs) is step 9's reword, deliberately not commit 4's.

## Verification commands

- `cargo test --workspace` — must be fully green at every commit (33
  suites at the step-6 tip; `validate_failures` runs 11 tests).
- `cargo test -p metagraph-exper --test koenigsberg` — byte-sensitive
  traversal-order assertions.
- `cargo test -p metagraph-exper --test validate_failures` — the pinned
  failure-message tails.
- `cargo test -p metagraph-exper --test compilefail` — pinned stderr;
  regenerate with `TRYBUILD=overwrite`.
- `MG_DEBUG=1 cargo test -p metagraph-exper --test <t>` — dump
  `implement!` / `__implement_gen` emissions when debugging.

## Known traps (two cost a debug cycle this stretch; all repeatable)

1. **Courier hops are brace-invoked with the paren group nested**:
   `courier! { ( tgt ; rest ) }`. Emitting `courier!(tgt ; rest)` makes
   the parens the invocation delimiter and the matcher mis-reads — "no
   rules expected" plus a misleading item-position error. Every hop
   emitter in `lib.rs` is the reference shape.
2. **`Span::mixed_site()` must pair on BOTH sides** — emitted bindings
   (hook params, scaffold lets, Walk struct FIELDS) and the engine's
   re-emitted mentions. One-sided mixed_site breaks resolution with
   E0425 against a *correct-looking* expansion. Corollary (how the bug
   class manifests): a local the engine FAILS to scope-bind gets
   silently re-emitted mixed-site and stops pairing with its verbatim
   binder — if a body local "cannot be found", suspect a missed scope
   binding first (see trap 7).
3. **`handle(x)` mints bare; bodies author `Some(handle(id))`** where
   the tail must be `Option`. The `.then(|| …)` recipes stay unwrapped.
4. Flat/composed `maps(A)` owner is `()` at plain and store-level
   placements — the "on {:?}" message tail shows `()`. Only composed
   tests pin messages, all via `contains`; don't "restore" a self id
   without making `host_of(v) != owner` typecheck.
5. A `backing!` invocation cannot carry doc comments before the
   declaration name; use `//` comments. Declare in a named module
   (E0255-avoidance, same as patterns).
6. **`rw_for`'s source scan counts call parens**: `m.iter()` scans as
   FOUR tokens (`m . iter ()`) — the MapIter arm matches
   `src.get(0..=3)` with an optional trailing EMPTY paren group. Any new
   source-shape match must account for the empty call parens riding
   along. (The original `len == 3` arm never fired; keyed validate was
   simply never run through the engine before this stretch.)
7. **Tuple loop patterns keep their idents inside the paren group** —
   `pat_idents` flattens exactly one level, so `for (k, v) in …` binds
   k/v like `for v in …` binds v. Deeper nesting would need more; none
   of the declarations use it.
8. **Debugging hygiene bugs**: the MG_DEBUG emission dump stringifies
   spans away — a hygiene bug reproduces CLEAN when the dump is compiled
   standalone (the /tmp probe trick), which proves it's span-sensitive.
   Then `cargo +nightly rustc -p metagraph-exper --test <t> --
   -Zmacro-backtrace` names the expansion the bad token lives in; from
   there, temporary `eprintln!`s inside `rw_for` (bindings firing per
   loop) localize it in one run. Remove the eprintlns after.
9. **Two `implement!`s in one module collide on class names** (new this
   stretch): each implement emits its classes' items (ids, Data, handle
   types, iterators) into the implementing module, un-prefixed by store
   name. Two compositions that share entity names (`OuterNode`, …) in
   one module → E0428/E0119 floods. One `implement!` per module; the
   corruption tests sit in the same submodule as their declaration
   (record fields are private to it).
10. **Guess field names from the naming formula, not the arrow's
    spelling** (new this stretch): `link_stem` strips one trailing
    `_edges` — `elems → next_elems`, `out_edges → next_out`, but
    `subedges → next_subedges` (no underscore; the stem is the whole
    name). Verify with MG_DEBUG before writing damage against generated
    fields.
