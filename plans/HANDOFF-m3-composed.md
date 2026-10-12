# HANDOFF — M3 continuation: composed codegen, then steps 6–9

**Status when written:** steps 2 and the flat half of 3–5 are COMMITTED
on `worktree-m3-implement` (worktree `.claude/worktrees/m3-implement`,
branched at `9c3569c` = main). Tip `fd131ae`; commits this stretch:
`f01d88e` (binding path form + backing-courier resolution + stdlib
`src/backings.rs`), `710bca2` (`roles.rs` + flat codegen data-driven),
`4ebf038` (the bounce gate — step 2's acceptance), `fd131ae` (plan
ticks + this handoff's earlier draft). Full workspace green: 31 suites
incl. trybuild and the two new downstream crates. Nothing merged; the
user merges.

**Scope:** the two remaining commits of the fused steps-2–5 sequence
(composed codegen data-driven; delete the last fixed table — which
closes steps 3–5), then steps 6–9 per the plan. Read the "Reality
check" of `HANDOFF-m3-step2.md` (still in this directory) for why the
sequence is shaped this way; this file supersedes it as the working
contract.

---

## What this stretch left behind

- `macros/src/roles.rs` — the role-substitution engine. A `RoleBinding`
  trait (one method per role: `rec`, `records`, `maps`, `host_of`,
  `idx`, `arena_of`, `map_from`, `determining`, `handle`, `id_ty`,
  `param`, `store`, `walk_ty`, `rename_field`, `is_walk_field`,
  `walk_field_is_raw`) and a textual, outermost-first rewriter
  (`roles::rewrite`) with: syntactic entity recovery (literal `a`/`b`/
  `k`; `for v in records(E)` binds v→E; `for (owner, m) in maps(A)`
  binds the map; `for (k, v) in m.iter()` binds K/B; ambient default B),
  the `Walk { .. }` literal gaining the reserved `store` field (+
  re-keyed fields), after-dot-only field renames, and the hygiene rules
  under "Known traps" below. `FlatRoles` in `codegen.rs` is the flat
  implementation — read it as the template for `ComposedRoles`.
- `macros/src/codegen.rs` — flat codegen fully declaration-driven:
  contributions with formula renames and the uniform init rule
  (ctor_arg / `Option`→None / `Vec`+`HashMap`→default / else reject);
  ctor args from `ctor_arg` clauses; `on_insert` hooks with authored
  param spellings; `iter`/`iterate`/`advance` (Walk renamed to
  `iter_ty(arrow)`, `w`→`self`, `Some(handle(id))` tails); `read`/
  `lookup` trait impls + builder read view; `__validate_<arrow>`
  methods summoned in binding order with the uniform `Option<id>` range
  pre-pass and **early return** (so authored walks may `rec(x)` freely);
  validate-variant selection by the arrow's membership facts; `set_`/
  `insert_` ops derived from clauses; clean rejection of phased
  backings until step 8. The flat fixed table is already gone.
- `macros/src/lib.rs` — `ChainConfig` carries `backing_queue` +
  `backing_decls`; `advance_chain` drains backing hops first, then
  roots, then **synthesizes one hop per defaulted `one` arrow** (key =
  binding key, courier = runtime `field`) before codegen —
  `unbound_one_arrow_keys` is the helper; keep it in sync with the
  codegens' meta lookups.
- `src/backings.rs` — the five stdlib declarations with real bodies.
  These are contract now: the composed port must make them emit at
  every composed placement without editing them (if one genuinely
  cannot, that's a finding to fold into the plan, not a body edit —
  the one sanctioned exception so far was `Some(handle(id))`, already
  applied).
- Bounce gate: `tests/downstream/backing-provider` (user backing whose
  `on_insert` names `note_push`, which exists only at the use site) +
  `tests/downstream/backing-user` (depends on the provider renamed;
  one `implement!` binding the user backing by path and `arena` by
  bare ident; the helper's counter, traversal orders, and `validate()`
  are the assertions). These two crates are the permanent step-2
  evidence — keep them green.

## Commit 3 — composed codegen data-driven (closes nothing by itself,
## keeps the tree green)

Port `codegen_composed.rs` onto `RoleBinding` with a `ComposedRoles`,
mirroring what flat did, under M2's placement machinery (union-find
classes, `ClassLoc::Root/Embedded(host)`, the `<->` partner map,
collision prefixing). The M2 suites are the gate: `hierarchy_subgraphs`
(incl. the `should_panic` duplicate-key test), `edge_induced`,
`edge_chain`, `compose_chain`, `validate_failures`, downstream
`composed-renamed`. Every trybuild fixture that names a composed
rejection pins its message byte-for-byte — keep texts stable (the
cardinality message survives unchanged because decl name == binding
spelling for the four stdlib names).

