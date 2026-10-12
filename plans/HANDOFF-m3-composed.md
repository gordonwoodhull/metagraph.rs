# HANDOFF — M3 continuation: step 9 (stdlib promotion + doc pass)

**Status when written:** steps 6–8 are COMPLETE on `worktree-m3-implement`
(worktree `.claude/worktrees/m3-implement`, branched at `9c3569c` = main).
Tip `5291012`; step 8 landed as `c9e1692` (enablers), `08e0f5e` (flat),
`81db892` (compile-fail proofs), `c9e4662` (composed), `42f4ccc`
(grammar hygiene), `5291012` (docs) — six commits this stretch. Full
workspace green: 37 suites (koenigsberg + csr + composed_csr +
`downstream-backing-indirect`'s two). Steps 3–8 ticked in the plan;
step 8's fused-root note recorded as emitted and HOLDING. Nothing
merged; the user merges.

**Scope:** step 9 per `plans/milestone-3-backings-and-stdlib.md` (the
plan is the contract; this file is the working state and the traps).

---

## What steps 6–8 left behind (the load-bearing facts)

- All five stdlib backings are live `backing!` declarations in
  `src/backings.rs`; `csr_adjacency` binds and runs on flat AND composed
  stores (drivers `tests/csr.rs`, `tests/composed_csr.rs`).
- **The type-state store (step 8):** when any bound backing `is_phased()`
  (carries `phase:`), both codegens emit `Store<P: __{Store}Phases =
  Frozen>` with `pub struct Build(())` / `pub struct Frozen(())` and a
  per-store doc-hidden phase trait (one assoc type per `on store build`
  field; `()` on Frozen — the accumulators are released at the TYPE
  level). Write side (`add_*`/insert/set ops, `__onadd_`/`__insert_`/
  `__set_`/`__compact_` hooks, the consuming `freeze()`, `finish()`)
  lives on `impl Store<Build>`; reads (counts, read view, root
  accessor(s), `validate()`) on `impl Store<Frozen>`; handles and
  iterators hold `&'g Store<Frozen>`. Manual `Default`/`Debug` per phase
  (no derives — the assoc-typed fields break derive's bounds). Unphased
  stores are byte-identical to the M1/M2 shape (single impl, derives) —
  that criterion held: all pre-existing suites passed at every commit.
- **Store-level field naming:** "the declared stem, bare when
  unambiguous" — colliding store fields rename per arrow (flat:
  `<arrow>_<name>`, via `store_overrides` threaded into `FlatRoles::new`;
  composed: `<inst>_<name>`, via `store_field_names` keyed `(i, arrow)`
  threaded into `ComposedRoles::new`, whose signature grew `i` +
  the map — 12 call sites). Two csr bindings on one store is the pinned
  legal case; compact hooks run in binding order at the single freeze().
- **Applicability:** phased + `hosts_arena(self, B)` shape = rejection
  (flat: unfused-root self, the only flat arm; composed: unfused-root
  self OR embedded target in the self class — both arms quoted with the
  self case + placement facts). The composed fused-root self
  (root-level fused class, root-level B) is LEGAL and emitted — the
  frontier note's inference (`a` = fused class id, `on_add` from the
  fused ctor, buckets store-level) HOLDS generically; the note is
  retired, not recorded broken.
