# HANDOFF — M3 continuation: steps 8–9

**Status when written:** steps 6–7 are COMPLETE on `worktree-m3-implement`
(worktree `.claude/worktrees/m3-implement`, branched at `9c3569c` = main).
Tip `fd2e445`; step 6 landed as `6c2afa5`, step 7 as `fd2e445` (see the
commit messages). Full workspace green: 35 suites incl. trybuild (33 +
`downstream-backing-indirect`'s two). Steps 3–7 ticked in the plan;
step 4's hygiene box ticked with step 7's shadowing rule. Nothing
merged; the user merges.

**Scope:** steps 8–9 per `plans/milestone-3-backings-and-stdlib.md` (the
plan is the contract; this file is the working state and the traps).

---

## What steps 6–7 left behind

- Step 6 (`6c2afa5`): `mod corrupt` in `tests/validate_failures.rs` — the
  uniform range pre-pass's only driver (one `implement!` per submodule;
  the intrusive-into-embedded driver is `corrupt::hierarchy` with
  `sub.subedges: intrusive_list`; the field is `next_subedges`, not
  `next_sub` — `link_stem` strips one trailing `_edges` and `subedges`
  has no underscore). `src/lib.rs` crate docs now state the canonical
  inventory + pre-pass semantics. See that commit's message.
- Step 7 (`fd2e445`):
  - Predicate evaluation, both codegens: `existence` (duplicate per
    target class names both arrows; `by`-derived contradicts existence),
    `hosts_arena` (boolean over placement: B embedded in the self class's
    record, or unfused-root self + root-level B; composed gained the two
    arms the old check missed — **unfused root at an embedded B, and a
    record-owning self at a root-level B** — flat gained the whole
    check), `direct_records` upgraded to the template (arrow, backing
    tokens, both endpoints' placements). Flat `direct_records` is a
    documented always-true (no embedded placements exist flat).
  - `roles::check_reserved_bindings` + `RESERVED_BINDINGS` (role heads,
    `store`, entity params `a`/`b`/`k`/`w`, scaffolds `errs`/
    `arrow_label`/`det_label`) — shallow let/fn/struct token scan,
    called per clause body in `backing.rs`'s `validate_inventory`.
    **False-positive watch:** any future stdlib body binding one of
    those as a local breaks the build — that is the check working.
  - Fixtures: `impl_existence_duplicate` + `impl_arena_at_element`
    (new neg pattern `TwoInto`: two primitive `many` arrows into N),
    `impl_unfused_root_field_flat`, `backing_shadow_reserved`,
    `backing_{unknown_clause,bad_store_label,closed_set,repeated_clause}`
    (backing! grammar rejections at declaration-site spans),
    `downstream-backing-indirect` (feature-gated re-export negative).
- **Backing-indirect span quirk:** the resolver error for a backing
  through a re-export lands on the *pattern!* block, not the
  `implement!` call (the pattern-side equivalent lands on `implement!`).
  Message is clean and names the fix; span degradation as usual. Not
  investigated further — noted, not adjudicated.

## Step 8 — `csr_adjacency` + type-state

- The declaration rides in `src/backings.rs` (bodies may be refined when
  emission lands — say so in the commit). `arena_len(A)` appears in its
  validate body and is NOT yet a role the engine rewrites — either grow
  the engine (a `records`- sibling role) or refine the body to the
  `b_arena_of`/`records` vocabulary; decide once and keep the closed
  vocabulary honest. Leaning: refine the body to `records(A).count()`
  (validate-only, O(n) is fine) unless the spike's CSR tests pin
  `arena_len` in messages — check spike 6's suite shapes before
  deciding.
- `on store build`/`on store frozen` partition; the store type parameter
  emitted only when a phased backing is bound; the store parameter only
  in phased signatures; `compact` at the single `freeze()` in binding
  order; the three compile-fail proofs (insert-after-freeze,
  frozen-as-builder, no accumulators on the frozen type);
  applicability rejections (entity arrow, unfused-root self — the
  unfused-root arm generalizes the existing orchestrated check since
  `on_add` is constructor-orchestrated); the fused-root emission
  attempted against the frontier note — emit it or record exactly what
  breaks.
- The pre-pass applicability checks already reject phased declarations
  cleanly (flat and composed); **removing that rejection is step 8's
  first act**.
- The plan's CSR section (around line 685): type-state on ONE store
  type parameterized by a phase marker, emitted only when a phased
  backing is bound; `freeze()` consumes the build store and compacts;
  `finish()` = freeze then validate; handles exist only for the frozen
  phase; the mixed-phase probe (CSR + intrusive_list coexisting on one
  `GraphStore<P>`) is the shape to port as the runtime driver; M4's
  flush ordering note (freeze = flush step 6, binding order for
  multiple phased backings) is pinned there.
- Spike 6's CSR tests + the mixed-phase probe's shape are named in the
  plan's step-8 acceptance; port them as `tests/csr.rs` (or extend an
  existing suite) — spike reports/probes may still be under /tmp from
  the spike era if the shapes need re-derivation.

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

- `cargo test --workspace` — must be fully green at every commit (35
  suites at the step-7 tip; `validate_failures` runs 11 tests).
- `cargo test -p metagraph-exper --test koenigsberg` — byte-sensitive
  traversal-order assertions.
- `cargo test -p metagraph-exper --test validate_failures` — the pinned
  failure-message tails.
- `cargo test -p metagraph-exper --test compilefail` — pinned stderr;
  regenerate with `TRYBUILD=overwrite`.
- `cargo build -p downstream-backing-indirect --features indirect` (and
  the downstream-indirect equivalent) — the resolver's negative cases,
  feature-gated off the default build.
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
9. **Two `implement!`s in one module collide on class names** (step 6):
   each implement emits its classes' items (ids, Data, handle types,
   iterators) into the implementing module, un-prefixed by store name.
   One `implement!` per module; the corruption tests sit in the same
   submodule as their declaration (record fields are private to it).
10. **Guess field names from the naming formula, not the arrow's
    spelling** (step 6): `link_stem` strips one trailing `_edges` —
    `elems → next_elems`, `out_edges → next_out`, but `subedges →
    next_subedges` (no underscore; the stem is the whole name). Verify
    with MG_DEBUG before writing damage against generated fields.
11. **Existence grouping keys on the target CLASS, not the entity name**
    (step 7): two instances' arrows can fuse into one class (that is the
    whole point of fusion) — grouping by `decl.target` string would miss
    duplicates across instances. Composed groups by `class_index_of`;
    flat (no classes) groups by target name.