Existing helpers to reuse (all in `codegen_composed.rs` today):
`rec_ref_ts` (record access by id value), `rec_ref_handle` (through a
handle binding), `enum_recs` (nested enumeration binding `self_id` +
`srec`), `out_of_range` (short-circuit, path-aware bounds test),
`rec_get` (safe `.get` walk access), `sum_sibling_expr`,
`ClassData`/`FieldContribution`, `field_of_arrow`, `hosted`,
`root_map_of`, `class_index_of`/`ent_index`, `arrow_name_users`,
`iter_name`, `check_name`. The port replaces the string-keyed match
arms with decl-driven emission; the collection/collision/naming logic
stays.

Role bindings per placement (decisions — do not relitigate):

- **Entity → class**: A/B/K map to classes via `class_index_of(i, e)`
  (the partner map resolves open entities). `id_ty` is the class's
  `id_name` — and `()` when the self is an unfused root (A slot).
- **`rec(x)`** = `rec_ref_ts(class_of_ent(x), x)`; at an unfused-root
  self, `rec(a)` folds to the store (flat already does this).
- **`records(E)`**: ids-only enumeration — `enum_recs` shape minus the
  `srec` binding, mapping to path ids (`#idt(#hid(hi), ri)`) when
  embedded; `once(())` for an unfused-root self.
- **`maps(A)`**: owner-yielding pairs whose shape must match
  `host_of`: per-record map at a plain self → **owner = the vacuous
  host `()`** (flat ids carry no host component; the authored
  `host_of(v) != owner` must typecheck — this is the role table's
  parenthetical, and it means the flat-style "on {:?}" self-id spelling
  is gone at flat/plain placements; only composed tests pin messages,
  all via `contains`, so this is safe); map on a host-embedded self →
  one pair per host record, **owner = the host id**; store-level →
  `once(((), &store.<map>))`.
- **`host_of(v)`** by v's class loc: `()` at Root, `(v).0` at
  Embedded. **`idx(v)`**: `.0 as usize` at Root, `.1 as usize` leaf at
  Embedded.
- **`arena_of(X, arg)`**: X's class Root → `store.<x_arena>`;
  Embedded(hc) → the arg-indexed record of the HOST class, then
  `.<x_arena>` (the arg names a host-class record — that is the only
  shape the declarations use: `b_arena_of(a)` with A the host, and
  `*_arena_of(host_of(v))`).
- **`map_from(a)`**: per-record map via `rec(a)`; store-level fold at
  an unfused-root self (`root_map_of`).
- **`determining(x)`**: `rec(x).<det field>` with the det field from
  `field_of_arrow` (the combined ctor rule already ran at collection).
- **`handle(x, raw)`**: target Root → `{store, id: x}` / raw
  `#Id(x)`; target Embedded → `{store, host: x.0, id: x.1}` / raw
  `#Id(w.host, x)` — the Walk carries the reserved `host` field at
  embedded placements, and only the index-mint recipe reads it
  (r4-advance probe).
