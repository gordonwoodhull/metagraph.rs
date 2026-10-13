# HANDOFF — M4 implementation (write-side builders)

Branch `worktree-m4-implement`, worktree `.claude/worktrees/m4-implement`,
based on main 4904680 + steps 1–10. Plan: plans/milestone-4-builders.md
(the contract — checklist § "Implementation checklist", dependency line
1 → 2 → 3 → (4, 5); 6 → (7, 8); 3 → (9, 12); 9 → 11; (8, 11) → 10).
This file: state, settled decisions, and the per-step continuation guide.
Retires to .handoffs/ with the worktree.

## Where things stand

Done and committed:

- **Step 1** (14a19f1): runtime `src/tx.rs` (TxToken/atomic mint,
  `RefKind<I>`/`Ref<I>` with eager `check_stamp`, marker-generic
  `HandleToRef<M>`); `macros/src/tx.rs` — the ONE Tx emitter, driven by
  a normalized `TxModel` (entities with ctor args + on_add hooks);
  flat codegen builds the model under a shape gate; emitted per
  implement!: ref aliases, per-entity `Into<Ent>Arg` (ref: stamp-checked;
  raw id: unstamped Existing; handle: hard store-identity assert), `Op`
  enum (Add/Remove per entity), `add_*`/`remove_*` staging
  (Pending-removal panics), `edit()`, `old()`.
  Tests: tests/tx_stage1.rs (3), compilefail/read_on_ref.rs (E0599).
- **Step 2** (5bdebc1): tests/tx_staging_panics.rs — 4 should_panic
  tests (foreign stamp, stale stamp, Pending removal, foreign handle).
  Suite green at every commit.

Step 2 notes: the plan's four-panic surface is split by verb
availability — `set_` of a Pending ref lands with step 7's set_ staging,
keyed-insert-on-live-key with keyed staging (steps 8/11); a panic can't
be exercised before its staging call exists. Recorded here per the plan's
"note it in the step notes".

- **Step 3** (this commit): `flush(self) -> Result<FlushOutcome,
  FlushError>` emitted by macros/src/tx.rs from the grown TxModel —
  per-entity record fields + store fields + intrusive-carry arrows.
  Pipeline: step 1 removals + range-checked Existing (UnknownOldId /
  DoubleRemoval), step 2 danglers (kept records per ctor-arg field,
  attributed to the removal op; staged adds naming removed endpoints,
  attributed to the naming op; range-guarded — the oob-repro bug),
  step 3 compaction maps, step 4a skeleton (raw record literals,
  ctor-arg ids remapped, Option None, containers Default, on_add fired
  per kept record), 4b intrusive rebuild-by-walk (head at record or
  store), 4c submission-order replay through the real ctors with
  per-entity pending tables, step 7 validate (ValidateFailed).
  Carry classification in codegen.rs: `one`+field shape, `many`+
  existence, `many`+intrusive head/link shape (exactly two stem
  `Option<BId>` fields + on_insert — counting_list qualifies);
  anything else gates the Tx off (no edit()) beside phased/keyed.
  Violation/FlushError/RecordDesc/FlushReport{per-arena _map}/
  FlushOutcome emitted per store.

- **Step 4** (this commit): flush steps 1–2 factored into a generated
  doc-hidden `Tx::__analyze(old, ops) -> Result<__TxAnalysis,
  FlushError>` (per-arena counts + removed/removed_by sets); `pub fn
  check(&self)` predicates on it, flush destructures its result and
  runs steps 3–7 on the same bindings. Tests in tests/tx_flush.rs:
  check-green→flush-commits (via edit — the into_edit half of the
  committed-mode contract lands with step 5), check's violations ==
  flush's on the same change set, byte-identical Display across six
  independent builds. Note: the flat non-phased pipeline has no
  HashMap-sourced order yet; the sort half of the determinism rule
  ("collect, sort") starts mattering with step 11's keyed carries —
  the emission point is where the keyed loops land.