- **Validate-variant selection:** a `Plain` body satisfies ANY
  membership kind, in BOTH the applicability check and the emission
  selection (csr's offsets check is membership-agnostic). The emission
  half was a real bug: with strict selection the mixed store validated
  Ok over a corrupted offsets array.
- Grammar: labeled `on store` and `compact` require a phased backing
  (declaration-site rejection, round-trip tests in `backing.rs`).
- The authored csr validate's loop variable is `win`, not `w` — `w` is
  a reserved param spelling; a `for w in <unrecognized source>` loses
  its binder to the mixed-site re-emit (E0425, trap 2's corollary).

## Step 9 — stdlib promotion + doc pass

- Promote M2's discovered roots into the stdlib (names as M2 resolved
  them; criterion = M2 suites unchanged modulo import paths): `EdgesOf`,
  the `main_*` one-arrows, the keyed pair (`SubNodeOf`/`SubEdgeOf`).
  They live in `tests/fixtures/src/lib.rs` (`roots` module) today;
  step 9 moves/copies them into `src/stdlib.rs` and re-points
  `m2-fixtures` at the stdlib (or thins the fixtures — whichever the
  criterion prefers). Watch: the fixtures' `Subgraph`/`SubPlain`/
  `Chain`/`KeyedPlain` patterns instantiate them by
  `crate::roots::MainNode` etc. — those paths change to
  `metagraph_exper::stdlib::…`, and the resolver's direct-dependency
  rule still applies. `neg::AnchoredKey` uses `crate::roots::SubNodeOf`
  too.
- The doc pass to M1 step-11's standard: generated-code doc comments
  carry the arrow that produced each item — the composed port already
  emits per-contribution docs naming `inst.arrow`; AUDIT FLAT too
  (flat's store-contribution/field docs name `arrow` + backing; the
  read-view/iter/trait docs name the arrow — check they all do).
- The doc-table prose in `macros/src/lib.rs` (the four built-in names in
  `implement!`'s docs) is step 9's reword — there are now FIVE stdlib
  backings and the type-state store to mention; deliberately not
  committed earlier.

## Verification commands

- `cargo test --workspace` — must be fully green at every commit (37
  suites at the step-8 tip; `validate_failures` runs 11,
  `csr` 12, `composed_csr` 3).
- `cargo test -p metagraph-exper --test koenigsberg` — byte-sensitive
  traversal-order assertions.
- `cargo test -p metagraph-exper --test compilefail` — pinned stderr;
  regenerate with `TRYBUILD=overwrite`.
- `cargo build -p downstream-backing-indirect --features indirect` (and
  the downstream-indirect equivalent) — the resolver's negative cases,
  feature-gated off the default build.
- `MG_DEBUG=1 cargo test -p metagraph-exper --test <t>` — dump
  `implement!` / `__implement_gen` emissions when debugging (note: it
  prints to the test binary's stderr; on compile errors use
  `cd tests/downstream/<crate> && cargo build` to see expansions).

## Known traps (three more earned their place this stretch)

1. **Quote-iterator exhaustion:** `#(#x)*` repetition consumes iterator
   `x`; interpolate a `Vec` when the same list appears twice in one
   `quote!` (assoc names/tys in the phase trait).
2. **Definition order in the macro code mirrors emission order:**
   closures/`quote!`s capture at definition — `phase_trait` must precede
   `store_contrib`, the `Default` field-init iterators must precede the
   `default_init` quote. E0425 in the macro CRATE means a codegen
   ordering bug, not an emission bug.
3. **The struct needs the bound, edition 2024 needs it public:** a field
   typed `<P as Trait>::Assoc` requires `struct S<P: Trait = D>`; the
   bound makes a private trait a `private_bounds` denial — emit
   `#[doc(hidden)] pub trait`.
4. (carried) Courier hops are brace-invoked; `Span::mixed_site()` pairs
   on both sides; `handle(x)` mints bare; two `implement!`s per module
   collide; `link_stem` strips one trailing `_edges`; existence groups
   on the target CLASS; `rw_for`'s source scan counts call parens;
   `pat_idents` flattens one level; the MG_DEBUG dump stringifies spans
   away (hygiene bugs reproduce clean in /tmp probes).
5. **Python patch scripts over codegen:** the worktree sandbox refuses
   heredocs and complex sed — Write a script file, run it plainly;
   assert exact occurrence counts (an atomic write means a failed
   assertion rolls everything back, but a PARTIAL match set can pass
   asserts yet miss one intendent variant — grep afterwards).
