# HANDOFF — M4 implementation (write-side builders)

Branch `worktree-m4-implement`, worktree `.claude/worktrees/m4-implement`,
based on main 4904680 + steps 1–2. Plan: plans/milestone-4-builders.md
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

Not started: steps 7–14.

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