- **Step 5** (this commit): `into_edit(self) -> Tx<'static>` — the
  old store rides as `OldStore<'a>` (Borrowed(&) | Owned(Box));
  `old()` now borrows `&self` (the owned arm has no 'a to return).
  The flush's carries were re-derived as ARRAYS before any write
  (`__<arrow>_heads/_head/_links` from walking the old chains), and
  step 4a became two write paths over the same derived data: fresh
  arenas with capacity (borrowed) vs forward in-place compaction
  (owned — every write slot <= its read slot; spike 7's ordering
  constraint, REPORT finding 6). Tests: into_edit ports of spike 7's
  owned-path tests (koenigsberg vs direct, self-loop tap_stage,
  empty-set clone) plus the borrowed-vs-owned equivalence on a
  removal-heavy change set (same signature, same reports). Note:
  TxIntrusive regained its `arrow` field (the arrays are named after
  the binding).

- **Step 6** (this commit): write-trait codegen — the courier's sixth,
  braced write slot `{ Trait, method, ARef, KRef?, BRef }` on RootMeta
  (grammar.rs, tolerant parse: `-`/absent = no write metadata);
  `SchemaBody::entity_ctor_args` implements the settled by-derived-only
  arity rule (inline `one` arrows + `by`-named determinants, declaration
  order; any other opaque instantiation arrow out of the entity vetoes
  `Add<E>`); root patterns emit the arrow-named `Insert<Arrow>`/
  `Set<Arrow>` traits beside the read trait and a `pub mod <snake>`
  holding per-entity markers + `Add<E>`/`Remove<E>` traits; instantiation
  patterns grow the same per-entity items into their existing module;
  finish_compose re-mints per-entity markers per instance (same dedup).
  codegen builds the trait-impl data into TxModel; tx.rs emits
  `HandleToRef` impls per entity (per-entity markers), `Add`/`Remove`
  Tx impls (Add only where the declaring block proved arity), the
  `insert_<arrow>` staging verb for primitive (non-`by`) bound many
  arrows with an insert hook — self-ful forms also get the `Insert`
  trait impl, unfused-root forms are inherent-only (self-less) — with
  `Op::Insert<Arrow>` variants replayed through `__insert_<arrow>` at
  flush on top of the carried skeleton. Tests: tests/tx_write_traits.rs
  (3 — spike 8's copy and replace_node recast against the generated
  surface, plus trait-vs-inherent equivalence) and
  tests/tx_flush_shapes.rs gains insert-through-Tx coverage (4 new:
  inherent + trait + self-less replay, dangler + oob rejections).
  Suite green incl. trybuild + downstream.

- **Step 7** (this commit): `set_` + grown hooks. `__set_<arrow>` grew
  into the re-point orchestration in BOTH codegens (flat: codegen.rs;
  composed: instance-qualified, codegen_composed.rs): unlink each
  by-derived derivative authored `on_unlink` (old host read from the
  record's own field — the by-derivation invariant, the only spelling
  that works in the honor case), write the field, insert each derivative
  authored `on_insert`; phased derivatives skipped entirely and keyed
  derivatives skipped (decision 6 + keyed relink lands with step 11).
  TxModel gained `sets` (every `ctor_arg` arrow — the same condition as
  the re-point hook; the `Set<Arrow>` trait impl wires the courier's
  write slot like the insert verbs, no inherent-only form since a
  `ctor_arg` self at an unfused root is rejected). tx.rs emits:
  `Op::Set<Arrow> { a, b }`, the `set_<arrow>` staging verb (Pending
  self → staging panic — step 2's deferred fourth panic), step-1
  last-wins effective-value tables (`eff_<arrow>`, riding the shared
  `__TxAnalysis`), a step-2 set-op pass (`SetOfRemoved{op, arena, id,
  removed_by}` + set-names-removed danglers attributed to the set op,
  emitted BEFORE the kept pass per the probe's discovery order), the
  honor guard in the kept-dangler pass (a re-pointed field skips its
  old-target check), the skeleton honor init (effective endpoint,
  born unlinked; Pending endpoint predicted as kept-count + preceding
  target adds, asserted at replay), and replay through `__set_<arrow>`
  against the rebuilt state. Tests: tests/tx_set_unlink.rs (12 — probe
  3's mid/head/tail + interleaved adds, honor, honor-with-pending,
  same-set-dangles contrast, skeleton replay order, SetOfRemoved, double
  last-wins, target-uncurable, names-removed attribution, oob-id listed
  error, Set-trait equivalence, check↔flush parity), plus the set_-of-
  Pending panic in tests/tx_staging_panics.rs. Suite green incl.
  trybuild + downstream.

- **Step 8** (this commit): unfused-root forms + the START of composed
  Tx emission. Flat side: store-level keyed arrows (`marks: G->M,
  key(N)` bound `hash_index` — the new KeyedRoot fixture) no longer
  gate the Tx off; TxModel grew `keyed: Vec<TxKeyed>` and tx.rs emits
  the self-less `insert_<arrow>(k, b)` staging verb (live key against
  the borrowed old store OR an already-staged op panics at staging —
  the staged-key probe's semantics: Existing/Pending never collide, a
  staged removal frees no key), step-1 range arms, step-2 dangler arms
  (removed key/target, attributed to the insert op), the step-4 keyed
  map carry (state, never rebuilt: filter to entries whose key AND
  target survive, remap both), and step-4c replay resolving refs
  through maps/pending tables into the backing's check-then-write
  hook. Flat codegen gained the `__<arrow>_key_live` predicate for
  parity with the composed side. Composed side: codegen_composed.rs
  builds the TxModel under the step-8 gate (non-phased AND every
  fusion class `ClassLoc::Root` — embedded classes are step 9's);
  hooks always instance-qualified (`__insert_sub_subnode`), public
  verbs bare-arrow or `insert_<inst>_<arrow>` on cross-instance
  collision via `basis_of`; ctor-arg resolution by rewritten BId type
  string (`class_by_id_name`). The composed Tx surface is INHERENT
  this step — no Add/Remove/Insert/Set trait impls (`add_trait`/
  `remove_trait`/`verb` = None); `HandleToRef` impls DO emit under
  finish_compose's per-instance entity markers. TxCtorArg gained
  `basis` so collision-prefixed ctor args still find their
  `eff_<basis>` honor tables. Tests: tests/tx_keyed.rs (7 — carry
  filter/remap, wholly-staged entry, Existing read-through, both
  staging panics, Existing/Pending non-collision, danglers + oob),
  tests/edge_induced.rs (+6 — the probe's Tx shape: staged induction,
  wholly-staged keyed round-trip, HandleToRef, live-key panic,
  intrusive+keyed splice, set_ re-point by derived membership),
  tests/root_intrusive.rs (+1 — self-less insert + root-headed carry).
  Suite green incl. trybuild + downstream, zero warnings.

Step-8 notes (scope decisions, each reasoned):

- **Store-level (root-sourced) keyed only.** A per-record keyed map
  (KeyedPlain's `tags`) still gates its store's Tx off, and the
  survivors-only `KeyRemovedWithLiveEntries` cascade stays unimplemented
  — both are step 11's keyed-carry work (the plan's step-11 keyed
  flush policy). The step-8 carry drops an entry when either endpoint
  is removed, which is exactly the probe-pinned filter+remap.
- **Composed Tx is inherent-surface.** The write traits name pattern
  entities, and which instance's marker a composed impl should key on
  is a design question step 8 doesn't need; the acceptance probes
  (root-intrusive, edge-induced) exercise inherent verbs only.
  `HandleToRef` is the exception — it's marker-generic read-side
  bridging, emitted under the canonical member's instance marker.

- **Step 9** (this commit): nested stores. `TxEntity` gained
  `host: Option<Ident2>` (the embedding host entity, depth 1) and
  `TxFieldInit::NestedArena(entity)` (a host record's embedded arena,
  rebuilt per kept host — filter + remap through the per-host maps).
  Surface: `tx.within(host) -> SubTx` (owned scope: token + host ref
  prefix, NO borrow of the tx; nested adds thread `&mut Tx` and
  re-check the token per call), `SubTx::add_<e>(&self, tx, args)`
  returning NOTHING (a nested record's path id is a resolved id;
  nothing stages a ref to a nested record this step), and
  `tx.remove_<e>(host, idx)` (Pending host panics at staging). Op
  variants carry the path as the host ref segment (embedding depth 1
  — the fixed-shape path the plan allows). Flush: step 1 keeps one
  removal set per OLD HOST per embedded arena
  (`Vec<Vec<Option<usize>>>`) with host range-guards before any
  indexed read; step 2 gains the RemovedPathHost pass (per host
  removal, first removal wins, nested adds AND removals collected
  ascending — after the kept-dangler pass, before the staged pass) and
  per-host kept-sub danglers named `RecordDesc::KeptNested{arena,
  host, idx}`; step 3 builds per-old-host sub maps (removed host =
  all-None, the cascade applied at map time and record-copy time);
  step 4c replays GROUPED root-then-nested (pass 1 fills the root
  replay tables, pass 2 resolves host segments — Pending via a
  BOUNDS-CHECKED `get`, the internal-bug "replay table miss" panic).
  New violation variants (UnknownNestedIdx, DoubleNestedRemoval,
  RemovedPathHost) and KeptNested emit ONLY for stores with embedded
  arenas — flat/root-only surfaces are byte-stable. Tests:
  tests/tx_nested.rs (15 — the probe's 10 as generated equivalents,
  plus KeptNested/staged danglers, the second embedded entity sharing
  the scope, check↔flush parity, and the into_edit owned path).
  46/46 suites green, zero warnings.

Step-9 notes (scope decisions, each reasoned):

- **The replay-table miss test IS the same-crate test the plan
  offered.** `tests/tx_nested.rs` is the crate the `implement!`
  expands into, so `tx.token` (pub(crate)) and the runtime's pub
  `Ref` fields let it forge a stamped `Pending(999)` host ref. The
  plan's either/or ("same-crate unit test or codegen-review
  invariant") resolves to the test.
- **Nested adds return no ref.** The probe returns (); a `Pending`
  ref to a nested record would need path-shaped Pending payloads
  (host ref + op index), and no op consumes one this step — embedded
  `set_` selves (path-id host translation at staging) and refs to
  nested records are the keyed-carry step's. Embedded entities DO get
  the uniform ref alias + Into + `HandleToRef` emission (harmless,
  and the later steps will need them).
- **Embedded entities: existence + field-one carries only.** Embedded
  intrusive chains and per-host keyed maps (Hierarchy's
  `sub.subnode`) keep their stores' Tx gated OFF — both carry
  per-host state, which is step 11's keyed-carry work; ctor args
  targeting an embedded entity likewise gate off (a path-id remap has
  no meaning yet), as does an embedded class with on_add hooks (the
  skeleton builds embedded records inside the host's field init,
  where per-record store hooks cannot fire).
- **The probe's builder-removal half has no equivalent.** Probe test
  10's live `remove_outer`/`remove_sub` are builder ops; generated
  stores stage removals through the Tx only. The path-id half is
  kept; flush-side compaction is covered by the other tests.
- **Multi-host naming is written but unexercised.** One host entity →
  `SubTx`/`within`; several would emit `<Host>SubTx`/`within_<host>`.
  The fixture has exactly one host (OuterNode); the general rule is
  codegen-review surface, not tested.

Not started: step 14.

- **Step 13** (this commit): negative suite. Three new trybuild fixtures:
  `write_wrong_marker.rs` (a generic transform bounded on a write trait
  at the WRONG entity marker — `AddNode<markers::Edge>` /
  `HandleToRef<markers::OutEdges>` — refuses at instantiation, E0277;
  the help text even names the correct marker),
  `handle_survives_into_edit.rs` (a handle outlives the `into_edit`
  move, E0505 — the pass discipline's compile-time half), and
  `phased_no_into_edit.rs` (a phased store's frozen face has no
  `into_edit`, E0599 — pins the step-12 borrowed-only scope decision).
  The staging should_panics are collected: tx_staging_panics.rs now
  holds every panic exerciseable over the flat surfaces (the 5 core
  ref-discipline panics + the 2 flat keyed panics moved from
  tx_keyed.rs; the keyed store lives in a `mod keyed` submodule because
  each implement! emits the Tx types unqualified — one Tx-emitting
  store per module), with the file doc listing the fixture-bound
  remainder (hosted keyed carry, nested, induction, phased re-runs,
  builder-side duplicates) and why each stays. Downstream portability:
  tests/downstream/basic gained a write-side module — a concrete
  edit/flush smoke and `splice_node`, a generic transform bounded on
  `Add`/`Remove`/`HandleToRef` at the per-entity markers, instantiated
  at the downstream crate's own `Tx` (the handle type arrives through
  `HandleToRef::Handle` — `Tx` is not a read surface, so the `Nodes`
  bound rides the SOURCE side in transforms, not `Dst`). 48/48 suites
  green incl. trybuild + downstream, zero warnings.

Step-13 notes (decisions, each reasoned):

- **One Tx-emitting `implement!` per module** is a hard rule, now
  documented by the collection: the emitted `Op`/`Tx`/`Resolve`/
  `OldStore`/iterators are unqualified module items. The collected
  panic file needed the keyed store in a submodule for exactly this.
- **The generic transform's handle bound rides `HandleToRef::Handle`,
  not a read-trait bound on `Dst`** — `Dst` is the transaction, not a
  graph; conflating the two was the first downstream draft's E0277.
  `AddNode<MN>` pins `MN` for inference (the unique impl), which then
  fixes `HandleToRef<MN>`.
- **`splice_node` splices an unconnected node** — removing an incident
  edge's endpoint without staging the edge's removal is a kept-dangler
  FlushError (the test's first draft proved the pipeline rejects it);
  the portability test is about the trait web resolving, not about
  cascade policies.

- **Step 12** (this commit): phased stores — the Tx gate lifts for flat
  stores binding a phased (`csr_adjacency`) backing. `TxModel` grew
  `phased: bool` + `csr: Vec<TxCsr>`; the flat classification gains the
  csr carry class (`many` + `is_phased()`: no per-record fields, an
  `on_insert(a, b)` hook, frozen store fields shaped exactly as
  `offsets: Vec<u32>` + `adj: Vec<BId>` — matched from the collected
  store fields by type; a root-sourced phased backing stays rejected).
  tx.rs: three phase positions (`Store<Frozen>` for the borrowed old
  store/FlushOutcome/`old()`, `Store<Build>` for the skeleton and the
  custom-op closure — a non-phased store is bare at all three); the
  skeleton literal gains `_phase: PhantomData` on a phased store; NEW
  step 4c-pre csr carry per binding — after the skeleton (the kept
  records' `on_add` fires opened one bucket per kept host, in
  compaction order) and before replay (staged inserts append to carried
  tails): walk each kept host's OLD FROZEN `offsets[old]..offsets[old+1]`
  slice of `adj` in forward order, skip removed members, remap, feed the
  backing's `__insert_<arrow>` hook (a pure bucket append); the ending
  becomes `finish()` (freeze, then validate — finalize BEFORE validate)
  with the frozen store as the FlushOutcome payload. `into_edit` is not
  emitted for a phased store — `OldStore` degenerates to the single
  borrowed variant and the skeleton match is irrefutable. Composed
  phased stores keep gating the Tx off (`tx_ok = !phased`), so the
  composed model carries `phased: false`. Tests: tests/tx_phased_flush.rs
  (12 — probe 1's 12 recast over the stdlib Digraph with
  `out_edges: csr_adjacency` + `in_edges: intrusive_list`, the probe's
  exact shape; the tests live inside the `implement!`ing module so the
  corruption tests reach the private fields). 48/48 suites green incl.
  trybuild + downstream, zero warnings.

Step-12 notes (scope decisions + probe deviations, each reasoned):

- **`into_edit` is not emitted for phased stores.** In-place forward
  compaction against live build-phase accumulators (buckets that must be
  re-opened per kept node mid-compaction) is a different design surface
  than the arena in-place path, and no probe pins it. The plan's step-12
  text says "Tx borrows `Store<Frozen>`" — the owned path stays
  non-phased-only this milestone.
- **The probe's finding c.2 splits against the generated validators.**
  The probe's hand-written validate was structural-only, so a truncated
  in-chain passed; the generated intrusive validator runs M2's partition
  walk and CATCHES the probe's exact truncation poke (names the missing
  edge). The generated CSR validator checks only offsets invariants
  (length/span/monotonicity — not membership or order), so the recast
  test (`validate_catches_truncation_but_misses_reorder`) truncates on
  the intrusive side (caught) and reorders within a CSR range (not
  caught) — the probe's lesson in generated form.
- **The generated `Display` prints raw ids** (`endpoint: 1`) where the
  probe printed typed ids (`NodeId(1)`); the recast asserts the
  structured violation vector (stronger) plus a light Display check.
- **The walk reads raw frozen fields, not the read view.** The read
  view's `many` getters are handle-graph-shaped (`graph().nodes()…`); the
  emitted walk indexes `old.offsets`/`old.adj` directly — the same
  module-private access the corruption tests use, and identical to the
  probe's `adj[out_range]` shape. Bounds are presupposed (the old store
  was frozen = validated), the same presupposition the intrusive walk
  already made.
- **Staged inserts on a phased backing ride the existing replay**: a
  staged `add_edge` fires the by-source orchestration (bucket append +
  intrusive link) through the real ctor — no phased-specific staging.
  Kept edges' records are built by the skeleton with NO backing hooks
  (the model's per-entity `onadd_hooks` carries only A-sourced bucket
  opens; the B records' hooks fire only in replay) — exactly the probe's
  4a/4b split.

- **Step 11** (this commit): removal + keyed carry — the per-record keyed
  map ungated (HierarchyStore's `sub.subnode`, gated since step 8).
  `TxKeyed` grew `host: Option<Ident2>` (the map-owning entity; None =
  store-level) and `member_host: Option<Ident2>` (an embedded member's
  target is a raw path id in the op — no ref can name an embedded
  record). Flat codegen (`codegen.rs`) classifies keyed with the
  record-self allowed and wires the verb like a hosted primitive insert;
  composed codegen (`codegen_composed.rs`) reclassifies: key must be
  root-level, member root OR embedded-under-the-fused-class, `member_host`
  from the member's `ClassLoc`, instance-qualified hook/`key_live`
  emission on the fused class (`__insert_sub_subnode(a, k, b)`,
  `__sub_subnode_key_live`). tx.rs: keyed op variants in three
  (host, member_host) shapes; keyed staging arms — store-level keeps the
  `__<arrow>_key_matches` helper + Custom reserve arm (now filtered to
  hostless arrows), hosted arm range-guards the old-map live-key check
  (`old.__store().#key_live(hid, kid)`), staged-vs-staged same-(host,key)
  panics; step-2 gains `s2_key_removed` (the survivors-only cascade —
  kept-owner entries whose target survives count, doomed owners don't,
  staged inserts keyed by a removed key DO count and attribute to the
  removal op; entries sorted, `KeyRemovedWithLiveEntries{op, arena, key,
  entries: Vec<(owner, key)>, staged_inserts}` — owner column 0 for
  store-level maps, one unified variant for both placements) and
  `s2_path_keyed` (Existing-only path-id targets: Pending host →
  `InsertHostTargetMismatch`, host≠target.host → mismatch Old, target idx
  removed → `DanglingInsertTarget`, all range-guarded by the old embedded
  arena length); `HostName` enum emitted beside RecordDesc when any
  member_host exists. Step 4a `record_field_init` carries a kept host's
  keyed map filtered + remapped (embedded target: host map ∘ per-host sub
  map composed remap); step 4c replays staged keyed inserts through the
  same composed remap into `new_store.#hook(a, k, b)`. Tests:
  tests/tx_keyed_carry.rs (15 — probe 4's 15 recast over HierarchyStore,
  the probe's Tag/Outer/Sub/note vocabulary mapped onto InnerNode/
  OuterNode/SubNode/`main_node`-field dangler), tests/hierarchy_subgraphs.rs
  Tx ungated. 47/47 suites green incl. trybuild + downstream, zero
  warnings.

Step-11 notes (scope decisions + deviations, each reasoned):

- **Store-level staged-arm semantics change** (the survivors-only cascade
  at the staging site): removing a key that a staged insert references is
  now `KeyRemovedWithLiveEntries` attributed to the REMOVAL op (the
  staged op's index in `staged_inserts`), not a `DanglingEndpoint` at the
  insert. Two legacy tests adjusted with comments:
  `tests/edge_induced.rs` (`tx_removal_splices_intrusive_and_keyed_carries`
  gains the KeyRemoved violation; a remove-edge alone now reports it) and
  `tests/tx_keyed.rs` (`keyed_insert_danglers_and_unknown_ids`: the
  staged insert keyed by a removed key attributes to op 1). Both match
  the probe-pinned policy.
- **Embedded intrusive chains stay gated** (step 9's deferral stands; the
  plan's step-11 text doesn't name them — per-host intrusive carry is a
  different surface from the keyed map).
- **`ReservedKey` variants only for store-level keyed arrows.**
  Per-record reservations are out of the acceptance shape (probe 4 pins
  store-level reserve-key flow only); `reserved_key_variants` is filtered
  `kk.host.is_none()`.
- **Embedded-target verbs are inherent-only.** The canonical hosted hook
  takes a target ref; a verb for an embedded member would need a raw
  path id — emitted inherent, not on the `Insert<...>` trait.
- **Value-host-agreement damage is validated on the store, not through a
  flush.** The acceptance test pokes a host's keyed map below the Tx's
  checks and asserts the composed validate names both hosts (path-id
  shape); routing through a flush would LAUNDER the damage — step 4a's
  embedded-target remap assumes the host-agreement invariant (value host
  = owner) and re-anchors the corrupt value. The probe pinned the
  validate surface; the flush-side launder is the carry working as
  designed on already-corrupt state.
- **The composed remap lives at two call sites** (4a carry, 4c
  staged-insert replay) and both take host map ∘ per-host sub map;
  `s4_keyed_maps` (store-level carry) is filtered to hostless arrows —
  hosted maps carry per host record via `record_field_init`, not the
  store-level pass (an early unfiltered run misindexed host arenas).
- **Kept-pass danglers precede KeyRemoved in discovery order.** The
  kept-record pass runs before the removed-key pass, so a kept sub whose
  `main_node` is the removed key lists first (the probe's DanglingNoteTag
  analog); tests assert the full ordered vector.
- **Report exposure ("old ids only") was step 3's surface**; this step
  pins the composed-remap read-offs (report maps index OLD ids; staged
  adds land past the map's end) in the happy-path test.
- **`InsertHostTargetMismatch` is a step-2 flush error, not a staging
  panic** (the per-step guide said "staging panic"; the probe chose the
  flush error — REPORT pin: an Existing path-id target under a Pending
  or mismatched host is detected at analysis, with the violation
  attributed to the op like every other step-2 finding).
- **Spike 7's 14 removal tests landed with step 3** (tests/tx_flush.rs,
  the Königsberg port); this step's acceptance is probe 4's 15 alone.

- **Step 10** (this commit): induction via the escape hatch (decision 4).
  Emitted by tx.rs from the UNCHANGED TxModel — the custom-op pieces are
  store-generic: `Op::Custom { run: Option<Box<dyn FnOnce(&mut Store,
  &Resolve<'_>)>>, named: Vec<__NamedRef>, reserve: Vec<ReservedKey> }`
  (always emitted; the manual `Debug` impl for `Op` replaces the derive —
  closures aren't `Debug`; `Custom` prints its `named`/`reserve` with the
  closure elided). `__NamedRef` — one `Entity(u32)` variant per ROOT-LEVEL
  entity: old ids a custom op names; step 1 range-checks them
  (`UnknownOldId`), step 2 rejects removed ones AT THE CUSTOM OP
  (`DanglingEndpoint`/`Staged` — the probe's naming-op rule). `ReservedKey`
  — one variant per store-level keyed arrow carrying
  `__SubmissionKey::{Old,Pending}(u32)`; reservation rides INSIDE the op
  queue (not a side list) because a keyed insert staged BEFORE the custom
  op must not see its reserved keys — the probe pins both orderings.
  `Resolve<'a>` — maps + pending tables of root-level entities with
  `resolve_<entity>` methods (bounds-checked pending get, internal-bug
  panic, decision 5's shape); NO old-store reference (the owned path has
  consumed old by replay — the closure reads the rebuilt store; staging-
  time reads go through `Tx::old`). `__stage_custom(run, named, reserve)`
  + `__ops()` doc-hidden on Tx; the keyed staging scan grew a `Custom`
  reserve arm via a per-arrow `Self::__<arrow>_key_matches` helper
  (kind-aware: Existing↔Old, Pending↔Pending — the t3 pin). Flush: pass 1
  switched to `ops.iter_mut().enumerate()` so the custom arm can
  `.take()` the closure (a custom op replays exactly once); the analysis
  match grew the `Custom` arm. A custom op fills no replay table (refs
  mint through the add verbs; closure-inserted records are reachable only
  through the store). Tests: tests/edge_induced.rs +14 — the induct-flush
  probe's 13 flat tests recast against the generated surface through a
  user-written `impl Tx<'_> { add_subedge }` extension (staging-time key
  computation from `old()` + `__ops()`, closure replay through the store's
  inducing constructor), plus a wholly-staged Pending-edge induction and
  an into_edit owned-path pin. 46/46 suites green incl. trybuild +
  downstream, zero warnings.

Step-10 notes (scope decisions + deviations, each reasoned):

- **`__reserve_keys` merged into `__stage_custom`'s payload** (deviation
  from decision 4's two-method sketch): a side list on Tx loses submission
  order — every staged insert, earlier or later, would see every
  reservation, and the probe's pinned ordering (standalone BEFORE a staged
  induction is legal; AFTER is a panic) would break. The reservation
  lives in the op it belongs to.
- **Custom ops are root-level.** `__NamedRef` and `Resolve` cover
  root-level entities only; a custom op replays in pass 1 and cannot name
  an embedded record (path ids have no root-level meaning). Custom ops in
  nested stores replay before all pass-2 nested ops (the grouping is
  root-then-nested for every root op; nothing in the acceptance shape
  binds a custom op against a nested op's replay order).
- **The probe's keyed cascade recast as the explicit minimal cascade.**
  The probe removes a main node and its induced subnode cascades through
  the keyed map — step 11's survivors-only cascade, unimplemented here.
  The recast stages `remove_node` + `remove_sub_node` (and edge +
  subedge) explicitly, which the generated store requires today anyway
  (the kept sub record's `main_node` field would dangle otherwise) — the
  same translation the step-8 tests already made.
- **Staging-time induction-key reads are range-guarded by the extension
  itself** (the probe's `induction_keys_of` shape: unresolvable → no
  reserve keys; step 1 still reports the `UnknownOldId` for the named id
  at flush). The generated read getters index unconditionally.
- **Custom ops are opaque to determinism** (decision 4's predicted cost):
  analysis sees only their `named` ids (deterministic); what a closure
  does is user code. The `Debug` impl prints `named`/`reserve` only.
- **Composed-renamed downstream caught a real emission bug**: the
  key-match helper is emitted inside `impl Tx<'_>` (where keyed_staging
  lands), so the staging arm must call it as `Self::__<arrow>_key_matches`
  — bare associated-function paths do not resolve.

Step-6 notes (deviations from this handoff's pre-work, each reasoned):

- **Marker dedup replaced the collision check.** The pre-work pinned "a
  pattern!-time collision check against camel(arrow) markers"; on
  contact the stdlib itself trips it — `MainNode`/`MainEdge` root
  patterns declare a *closed* target entity whose verbatim name equals
  the arrow's camel marker. Renaming those entities would churn
  root-fixed read assoc names for no gain, so instead the emission
  dedups: a name already minted by an arrow marker is not re-minted —
  one struct serves the read impl of that arrow and the write impls of
  that entity (the traits differ; no impl can conflict). The markers
  module is unique by construction; no error surface.
- **Inserts DO get step-1/2 checks now.** The pre-work deferred all
  step-1/2 checks for Insert ops to the keyed steps; but the replay
  resolve indexes the compaction map with `.expect("validated in
  steps 1-2")`, and an unvalidated raw id or a removed endpoint would
  panic at flush on user error — exactly what the pipeline's contract
  forbids. Insert refs now range-validate (`UnknownOldId`) and a staged
  insert naming a removed host/member is a `DanglingEndpoint`
  attributed to the insert op (same shape as staged-add danglers). The
  keyed live-key checks still land with steps 8/11.
- **Generic transforms thread one marker per bound trait.** Spike 8's
  hand-written surface used one marker for everything; the generated
  surface has M1's per-arrow read markers, so the recast copy/replace
  bounds carry MN/MO/MS/MT (read) + MEN/MEE (per-entity write) —
  inference resolves them from the unique impls. `HandleToRef`'s
  per-entity marker is the same struct `Add`/`Remove` use, so the
  equality web collapses onto MEN/MEE.
- **"Two marker instantiations" reads as two store types here.** A
  second marker set over the same pattern requires composition, and
  composed Tx lands at step 8; the acceptance instantiates the
  transforms at the Digraph markers against two `implement!` stores
  (Ka→Kb cross-store copy; Ka replace vs direct staging).
- **Per-entity emission skips OPEN entities.** Their records live in a
  fused peer store, so existence ops have nothing to name; this also
  keeps Subgraph's open `MainNode`/`MainEdge` clear of the `main_node`/
  `main_edge` arrow markers. Nested/composed Tx (steps 8/9) sees open
  entities through their host.
- Duplicate membership (re-inserting an already-listed member through
  the primitive insert verb) corrupts the intrusive chain — same hazard
  as the raw store op; the Tx does not guard it, and the plan's
  staging-panic list doesn't ask it to. Noted on the verb's doc.

## Step 7 notes (deviations/adjustments, each reasoned)

- **Three existing tests pinned the field-only-write semantics and were
  adjusted, not kept verbatim.** The handoff's "keep it (it pins the
  hooks)" for tests/unlink.rs's by_derived trio predates the grown hook:
  with `__set_source` orchestrating unlink+write+relink, the manual trio
  double-links. Adjusted: by_derived + composed_impl now exercise the
  builder op as the one-call orchestration (the `primitive` module still
  pins `__unlink` head/middle/tail individually; composed keeps the
  instance-qualified hooks pinned via the total-absence answer).
  tests/koenigsberg.rs's `set_one_arrow_writes_through_named_hook`
  asserted "by-derived membership is not re-derived, by design" — that
  design died with this step; the test now pins the re-derivation (c's
  in-list gains, b's loses, validate green).
  tests/validate_failures.rs's determinant-contradiction/coverage pin
  corrupted via `set_target(e, a)`; with the orchestration, no public op
  can leave field and lists disagreeing (no pub un-insert exists — a
  plan non-goal), so the corruption moved below the orchestration
  (unlink + wrong-host insert through the private hooks) and the test
  moved inside `mod flat` for hook access — the file's own `corrupt`
  submodule pattern.
- **Step-2 pass order: set-op checks before the kept pass** (probe 3's
  discovery order — the set op's own violations first, then removal-
  attributed danglers). Existing suites emit no set ops, so no prior
  ordering changed; new tests pin exact vectors.
- **Decision 6 checked and confirmed**: tests/csr.rs exercises no
  builder `set_` on the csr store (grep-clean), so skipping phased
  derivatives leaves no stale-bucket test exposure. The csr × set_
  residual (a set_ re-pointing a determinant of a by-derived csr arrow
  leaves carried buckets stale) remains noted for a real binding.
- **`(id).0 as usize < count` needed double parens** — `<` after `as
  usize` parses as generics (the suite's downstream compiles caught it);
  the existing `<` checks all double-parenthesize.

## Architecture decisions (settled in the survey, keep unless forced)

1. **Generic tx pieces in the runtime crate** (`src/tx.rs`, landed):
   token, Ref, stamp check, `HandleToRef<M>`. Everything pattern-specific
   — `Tx<'a>`, `Op`, staging, `flush`, error/report types, write-trait
   impls — is emitted by `macros/src/tx.rs` from a `TxModel` built by
   BOTH flat and composed codegen, so the flush pipeline exists in
   exactly one place. Grow the model per step (set ops → 7, primitive
   inserts → 6, paths/nesting → 9, keyed carry → 11, phased → 12).
2. **Shape gate**: `tx::emit` supports flat ∧ non-phased ∧ keyless
   today; other shapes emit no `Tx` (no `edit()`) until their step
   lands. Main's tests never call `edit()` on those shapes
   mid-milestone, so the suite stays green. Final state: all shapes.
3. **Flush skeleton copy bypasses constructors** (the probes' shape):
   kept records pushed as raw record literals — ctor-arg id fields
   remapped through the compaction maps, `Option` links `None`,
   Vec/HashMap `Default` — then `on_add` hooks fired manually, then
   per-backing carry (intrusive: rebuild-by-walk; keyed: filter+remap
   carry, never rebuilt; csr: walk old frozen ranges feeding `__insert_`
   hooks), then staged ops replayed through the REAL constructors/hooks
   in submission order. validate() at the end (defensive Err arm
   `ValidateFailed`, D4).
4. **Induction (step 10) via a generated escape hatch** (my design, not
   yet probed): `Op::Custom(Box<dyn FnOnce(&mut Store, &Resolve<'_>)>)`
   + doc-hidden `Tx::__stage_custom` + `__reserve_keys(Vec<ReservedKey>)`
   where `ReservedKey` is a generated one-variant-per-keyed-arrow enum
   carrying the (host-,)key refs. User writes `impl Tx<'_> { add_subedge
   }` in the same crate (works — the store/Tx types are generated into
   the user's module; today's edge_induced.rs already extends the STORE
   this way), computing induced keys from `old()` + a doc-hidden
   ops-readonly accessor, staging the closure for replay. This keeps
   induction undeclared (plan's Induction section) while the live-key
   staging check still sees induced keys (the induct-flush probe's
   sharpened reading). Cost: Custom ops are not inspectable — note in
   step-10 notes. If review rejects the closure, the alternative is an
   optional `implement!` induction clause — but that contradicts
   "undeclared is the normal case"; the closure is the faithful reading.
5. **Step 9's decision obligation (replay-table miss)**: decided —
   same-crate unit test in the runtime crate (an in-crate `implement!`
   can forge a stamped ref against module-private Tx internals; a
   downstream tests/ crate cannot — probe 2's miss test forged
   same-crate). Emission: bounds-checked `get` + internal-bug panic
   message.
6. **csr × set_ residual question**: plan says "a phased backing needs
   nothing" for the set_ orchestration's derivative relink. Emission
   follows that (no unlink, no relink for phased derivatives — the field
   write only), leaving carried buckets stale if a set_ re-points a
   determinant of a by-derived csr arrow. No current test binds that
   combination (csr.rs binds out_edges: csr on Digraph — set_source
   exists there! Check tests/csr.rs when doing step 7; if the builder
   set_ on that store is exercised, revisit: either bucket-splice via a
   new on_unlink in csr's declaration, or an implement-time rejection
   naming the combination).
7. **Error/report surface, generated per store** (probes' shapes):
   `Violation` enum — UnknownOldId{op,arena,id}, DoubleRemoval{op,arena,id},
   DanglingEndpoint{op, record: Kept(u32)|Staged(op), endpoint_arena, endpoint},
   SetOfRemoved{op,arena,id,removed_by} (step 7), RemovedPathHost{ops
   asc, host, removed_by} (step 9), KeyRemovedWithLiveEntries{op,key,
   entries sorted} + keyed coherence (step 11), ValidateFailed(String).
   Deterministic order: step order then op index; anything derived from
   a HashMap gets collected+sorted (probe 4's byte-identical Display
   test, step 4). `FlushOutcome { store, report }`; report = per-arena
   `Vec<Option<u32>>` (old-id-indexed; embedded arenas gain per-old-host
   sub-maps at step 9, composed remap = host map ∘ per-host sub map).
   `check(&self)` = steps 1–2 factored pure (same analysis struct flush
   consumes); committed-mode contract: green check → into_edit flush
   commits (step 4).
8. **Write traits (step 6)**: arrow-named verbs root-emitted — pattern!
   root emission grows the write trait beside the read trait
   (Insert<Arrow> for many/keyed roots, Set<Arrow> for one roots) and
   the courier metadata tuple gains the SIXTH, braced element
   `{ Trait, method, ARef, KRef?, BRef }` (ref assoc names = `<Entity>Ref`,
   spike 8's de facto contract: AddNode/RemoveNode/InsertOutEdges with
   NodeRef/EdgeRef assoc types; AddEdge = per-pattern constructor trait
   with ctor-arg assoc types per arrow: type Edge; type Source; type
   Target; fn add_edge(...) -> Self::Edge). Existence traits are
   DECLARING-BLOCK-owned: pattern! (and compose re-mint) emits
   Add<E>/Remove<E> per NON-ROOT entity + a per-entity marker in the
   same markers module (entity name verbatim; add a pattern!-time
   collision check against camel(arrow) markers). Canonical arity: self
   ref always present in the trait method; unfused-root forms are
   inherent-only on Tx (no trait impl emitted). `HandleToRef<M>` impls
   per entity under per-entity markers. impl! emits Tx trait impls only
   where the verb is real (existence many → no Insert impl; by-derived →
   none; keyed → Insert with key).

## Step 6 pre-work — Add<E> ctor-arg arity SETTLED (impl-m4-3)

The emission mechanics are clear; the one design question — whose
trait signature carries the entity's ctor args when one-ness of an
instantiation's arrows is ROOT-side data the declaring pattern cannot
see at expansion time (`pattern!` for Digraph cannot resolve
`stdlib::Source`'s cardinality; the plan says "the constructor
argument list is the entity's total `one` arrows" but never how the
declaring block learns it) — was settled with Gordon 2026-10-12:

**By-derived only, skip opaque.** An `Add<E>` trait's ctor args are
the arrows the declaring pattern can PROVE total at expansion time:

1. inline-declared `one` arrows (a root pattern knows its own
   cardinality), and
2. arrows named by a sibling `by` clause — `by` requires a total
   `one` determinant; the pattern-time check (macros/src/lib.rs:304)
   already validates the determinant is a declared opposite-running
   sibling, and implement-time codegen enforces the bound-`one`
   requirement (macros/src/codegen.rs:1124), so a contradicting
   instantiation fails at `implement!` before any store compiles —
   the derived arity cannot be silently wrong.

Args in arrow-declaration order (Digraph's Edge: source, target —
exactly spike 8's `add_edge(source, target)`). An entity with ANY
*opaque* outgoing arrow (a plain instantiation — Subgraph's SubNode
via `main_node: stdlib::MainNode`) gets NO Add trait emitted:
`Remove<E>` and the per-entity marker still emit, and the verb stays
Tx-inherent (step 1 already emits `add_<entity>` with the real
implement-time arity). Gap flagged in step notes; when a real
mechanism lands, the skipped Add traits appear additively.

Rejected alternatives (for the record): an argless placeholder Add
trait would falsely read as "zero ctor args" — indistinguishable from
Digraph's Node, which legitimately has none — and its shape must
break later; an assoc-type arg tuple (`args: Self::CtorArgs`) covers
every shape today but deviates from the spike-8-pinned and
plan-reviewed multi-param contract; a grammar clause asserting
cardinality at instantiation restates root-side truth, which M1's
"instantiation blocks never restate cardinality" rule single-sources
at the root.

Facts verified this session, before writing code:

- The runtime piece of decision 8 already landed with step 1:
  `HandleToRef<M>` in src/tx.rs with assoc types `Handle`/`Ref` and
  method `to_ref` (NOT spike 8's per-entity `node_ref`/`edge_ref`
  traits — the plan's single marker-generic trait, one impl per
  entity under the per-entity marker).
- Spike 8's `Commit` trait is NOT part of the generated surface —
  "flush is deliberately not marker-generic"; the acceptance test
  calls `tx.flush()` concretely.
- The recast copy bounds on `AddNode + AddEdge` only (Digraph's
  out_edges is by-derived — no InsertOutEdges for it): AddEdge's
  assoc types are `EdgeRef` (the entity, `<Entity>Ref` naming) plus
  `Source`/`Target` (camel of the ctor arrows), with the copy's
  equality web setting `Source = Target = AddNode::NodeRef`. Within
  one Add trait, a cheap assoc-name collision check (camel(arrow)
  vs `<Entity>Ref` vs sibling camels) belongs beside the marker
  collision check.
- Starting points read and current: RootMeta is five slots today
  (method, assoc, gat, card, key — grammar.rs:682, parse 724, emit
  763, round-trip test 996); the sixth braced write slot
  `{ Trait, method, ARef, KRef?, BRef }` extends it like M2's key
  slot. expand_root_pattern (lib.rs:374) grows the write traits
  beside the read trait; expand_instantiation_pattern (lib.rs:452)
  and finish_compose (lib.rs:925, marker minting ~964) grow the
  per-entity traits/markers; macros/src/tx.rs holds the TxModel and
  the whole Tx emission.

Everything else about step 6 is settled reading: sixth braced slot
`{ Trait, method, ARef, KRef?, BRef }` on RootMeta (grammar.rs parse/
emit + round-trip tests, like the M2 key slot); Insert<Arrow> for
many/keyed roots and Set<Arrow> for one roots emitted beside the read
trait in expand_root_pattern; Add<E>/Remove<E> per non-root entity +
per-entity markers (entity name verbatim) in expand_instantiation_
pattern AND finish_compose (re-mint), with a pattern!-time collision
check against camel(arrow) markers; canonical arity with self ref
always present, unfused-root forms inherent-only; runtime
HandleToRef<M> impls per entity under per-entity markers; Tx trait
impls only where the verb is real. New staging verb `insert_<arrow>`
for primitive many arrows queues a new `Op::Insert<Arrow>` variant —
remember flush's match arms grow with it (replay through
`__insert_<arrow>`; no step-1/2 checks for it until keyed live-key
lands at 8/11).

## Per-step continuation guide (3–14)

Acceptance instruments: probe crates under /tmp are the reference
implementations — reproduce their tests against GENERATED code.
Spike-7's 32 tests live in /tmp/spike7-tx (src/tx.rs + tests); spike 8
at /tmp/spike8-transform (src/write.rs = the name contract,
src/transform.rs = the two scenarios).

- **3. Flush (flat, non-phased)** — the big one. Extend TxModel with
  arrows: per bound arrow a `TxCarry` classification (Existence |
  FieldOne{field, a_ent, b_ent} | Intrusive{head owner (record/store) +
  head field, link field on B, insert/unlink hooks, det} | Keyed | Csr —
  last two rejected at build until steps 11/12). Emit `flush(self) ->
  Result<FlushOutcome, FlushError>` following /tmp/spike7-tx/src/tx.rs
  and /tmp/m4-probe-set-unlink/src/main.rs (the pipeline WITH honor
  semantics — read its flush once before emitting): step 1 removals +
  range-guarded Existing validation (range-guard EVERY old-id read —
  spike7-oob-repro's bug: step 1 flags but doesn't return before step 2
  indexes); step 2 danglers over kept records per one-arrow field
  (Kept, attributed to the removal op) + staged adds naming removed
  endpoints (Staged, attributed to the naming op); step 3 compaction
  maps; step 4 skeleton (records with remapped id fields, on_add hooks,
  intrusive rebuild-by-walk per binding — head at record vs store) then
  submission-order replay through constructors with a per-entity
  pending table (`pending[i].expect("internal bug")`); step 7
  validate(). Dangler detection generalizes spike 7's source/target
  loop: for EVERY bound one-arrow, kept records whose field target was
  removed. Königsberg signature-equality test: port from spike 7's
  tests (tx-built vs directly-built). Out-of-range Existing → the
  listed error, never a panic — port /tmp/spike7-oob-repro's test.
- **4. check() + determinism** — factor steps 1–2 into
  `fn analyze(old, ops) -> Result<Analysis, FlushError>`; `pub fn
  check(&self)` calls it; flush consumes. Byte-identical Display across
  runs (probe 4's error_display_is_deterministic pattern: six
  independent builds). Committed-mode contract test (check green →
  flush commits) can only be exercised fully at step 5 (into_edit);
  test the check-then-flush path via edit() now and note it.
- **5. into_edit()** — `OldStore` enum shape from spike 7: Borrowed(&) |
  Owned(Box<Store>); derive-before-write, forward writes (write slot ≤
  read slot), the walk arrays computed before any in-place write. Port
  spike 7's into_edit tests. Note: with carried Vec<Op> closures (step
  10) into_edit stays fine — closures are FnOnce over the new store.
- **6. Write-trait codegen** — decision 8 above. Changes: grammar.rs
  (RootMeta gains the write slot, parse/emit + round-trip tests),
  macros/lib.rs expand_root_pattern (emit write traits + slot),
  per-entity traits in expand_instantiation_pattern + finish_compose
  (markers re-mint), TxModel gains per-arrow/per-entity trait-impl data
  (trait path from courier meta; marker path like read impls), and the
  staging verbs: insert_<arrow> for primitive many (queue Insert op),
  so spike 8's copy (AddNode+AddEdge) and replace_node (full web incl.
  HandleToRef) compile at two marker instantiations against generated
  code (tests/tx_write_traits.rs, modeled on spike8's tests/transform.rs
  with the recast bounds the plan names). Keyed insert staging can land
  here or with 8/11 — needs __key_live predicates in the FLAT codegen
  (composed already emits them; copy that emission).
- **7. set_ + grown hooks** — set_ staging (panic on Pending self, step
  2's deferred test), Op::Set<Arrow>, honor semantics in flush steps
  1–2 (eff overrides last-wins; SetOfRemoved; set_-naming-removed
  attributed to the set op; set_ cannot cure the far side), skeleton
  born-unlinked honor case with Pending prediction (kept + preceding
  adds of the target entity, asserted at replay). Grow `__set_<arrow>`
  (M2's field-write hook) into the re-point orchestration in BOTH
  codegens: for each by-derived arrow on this field, unlink(old_host,
  rec) if its backing authors on_unlink, write, insert(new_host, rec)
  if it authors on_insert — SKIP phased derivatives entirely (decision
  6). tests/unlink.rs's by_derived test already drives the trio
  manually — after step 7 the builder set_source does it in one call;
  update that test? NO — keep it (it pins the hooks); add probe 3's
  tests as tests/tx_set_unlink.rs (head/middle/tail, honor, double
  set_ last-wins, same-verbs-in-order normalization, pending-new-source
  honor case).
- **8. Unfused-root forms** — self-less insert staging (primitive:
  insert_<arrow>(member_ref); keyed: insert_<arrow>(key_ref, target))
  at unfused-root selves, store-level ops; no trait impls for those
  verbs (canonical arity unemittable). Acceptance: m3-probe-r3-root-
  intrusive + m2-probe-edge-induced tests as generated equivalents —
  the latter needs the COMPOSED codegen to build a TxModel (extend
  codegen_composed.rs with the same model construction; placements via
  ClassData). This is where composed Tx emission starts; flush for
  composed root-level-only stores (edge-induced: keyed at unfused root,
  flat arenas) needs keyed carry → may pull step 11's keyed-carry work
  forward for the store-level map case (filter+remap through two maps —
  no host composition at root level). Sequence 8 → 11 per the plan.
- **9. Nested stores** — `within(host_ref) -> SubTx{token, prefix}` (no
  borrow; nested calls thread `&mut Tx`, token re-checked), ops gain
  `path: Vec<Ref<HostId>>` (or a Path struct; codegen knows the one
  embedding depth — emit the fixed-shape path), per-old-host sub maps,
  RemovedPathHost violation (staged ops' indices ascending + removal
  op), root-before-nested replay grouping, per-level Pending resolution
  (Existing through per-level maps, Pending through the root pass's
  op-index replay table — bounds-checked get, decision 5's same-crate
  miss test in src/stdlib.rs tests or a new in-crate tests module).
  Acceptance: m4-probe-nested-pending's 10 tests. Read its REPORT first.
- **10. Induction** — decision 4's escape hatch; the induct-flush probe
  (/tmp/m4-probe-induct-flush, 17 tests — REPORT has the full inventory
  and the three fixed failures' lessons: standalone = record + keyed
  insert BOTH; live-key check counts induced keys in submission space)
  + edge_induced.rs's tests as generated equivalents. Extend
  edge_induced.rs (or a new test file) with the Tx extension impl.
- **11. Removal + keyed carry** — report exposure, survivors-only
  cascade ("live" = surviving the change set, all three reference
  sites — doomed-host entries cascade-covered; staged insert keyed by a
  removed key counts live and errors), keyed maps carried never rebuilt,
  composed remap at carry AND staged-insert replay, Existing-only
  keyed host/target for path-id targets (staging panic), cascade
  through keyed maps on removal (induct-flush's keyed cascade).
  Acceptance: m4-probe-keyed-carry's 15 + spike 7's 14 removal tests.
- **12. Phased** — Tx borrows `Store<Frozen>`, flush builds through
  `Store<Build>` (skeleton via raw pushes + on_add bucket growth; csr
  carry walks old frozen ranges feeding `__insert_` hooks; replay;
  `finish()` = freeze + validate). Acceptance: m4-probe-phased-flush's
  12 tests. Its REPORT pins: 4a–4d sub-order, old-store reads are
  Frozen-phase reads only, per-field remap is WRONG (rebuild-by-walk),
  validate cannot backstop semantic truncation.
- **13. Negative suite** — trybuild fixtures (read-on-ref done; add
  wrong-marker bounds, handle survival under into_edit), collect the
  staging should_panics in one place, downstream-crate transform
  portability test (M1 step 9's pattern — a new tests/downstream crate
  or extend an existing one).
- **14. Doc pass** — crate-root docs (transaction/flush/pass
  discipline), macro docs (implement! gains the write side), the
  backing! doc for on_unlink already landed with M3.

Rules of engagement (from the handing-off session): no subagents; the
plan's checklist is the queue; tick a step's boxes in the same commit as
the work; full workspace suite (trybuild + downstream included) green
before each commit; new probe/scratch crates under /tmp only, never
committed; spikes.md row if a probe earns one; landed code wins over
plan text, note disagreements in step notes; do not merge to main —
leave the branch for review; do not touch m5-plan/m6-plan worktrees.

## Session log

- 2026-10-13 (impl-m4-8): step 13 DONE (see "Where things stand" and the
  step-13 notes — three new trybuild fixtures: write_wrong_marker E0277
  with the correct marker named in the help, handle_survives_into_edit
  E0505, phased_no_into_edit E0599; the staging should_panics collected
  into tx_staging_panics.rs with the fixture-bound inventory documented;
  downstream/basic gained the write-side portability module: concrete
  edit/flush smoke + the splice_node generic transform over the
  Add/Remove/HandleToRef bound web). One module rule documented: one
  Tx-emitting implement! per module (emitted Tx types are
  unqualified). 48/48 suites green incl. trybuild + downstream, zero
  warnings at the commit. Next: step 14 (doc pass — crate-root docs for
  transaction/flush/pass discipline, macro docs).

- 2026-10-13 (impl-m4-8): step 12 DONE (see "Where things stand" and the
  step-12 notes — TxModel phased/csr, the flat gate lifting for
  csr-bound stores, the three phase positions, the 4c-pre frozen-range
  walk feeding `__insert_<arrow>` buckets after the skeleton and before
  replay, `finish()` as the ending, borrowed-only OldStore for phased
  stores; probe 1's 12 tests recast as tests/tx_phased_flush.rs over the
  stdlib Digraph with csr out + intrusive in). One probe finding split:
  the generated intrusive validator catches the probe's truncation poke
  (partition walk) while the generated CSR validator misses in-range
  reorders — the recast test asserts both halves. 48/48 suites green
  incl. trybuild + downstream, zero warnings at the commit. Next: step
  13 (negative suite — trybuild fixtures beyond read-on-ref, the staging
  should_panics collected, downstream-crate transform portability).

- 2026-10-13 (impl-m4-8): step 11 DONE (see "Where things stand" and the
  step-11 notes — TxKeyed host/member_host, the survivors-only cascade as
  `s2_key_removed` at BOTH placements, `s2_path_keyed` for Existing-only
  path-id targets (InsertHostTargetMismatch/DanglingInsertTarget), the
  composed remap at 4a carry and 4c staged-insert replay, the
  hosted keyed staging arm with old-map live-key range guards;
  HierarchyStore's `sub.subnode` Tx ungated; probe 4's 15 tests recast
  as tests/tx_keyed_carry.rs over HierarchyStore). Two legacy tests
  adjusted for the survivors-only staging-site semantics (edge_induced,
  tx_keyed — both pinned by the probe). 47/47 suites green incl.
  trybuild + downstream, zero warnings at the commit. Next: step 12
  (phased stores — flush into Store<Build>, kept-fed-then-replay carry,
  finish() = steps 6-7; acceptance: probe 1's 12 tests).

- 2026-10-13 (impl-m4-7): step 10 DONE (see "Where things stand" and the
  step-10 notes — the escape hatch emitted store-generically from the
  unchanged TxModel: Op::Custom + __NamedRef/ReservedKey/__SubmissionKey/
  Resolve/__stage_custom/__ops, the reserve-aware keyed staging scan, the
  naming-op rule for custom ops in steps 1-2, the iter_mut pass-1 replay;
  the induct-flush probe's 13 flat tests recast through a user-written Tx
  extension, plus wholly-staged Pending induction and the owned path).
  46/46 suites green, zero warnings at the commit. Next: step 11
  (removal + keyed carry — the survivors-only cascade, report exposure,
  staged-insert-on-removed-key counts live; the probe's 15 tests + spike
  7's 14 removal tests; the step-9 notes' per-host keyed maps
  (Hierarchy's `sub.subnode`) are the composed target).
- 2026-10-13 (impl-m4-6, cont.): step 9 DONE (see "Where things stand"
  and the step-9 notes — within/SubTx scopes, per-host removal sets
  and maps, RemovedPathHost, grouped root-then-nested replay with the
  bounds-checked replay-table get + same-crate forge test). 46/46
  suites green, zero warnings at the commit. Next: step 10
  (induction — the induct-flush probe's 17 tests + the edge-induced
  store's Tx extension impl; the probe's REPORT pins the
  standalone-equals-record-plus-keyed-insert rule and the induced-key
  staging check).
- 2026-10-12 (impl-m4-6): step 8 DONE (see "Where things stand" and
  the step-8 notes — flat store-level keyed Tx, composed TxModel
  emission under the root-only gate, inherent-surface decision, the
  `basis` addition for collision-prefixed set verbs). Suite green
  incl. trybuild + downstream at the commit, zero warnings. Next:
  step 9 (nested stores — `within()`, ref-segment paths, per-level
  maps; the composed gate's `ClassLoc::Root` condition is where
  embedded classes come in).
- 2026-10-12 (impl-m4-5): step 7 DONE (see "Where things stand" and the
  step-7 notes above — the grown-hook emission, the honor pipeline, and
  the three adjusted legacy tests). Suite green incl. trybuild +
  downstream at the commit. Next: step 8 (unfused-root forms — self-less
  insert staging, store-level ops, and the START of composed Tx
  emission: codegen_composed.rs grows TxModel construction; edge-induced
  may pull step 11's keyed-carry work forward per the continuation
  guide).
- 2026-10-12 (impl-m4-4): step 6 DONE (see "Where things stand" and the
  step-6 notes above — five recorded deviations/reasonings). Suite green
  incl. trybuild + downstream at the commit. Next: step 7 (`set_` +
  grown hooks); decision 6's csr × set_ residual applies (check
  tests/csr.rs's builder usage of set_source when doing step 7).
- 2026-10-12 (impl-m4-3): step 6's Add<E> ctor-arg arity question
  settled with Gordon (by-derived only, skip opaque — see the
  pre-work section). No step-6 code written; session stopped at
  Gordon's request so the next agent implements from the settled
  decision. Worktree clean at 0a8124e before this handoff commit.
- 2026-10-12 (impl-m4-2): steps 3, 4, 5 DONE (4cbddf0, c22cbb2,
  20f1bda — see "Where things stand"). Step 6 investigated, not
  started; see the pre-work note below. Suite green incl. trybuild +
  downstream at every commit.
- Step 3 DONE (this commit): flush emitted from TxModel — the model grew
  per-entity record fields (ctor-arg/None/Default init), store fields,
  and intrusive-carry arrows (host record vs store head); codegen
  classifies each bound arrow's carry and gates the Tx off for an
  unclassifiable `many` backing (a custom non-list carry) beside the
  existing phased/keyed gates. Tests: tests/tx_flush.rs (22 — spike 7's
  tx_koenigsberg 11 + tx_removal minus its 4 staging panics, already
  step 2's file, plus the oob-repro regression) and
  tests/tx_flush_shapes.rs (3 smokes: primitive list carry, field
  danglers, root-headed single chain — the other emitted shapes were
  compile-only). Suite green incl. trybuild + downstream.
  Step-3 notes: (a) RecordDesc::Kept carries `{arena, id}` not the
  decision-7 `Kept(u32)` — a bare u32 is ambiguous in a multi-arena
  store; the arena rides the desc, not a sixth field. (b) into_edit
  entry points and their tests land with step 5 (all ports go through
  edit()); spike 7's 7 direct-build koenigsberg tests are already the
  M1 suite, unported. (c) honor semantics read from the set-unlink
  probe but vacuous here — no Set op exists until step 7; the pipeline
  shape (step-1 eff slot, step-2 honor pass) grows into it.
  (d) A staged Add for an entity with a primitive-membership list is
  born UNLISTED in the flush (insertion is not a ctor arg) — asserted
  in prim_list_carry_and_field_remap; step 6's insert_ staging verbs
  are where primitive membership is staged.
