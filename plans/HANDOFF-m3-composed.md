# HANDOFF — M3 composed codegen (steps 3–5 finish) + steps 6–9

**Status when written:** steps 2 and the flat half of 3–5 are COMMITTED
on `worktree-m3-implement` (worktree `.claude/worktrees/m3-implement`):
`f01d88e` resolution machinery + stdlib `src/backings.rs`,
`710bca2` roles.rs + data-driven flat codegen, `4ebf038` the bounce
gate. Full workspace green (31 suites incl. trybuild and the two new
downstream crates). Plan boxes ticked to the honest state: step 2 all
three; step 3 field + both acceptance boxes; step 4 contributions +
validate-selection + acceptance.

**Scope of this handoff:** commit 3 of the fused sequence (composed
codegen data-driven), commit 4 (delete the remaining fixed table), then
steps 6–9 per the plan.

## What exists now

- `macros/src/roles.rs` — the role-substitution engine: a `RoleBinding`
  trait (`rec/records/maps/host_of/idx/arena_of/map_from/determining/
  handle`, id types, param substitutions, Walk rename, field renames)
  plus a textual outermost-first rewriter. `FlatRoles` implements it.
  Hygiene rules that are LOAD-BEARING (proven this session):
  - scaffold idents (`errs`/`arrow_label`/`det_label`), kept hook params
    (`a`/`b`/`k`), and Walk struct fields/keys/after-dot mentions are
    re-emitted with `Span::mixed_site()` on BOTH sides (emitted binding
    and body mention) — plain call-site spans on either side break
    resolution (E0425, seen and fixed);
  - loop-bound locals (`for ra in records(A)`, `(k, v)` destructuring)
    keep the body's own courier spans so pattern and uses pair;
  - field renames apply only after-dot; authored locals survive
    (csr's `let mut offsets` beside `store.offsets`).
- `macros/src/codegen.rs` — flat codegen fully declaration-driven:
  contributions (formula renames, uniform init rule), ctor args,
  on_insert hooks (authored params, root-self folds `a`), iter/iterate/
  advance (Walk rename + reserved `store` field; `w`→`self`),
  read/lookup trait impls + builder read view, `__validate_<arrow>`
  methods in binding order with the uniform `Option<id>` range pre-pass
  (early return ⇒ authored walks may `rec(x)` freely), validate-variant
  selection by membership facts, `set_`/`insert_` ops derived from
  clauses. Phased backings rejected cleanly until step 8.
- `advance_chain` synthesizes one extra backing hop per **defaulted one
  arrow** (key = binding key, courier = runtime `field`) before codegen.
- Stdlib declarations in `src/backings.rs` carry the real bodies;
  intrusive/csr `advance` tails are `Some(handle(id))` (r4-advance
  probe's pinned shape — `handle()` mints bare, bodies author `Some`).
- Bounce gate: `tests/downstream/backing-provider` (user backing whose
  `on_insert` names a use-site-only helper) + `tests/downstream/
  backing-user` (renamed dep, path-form + bare-ident in one
  `implement!`, counter proves the helper ran, orders + validate
  asserted).

## Commit 3 — composed codegen data-driven

Port `codegen_composed.rs` onto `RoleBinding` (a `ComposedRoles`),
mirroring what flat did, under M2's placement machinery (classes,
`ClassLoc`, partner map, collision prefixing). The M2 suites
(`hierarchy_subgraphs`, `edge_induced`, `edge_chain`, `compose_chain`,
`validate_failures`, `composed-renamed`) are the gate; trybuild
fixtures pin several rejection messages — keep texts byte-stable.

Design decisions already made (do not relitigate):

- One `ComposedRoles` per (instance, arrow) built from the placement
  tables: `rec` through `rec_ref_ts` shapes; `records(E)`/`maps()` via
  `enum_recs`-style nested enumeration with **owner = the vacuous host
  `()` wherever ids are flat** (role table: owner shape must match
  `host_of`, else the authored `host_of(v) != owner` cannot typecheck);
  `host_of` = `()` flat / `v.0` path; `idx` = `.0` / `.1` leaf.
- Labels: `arrow_label` = `` "arrow `inst.arrow`" ``; hook/validate
  names instance-qualified as today (`__insert_<inst>_<arrow>`,
  `__validate_<inst>_<arrow>`); validate summoned in binding order.
- Field-name collisions keep today's resolution (instance/arrow-stem
  prefixes at collection; `field_of_arrow` map); formula renames apply
  per binding exactly as flat.
- Known gap to carry (mirrors today's omission, do not silently
  "fix"): store-level self with an **embedded** target class — today's
  codegen emits no host-agreement arm there; the uniform authored body
  cannot typecheck (`host_of(v): HostId` vs `owner: ()`). Either reject
  at implement time quoting the placement facts, or special-case the
  emission — record the choice in the plan.
- Intrusive-at-unfused-root relaxation applies composed too (the merge
  chore): store-level head, self-less hook, root-handle read — drop the
  `!classes[sc].owns_records` rejection for `intrusive_list` (keep the
  `field`/ctor_arg one: `impl_unfused_root_field` fixture) and keep
  `impl_intrusive_at_embedded` (embedded-sourced rejection stays).
- The by-derivation ctor hook-call and the combined ctor rule stay
  codegen's (orchestration); `determining(x)` binds to the det field
  through `field_of_arrow`.
- Advance at embedded targets: reserved `host` field on the Walk (only
  the index-mint recipe reads it; the link walk takes the host from the
  path id — r4-advance probe). `handle()` raw-vs-id as flat.

## Commit 4 — delete the fixed table

After commit 3 the composed `["arena","field","intrusive_list",
"hash_index"]` membership checks and name-keyed arms are dead; delete
them. Then tick step 3's table box, step 5's boxes (the three self
placements: plain + host-embedded + store-level, keyed tests green via
declarations, should_panic dup-key), and the overall gate's inspection.

## Then steps 6–9 (plan is the contract)

- Step 6: corruption fixtures per spike 4 against the new
  `__validate_*` surface (ghost keys, out-of-range/cross-host values,
  shared-vs-cycle distinguished).
- Step 7: predicate evaluation + error templates (cardinality,
  existence, hosts_arena, direct_records), ctor_arg/on_add orchestrator
  rejection, trybuild fixtures for step-1 grammar rejections too.
- Step 8: `csr_adjacency` — the declaration already rides in
  `src/backings.rs` (bodies may be refined when emission lands);
  type-state store, `on store build/frozen` partition, single
  `freeze()`, compaction in binding order.
- Step 9: promote M2's discovered roots into the stdlib; doc pass.

## Open fixture debts (small, fold into step 7)

- No fixture pins backing re-export routing rejection (the mechanism
  fires in `GenInput::BackingStep` via `declaring_crate_root`).
- Reserved-set shadowing (hygiene caveat 3's grammar rule) is not yet
  checked — engine rewrites reserved heads unconditionally.
- `hosts_arena`/`direct_records`/`existence` are parsed but not
  evaluated (step 7's table).

## Verification

- `cargo test --workspace` green at every commit.
- `MG_DEBUG=1 cargo test -p metagraph-exper --test koenigsberg` dumps
  `implement!`/`__implement_gen` emissions when debugging hops.
