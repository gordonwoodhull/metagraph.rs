# HANDOFF — M3 continuation: steps 6–9

**Status when written:** the fused steps-2–5 sequence is COMPLETE on
`worktree-m3-implement` (worktree `.claude/worktrees/m3-implement`,
branched at `9c3569c` = main). Tip `64de74a`; commits this stretch:
`7b3202a` (prior handoff), `aab753f` (commit 3 — composed codegen
data-driven over `backing!` declarations, see its message for the two
recorded decisions and the two role-engine fixes), `64de74a` (commit 4 —
fixed-table inspection, step-5 flat keyed test, plan ticks). Full
workspace green: 33 suites incl. trybuild and the three new tests
(`root_intrusive`, `flat_keyed`, the `impl_keyed_embedded_gap` compilefail
pair). Steps 3–5 ticked in the plan; the milestone gate's fixed-table item
ticked. Nothing merged; the user merges.

**Scope:** steps 6–9 per `plans/milestone-3-backings-and-stdlib.md` (the
plan is the contract; this file is the working state and the traps). A
step-6 start was attempted and reverted (tree is clean at tip) — its
design work is written out under step 6 below so it doesn't have to be
re-derived.

---

## What this stretch left behind

- `macros/src/roles.rs` — the role-substitution engine, now with
  `walk_host()` on `RoleBinding`: the Walk's reserved `host` field at
  embedded-target placements (the initializer a `Walk { .. }` literal
  carries, and the anchor a raw `handle(x)` mints through; call-site
  spans on both sides — only authored Walk FIELD names are mixed-site).
  Two latent bugs fixed there (see traps 6–7): tuple-pattern binding and
  the MapIter source scan. `FlatRoles` (flat) and `ComposedRoles`
  (composed, in `codegen_composed.rs`) are the two bindings; read either
  as the template for a new one.
- `macros/src/codegen_composed.rs` — fully declaration-driven: field
  contributions via the free fn `push_contribution` (naming formulas,
  collision prefixing, the combined ctor rule, the uniform init rule),
  hooks/read/lookup/iterate/advance from authored bodies, sums via
  rewritten sibling iterate bodies (`sibling_iterate_expr`; the old
  hand-inlined `sum_sibling_expr` is gone), per-arrow
  `__validate_<inst>_<arrow>` methods with the uniform `Option<id>` range
  pre-pass and early return, summoned from `validate()` in binding order.
  Applicability checks are declaration-driven: phased (until step 8),
  `CtorArg`/`OnAdd` orchestrator rule, One-cardinality-into-embedded
  handle mint, declared `requires:` predicates (`direct_records` →
  embedded self, `hosts_arena` → cross-host), validate-variant selection,
  `by`-totality, and the keyed host-agreement gap (below). `rec_ref`
  (store-token-parametrized record access) replaced `rec_ref_ts`/
  `rec_ref_handle`; `rec_get`, `root_map_of`, and `ClassData.maps` are
  gone — per-arrow registries (`field_of_arrow`, `map_of_arrow`) name
  contributed fields instead.
- `macros/src/codegen.rs` — flat codegen unchanged except its declaration
  helpers are now `pub(crate)` and shared: `card_of_decl`,
  `contributed_fields`, `on_clause`, `body_of`, `FieldInit`/`init_of`,
  `scaffold_lets`.
