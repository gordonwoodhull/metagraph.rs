# HANDOFF — M3 Step 2: binding resolution in `implement!`

**Status when written:** step 1 committed at `7d84b0b` on
`worktree-m3-implement` (worktree `.claude/worktrees/m3-implement`,
branched at `9c3569c` = main). Full workspace green (27 suites,
including trybuild compile-fail fixtures). Step 1's boxes are ticked in
`plans/milestone-3-backings-and-stdlib.md`.

**Scope of this handoff:** step 2 as the plan pins it — the binding
path form, the backing-courier resolution hop, and the permanent
in-repo bounce gate. Read the "Reality check" section first: steps 2–5
fuse in implementation, and this handoff says how to sequence them
without leaving the tree red.

---

## What step 1 left behind

- `macros/src/backing.rs` — the full `backing!` declaration grammar:
  header (`name: A -> B, cardinality`), the round-4-pinned clause
  inventory with legality-per-cardinality, the repetition rule, all
  closed vocabularies quoted in rejection messages, re-emission
  (`backing_decl_tokens`, `packaged_backing_tokens`) with round-trip
  tests, and one rejection fixture per closed set.
- `macros/src/lib.rs` — the `backing!` proc macro emitting the
  target-parameterized courier `__metagraph_backing_<Name>` with the
  declaring crate's `CARGO_PKG_NAME` baked into a
  `("pkg", { decl })` payload, plus the E0255 doc constraint.
- **Flagged extension** (noted in the plan file): per-field naming
  formulas `= arrow | stem | suffix` (default bare), because the plan
  pins per-site formulas but shows no syntax. Authored bodies reference
  *logical* field names; codegen renames per binding.

Nothing consumes backing couriers yet. `implement!` still treats
bindings as bare idents and codegen still matches the fixed
`"field" | "arena" | "intrusive_list" | "hash_index"` table — that
table's deletion is the far side of steps 3–5, not step 2.

## Step 2's contract (the plan, verbatim anchors)

- Binding grammar grows the path form
  (`out_edges: stdlib::intrusive_list`); bare-ident stays legal and
  resolves against the runtime crate, where the stdlib's backings live.
- Resolution is the declaring-crate resolver, not a new mechanism:
  direct dependency enforced, renames honored, re-export routing
  rejected, clean `compile_error!`. The backing courier bakes
  `CARGO_PKG_NAME`; the implement hop maps it through
  `proc-macro-crate`.
- **The bounce gate, in-repo and permanent**: two distinct backing
  couriers resolved per one `implement!`-shaped invocation, bodies
  spliced hygienically, with cross-crate arms — a stdlib backing
  resolved from a user crate, a user backing declared in a second
  crate, a renamed dependency — plus the use-site helper-resolution
  arm (spike 5 caveat 2: a helper named in a hook body resolves at the
  backing's *use* site, so the gate's hook body must name a helper that
  only exists at the use site). This gate is the evidence for the
  resolution × hygiene conjunction; a throwaway probe was explicitly
  superseded (plan header), so do not substitute one.

## Reality check: steps 2–5 are one refactor in practice

The bounce gate requires bodies to be *spliced* — which requires
codegen to consume declarations. The working sequence that keeps the
tree green:

1. **Resolution machinery** (this handoff's core): `Binding` path form,
   backing queue + collected decls riding `ChainConfig`, backing hop
   parsing in `__implement_gen`. Land it with codegen still keyed on
   `decl.name` (a temporary match that maps the four stdlib names onto
   the old string-table arms). M1/M2 suites stay green; commit.
2. **Flat codegen data-driven** (field, arena, intrusive_list,
   hash_index) with the stdlib declarations in a new runtime
   `src/backings.rs`. M1 suite green; commit.
3. **Composed codegen data-driven** (same four; full self-case
   taxonomy, placements, validate family). M2 suites green; commit —
   this also completes steps 3–5's acceptance gates.
4. **Delete the fixed table** (the `["arena","field",...]` membership
   checks and name-keyed arms) — after 1–3 it is dead; the overall
   gate's "no hidden fixed table" item then holds by inspection.

## Concrete design decisions already made (do not relitigate without
## folding back into the plan)

- **Role substitution engine** (`macros/src/roles.rs`, to be written):
  textual token rewrite of authored bodies, outermost-role-first, with
  an *ambient var* defaulting to B (overridden when the inner expr is
  literally `a`/`b`/`k`). `host_of(v)`/`idx(v)` shapes are driven by
  the ambient var's class placement, not by v's spelling — that is
  what makes one authored body emit at every placement.
- **Scaffold bindings** codegen injects into bodies (closed set,
  document it in roles.rs): `errs` (the `Vec<String>` sink),
  `arrow_label` (`&str` like `` "arrow `sub.subnode`" ``),
  `det_label` (the determinant arrow's name, bound only in
  `validate by`). Spike 5 caveat 1: every scaffold ident emitted with
  `Span::mixed_site()`.
- **Validate emission** (round-4 convention): one doc-hidden method
  `fn __validate_<arrow>(&self) -> Result<(), String>` per bound
  backing, summoned from `validate()` in binding order, each returning
  one joined message; `validate()` concatenates. A uniform
  **range pre-pass** over contributed `Option<id>` fields is emitted by
  codegen *before* the authored body (derivable from the declaration —
  the (b)-class half), so authored walks may assume links in range and
  use `rec(x)` freely. Authored map checks use `idx()` comparisons and
  are panic-free on their own.
- **Authored bodies must reproduce the pinned failure-message tails**
  asserted in `tests/validate_failures.rs`: "key {:?} does not name a
  record", "value {:?} of key {:?} on {:?} is not a live record",
  "is anchored at the wrong host", "appears in more than one list —
  shared membership", "repeats {:?} — a cycle", "membership contradicts
  the determinant", "is missing from the list of its `target`". Use
  `det_label` for the determinant name; use `owner` from `maps(A)` for
  the self id so one body serves all three self placements.
- **Hook-maintained field init rule** (uniform, derivable):
  `ctor_arg`-fed ← the constructor arg; `Option<_>` → `None`;
  `Vec<_>`/`HashMap<_,_>` → `new()`; anything else hook-maintained is a
  codegen rejection (M3's five need nothing more).
- **Emitted hook/param names keep the authored spelling** (`a`, `b`,
  `k`) with mixed-site spans; `w` in `advance(w)` rewrites to `self`
  in the emitted `next()`; `handle(x)` mints the target handle —
  raw-index Walk fields mint `#Id(x)` (+ reserved host at embedded
  placements, per the r4-advance probe), id-typed exprs path-split
  (`host: x.0, id: x.1` embedded; `id: x` flat).
- **Vacuous host** at flat/store-level placements: `()` — the
  hostagree probe used a `NoHost` struct; `()` typechecks the same
  (`() == ()` vacuously true) and needs no emitted type.
- **Intrusive at an unfused root** becomes DSL-legal in this refactor
  (store-level head, self-less op, root-handle read — the merge chore
  names this relaxation as M3's job). The composed
  intrusive-at-embedded rejection (`impl_intrusive_at_embedded.rs`
  fixture) stays. `field` at an unfused root stays rejected
  (`impl_unfused_root_field.rs` — the `ctor_arg` orchestrator rule).

## Resolution machinery details (step 2 proper)

- `grammar.rs::Binding` gains `backing_path: Option<Path>`;
  `parse_bindings` takes a path when it sees `ident ::` or leading
  `::`; `bindings_tokens` re-emits it. Round-trip test extends the
  existing `bindings_parse_and_roundtrip`.
- `ChainConfig` (macros/src/lib.rs) gains `backing_queue:
  Vec<BackingHop>` and `backing_decls: Vec<(String,
  backing::PackagedBacking)>`, where `BackingHop { key: String,
  courier: TokenStream2 }`. `key` is `b.key_str()` — the same string
  `stage1` already dedupes bindings by.
- `chain_config_tokens` serializes `@backings [ (key, path),* ]` and
  `@bdecls [ (key, {packaged}),* ]` after `@meta`; `parse_chain_config`
  parses them (empty lists default — old couriers in flight never
  happen within one crate graph, but keep the parse tolerant).
- `GenInput` gains `BackingStep { packaged: backing::PackagedBacking,
  config: ChainConfig }`: input shape `{ ("pkg", {decl}) } ; @config
  { .. }` (the backing courier wraps its payload around the same
  payload spine patterns use).
- `advance_chain` drains `backing_queue` first, then `roots`:
  - bare-ident binding → courier path
    `runtime_target()::backings::__metagraph_backing_<name>`;
  - path binding `p::..::X` → `p::..::__metagraph_backing_X`.
- The backing hop enforces the direct-dependency rule: call
  `declaring_crate_root(&packaged.pkg)` and discard the result — a
  backing whose declaring crate is not a direct dependency fails here
  with the resolver's clean error (same text patterns use). This is
  also what rejects re-export routing for backings reached through a
  re-exporting middle crate.
- Stage 1 (flat and composed branches) builds the queue from bindings
  *after* the existing binding checks, so error spans/messages for
  unknown/duplicate bindings are unchanged (trybuild fixtures pin
  them).
- **Stdlib backings module**: new runtime `src/backings.rs` declaring
  the five in `backing! { .. }` blocks inside `pub mod backings`-era
  named module (never a crate root — E0255). `src/lib.rs` adds
  `pub mod backings;` and re-exports the macro:
  `pub use metagraph_exper_macros::backing;`.

## The bounce gate (step 2 acceptance)

New downstream crates under `tests/downstream/` (workspace members),
mirroring the M2 pattern:

- one crate declaring a user backing via `backing!` (in a named
  module), whose `on_insert` body names a free helper function defined
  at the *use* site of the backing (spike 5 caveat 2's arm);
- a second crate depending on the first **renamed**, implementing a
  small pattern with (a) the user backing via path form, (b) a stdlib
  backing via bare ident, in one `implement!` block — two distinct
  couriers, one invocation;
- assertions: the built store behaves (insert + traversal order +
  `validate()`), proving resolution and hygienic splicing together.
- The stdlib-resolved-from-a-user-crate and renamed-dependency arms
  already exist implicitly for every downstream crate once the stdlib
  backings are courier-resolved — the M2 downstream crates
  (`basic`, `renamed`, `own-root`, …) become those arms for free; keep
  them green.

## Verification commands

- `cargo test --workspace` (must be fully green at every commit;
  includes trybuild).
- `cargo test -p metagraph-exper --test koenigsberg` — M1's
  byte-sensitive assertions (reverse-insertion orders).
- `cargo test -p metagraph-exper --test validate_failures` — the
  message tails listed above.
- `MG_DEBUG=1 cargo test -p metagraph-exper --test koenigsberg` —
  dump `implement!` / `__implement_gen` emissions when debugging
  courier hops.

## Known traps

- **Courier transcribers match `$($fwd:tt)::+`**: the forwarding target
  must have no leading colons — use `runtime_target()`, never
  `runtime_path()`, in hop positions (existing helper, same trap as
  M2).
- **Config payloads must round-trip as token groups** through every
  hop: never re-tokenize the schema or the packaged backing in flight;
  parse only at `__implement_gen`.
- **Span degradation**: by codegen, spans are the call site; error
  messages must quote tokens (the plan's predicate-error templates),
  not rely on spans. Binding-grammar errors still have real spans —
  keep them in stage 1.
- **`extern crate self as metagraph_exper`** in src/lib.rs is what lets
  in-crate expansions resolve `::metagraph_exper` — the bare-ident
  backing form leans on it too.
- The `key(K)` slot of a keyed arrow binds the *sort*; `KId` in backing
  bodies is that sort's id type — the r3-hashindex probe (in /tmp)
  shows all three self placements if shape examples are needed.
- Probes surviving in /tmp with pinned shapes: `m3-probe-r4-advance`
  (Walk + advance + handle), `m3-probe-r3-hostagree` (maps/owner/
  host_of), `m3-probe-r3-root-intrusive` (store-level head, self-less
  op — note its validators are free functions there; the round-4
  convention overrides to methods), `m3-probe-mixed-phase` (CSR
  type-state, step 8's shape), `m3-probe-poly-validate`,
  `m3-probe-r3-hashindex`, `m3-probe-r3-ctor-share`,
  `m3-probe-ctor-merge`.