- **Labels and names**: `arrow_label` = `` "arrow `inst.arrow`" ``;
  hooks/validators instance-qualified (`__insert_<inst>_<arrow>`,
  `__validate_<inst>_<arrow>`), summoned in binding order; public ops
  take the instance prefix only on cross-instance arrow-name collision
  (today's rule).
- **Intrusive-at-unfused-root relaxation (the merge chore)**: drop the
  `!classes[sc].owns_records` rejection for `intrusive_list` —
  store-level head, self-less hook, root-handle read. KEEP the
  `field`/ctor_arg rejection (`impl_unfused_root_field` fixture) and
  the embedded-sourced intrusive rejection
  (`impl_intrusive_at_embedded` fixture).
- **Known gap to carry, not silently fix**: store-level self with an
  *embedded* target class — today's codegen emits no host-agreement
  arm there, and the uniform authored body cannot typecheck
  (`host_of(v): HostId` vs `owner: ()`). Either reject at implement
  time quoting the placement facts, or special-case the emission —
  decide once, fold the choice into the plan's Constraints section,
  and say so in the commit message.

## Commit 4 — delete the fixed table (closes steps 3–5)

After commit 3 the composed `["arena","field","intrusive_list",
"hash_index"]` membership checks and name-keyed arms are dead; delete
them (grep the macros crate for the four names — `src/lib.rs`'s doc
table is prose, reword it in step 9's doc pass, not here). Then tick:
step 3's table box, step 5's boxes (the three self placements — plain,
host-embedded, store-level — emitted from declarations; M2 keyed tests
green in both M2 placements plus the plain-self placement, which needs
a new flat keyed test since M2's keyed tests are all composed; the
`should_panic` duplicate-key test stays green), and the overall gate's
"fixed table gone / no hidden fixed table" inspection item. Step 5's
plain-self test: a small flat keyed pattern (m2-fixtures `Subgraph`
shape implemented flat, per-self map) added to the runtime's test
suite — the placement exists in code but no test drives it.

## Then steps 6–9 (the plan is the contract; highlights only)

- **Step 6** — corruption fixtures per spike 4 against the new
  `__validate_*` surface: ghost keys, out-of-range and cross-host
  values, chain corruption in both partition shapes (shared membership
  vs cycle, distinguished in the report). The early-return pre-pass
  changes joint-report behavior for corrupt links — fixtures should
  pin the new behavior deliberately.
- **Step 7** — predicate evaluation with the pinned error templates
  (cardinality, `existence`, `hosts_arena`, `direct_records`), the
  `ctor_arg`/`on_add` orchestrator-rule rejection, trybuild fixtures
  for each predicate and for step 1's grammar rejections. Fold the
  open fixture debts (below) here.
- **Step 8** — `csr_adjacency` + type-state: the declaration already
  rides in `src/backings.rs` (bodies may be refined when emission
  lands — say so in the commit); `on store build/frozen` partition,
  the store parameter emitted only when a phased backing is bound,
  `compact` at the single `freeze()` in binding order, the three
  compile-fail proofs, applicability rejections.
- **Step 9** — promote M2's discovered roots into the stdlib (names as
  M2 resolved them; criterion = M2 suites unchanged modulo import
  paths), then the doc pass to M1 step-11's standard (generated-code
  doc comments carry the arrow that produced each item).

## Open fixture debts (small; fold into step 7)

- No fixture pins backing **re-export routing** rejection — the
  mechanism fires in `GenInput::BackingStep` via `declaring_crate_root`
  (a middle crate re-exporting the declaration is the dep, the
  declarer is not → the resolver's clean error). Mirror the pattern
  side's `reexporter` downstream crate.
- **Reserved-set shadowing** (hygiene caveat 3's grammar rule) is not
  checked — the engine rewrites reserved heads unconditionally. A body
  defining a local named `rec`/`handle`/… is silently captured.
- `hosts_arena`/`direct_records`/`existence` parse but are not
  evaluated anywhere yet.

## Verification commands

- `cargo test --workspace` — must be fully green at every commit
  (31 suites at tip `fd131ae`; grows with the step-5 plain-self test).
- `cargo test -p metagraph-exper --test koenigsberg` — byte-sensitive
  traversal-order assertions.
- `cargo test -p metagraph-exper --test validate_failures` — the
  pinned failure-message tails.
- `MG_DEBUG=1 cargo test -p metagraph-exper --test koenigsberg` — dump
  `implement!` / `__implement_gen` emissions when debugging courier
  hops.

## Known traps (three cost a debug cycle this stretch; all fixed,
all repeatable)

- **Courier hops are brace-invoked with the paren group nested**:
  `courier! { ( tgt ; rest ) }`. Emitting `courier!(tgt ; rest)` makes
  the parens the invocation delimiter, the matcher then sees `tgt`'s
  first token where it expects `(` — "no rules expected", plus a
  misleading "macros that expand to items" error. Every hop emitter in
  `lib.rs` is the reference shape.
- **`Span::mixed_site()` must pair on BOTH sides**: the emitted
  binding (hook params `a`/`b`/`k`, Walk struct fields, scaffold
  `let`s) AND the engine's re-emitted body mentions. One-sided
  mixed_site (or plain call-site on either) breaks resolution with
  E0425 against a *correct-looking* expansion. Meanwhile loop-bound
  locals (`for ra in …`, tuple destructuring) must keep the body's own
  courier spans so pattern and uses pair — `roles.rs` checks the scope
  before re-spanning.
- **`handle(x)` mints bare; bodies author `Some(handle(id))`** where
  the tail must be `Option` (intrusive walk). The `.then(|| …)` recipes
  stay unwrapped. Pinned by the r4-advance probe; the stdlib
  declarations already spell it.
- Flat `maps(A)` owner is `()` at plain-self too — the authored
  value-in-arena message gains `on ()` there. Only composed spellings
  are test-pinned (`contains` assertions); do not "restore" the self
  id without making `host_of(v) != owner` typecheck.
- A `backing!` invocation cannot carry doc comments before the
  declaration name (the parser expects the ident) — use `//` comments.
  Same E0255-avoidance as patterns: declare in a named module.