- Keyed decisions recorded: the **host-agreement gap** — `host_of(v) !=
  owner` binds only where a self-side anchor of the host id type exists
  (self record IS the host, or self shares the target's host); anywhere
  else it is an implement!-time rejection quoting placement facts (folded
  into the plan's Constraints section; pinned by
  `tests/compilefail/impl_keyed_embedded_gap.*`). The **intrusive-at-
  unfused-root relaxation** is emitted and driven by
  `tests/root_intrusive.rs` (store-level head, self-less op, root-handle
  read; the flat side was the r3 probe's).
- `tests/fixtures` — three new patterns: `shapes::KeyedPlain` (flat
  keyed, plain-self + store-level arrows; driven by
  `tests/flat_keyed.rs`), `neg::AnchoredKey` (the gap fixture's key),
  `neg::RootMany` (root-sourced many arrow; the relaxation driver).
- The keyed live-key predicate (`__<inst>_<arrow>_key_live`, M4's pure
  check form) is derived from the authored lookup body (`.is_some()`).
- Arena iterators no longer emit the hand-rolled `size_hint` — the
  advance body owns `next()` entirely, and nothing in the suites reads
  size hints.

## Step 6 — validate machinery + corruption fixtures

Contract: the plan's "The validate contribution".

- **Box 1 is already done** — the `__validate_<arrow>` convention (per
  arrow, doc-hidden, summoned in binding order) landed with the port in
  both codegens; tick it first.
- **Pre-pass fixtures (the new work).** The existing
  `tests/validate_failures.rs` covers ghost keys, value-in-arena,
  host-agreement, shared membership, cycles, and the by-contradiction +
  coverage pair — all through public ops. What no fixture pins is the
  **uniform range pre-pass and its early return**: the ops cannot put a
  link out of range (hooks only ever insert live ids), so corrupt
  directly. The generated records are private to the implement module —
  put the `implement!` and the tests in ONE module inside
  `validate_failures.rs` (`mod corrupt { use metagraph_exper::implement;
  implement! { ... } #[test] fn ... }`) and the field damage is legal:
  `s.inner_edge_arena[0].next_elems = Some(InnerEdgeId(99));`.
  Pin both the message and the early return:
  - message: `arrow `inst.arrow`: `next_elems` link of InnerEdgeId(0)
    points at InnerEdgeId(99), which is not a live record` (the pre-pass
    names the field, the owning record's id, and the dead target; the
    head/store variants say `head` / `head of the store`);
  - early return: assert the SAME report carries no `shared membership` /
    `repeats` text — the authored partition walks never ran. This is the
    deliberate pin of the joint-report change the handoff-before-last
    flagged.
- **The intrusive-into-embedded emission has no driver yet.** A Hierarchy
  variant binding `sub.subedges: intrusive_list` (instead of `arena`)
  drives it: path-id links, the nested `rec(id)` walk, the non-raw
  `handle(id)` split (`{host: id.0, id: id.1}`), and `records(B)` as the
  embedded host×leaf enumeration. Walk it through the fused handle
  (`<OuterNode<'_> as EdgesOf<markers::sub::Subedges>>::subedges(&h)`)
  beside the corruption tests. It also gives the **path-aware pre-pass
  fixture**: corrupt a link's host component
  (`s.sub_sub_edge_arena[0].next_sub = Some(SubEdgeId(OuterNodeId(99),
  0))`) — host liveness must fire before leaf bounds are even computed
  (the test's out-of-range-host case is otherwise in-bounds-looking).
  NOTE: that Walk carries the reserved `host` field unread (the intrusive
  recipe takes the host from the path id — r4-advance pinned the field as
  reserved, not always read), so the field triggers a dead_code warning
  in the test crate; harmless, but don't "fix" it by dropping the field.
- **Field-name formula for damage:** `link_stem` strips one trailing
  `_edges` — `out_edges` → `first_out`/`next_out`, `elems` →
  `first_elems`/`next_elems`, `subedges` → `first_sub`/`next_sub`
  (`subedges` → stem `sub` — easy to misguess; check with MG_DEBUG).
- **Doc-contract box:** expand the canonical-inventory paragraph in
  `src/lib.rs` crate docs (currently one paragraph predating the
  pre-pass) to state the inventory per backing class and the pre-pass +
  early-return semantics — the documented contract M4's flush runs.
- Then tick box 1 and the corruption-fixture box (and the doc box).

## Step 7 — constraint predicates + trybuild error tests

Contract: the plan's "Constraints and applicability" and its evaluation
table.

- **Predicate evaluation with the pinned templates.** Cardinality exists
  (both codegens, byte-pinned). To add: `existence` (a primitive `many`
  arrow bound to a `membership: existence` backing is rejected — today
  nothing reads `Clause::MembershipExistence`; also the duplicate case,
  a second existence binding on one target naming both arrows),
  `hosts_arena` and `direct_records` upgraded from commit 3's
  plain-worded rejections to the table's template style (error names the
  arrow, the computed placement facts — B's arena location, A's self
  case, endpoint placements — and quotes the backing's tokens). The
  evaluation hooks are already there in codegen_composed's applicability
  loop; flat has the orchestrator check only — bring flat up to parity.
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
  suites at tip `64de74a`; grows with step 6's fixtures).
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
