# Milestone 4: write-side builders — mutation by rebuild

**Goal**: change an existing graph without ever mutating a live store. The
write side is generated from the schema the same way the read side is: one
interface per arrow, marker-generic, so generic graph-transformation
algorithms are as portable as generic traversals.

Specification level: preliminary, with exceptions that have grown by
probe. The flat-`Digraph` case (change-set queue, tx-stamped refs, the
flush pipeline, removal with the id-remap report) is hand-built and
tested (32 tests, including Königsberg signature equality between
tx-built and directly-built stores — six steps as numbered at spike
time; step 6, backing finalization, postdates the spike and first
executed inside a flush in the phased-flush probe below). Flush into a
Build-phased store (kept-membership carry, replay, freeze, validate)
and nested staging under provisional hosts are probe-verified
(/tmp/m4-probe-phased-flush, /tmp/m4-probe-nested-pending — REPORT.md
in each). `set_` re-pointing of a `by`-derived membership — the
delete-hook growth, effective-endpoint dangler detection, replay
against the rebuilt skeleton — is likewise probe-verified
(/tmp/m4-probe-set-unlink), as is the keyed-map flush policy
(carry, composed remap, cascade split, two-flush re-keying;
/tmp/m4-probe-keyed-carry). What remains uncompiled: write traits as
generated code and induction.

## Premises (what M1–M3 established)

- Stores are immutable once built; construction uses `&mut` methods on the
  store (epic invariant 5). Handles borrow the store and identify by store
  pointer + index path. Terminology: the store in its `&mut` construction
  phase is *the builder*; the staging type M4 adds is *the transaction*;
  flush re-runs the builder machinery.
- Hooks are the single write path: every mutation primitive (field writes,
  intrusive links, hash-index maintenance) lives in a named per-backing
  hook method on the store (doc-hidden, `__insert_<arrow>`-shaped — M3's
  grammar section pins the convention), and the `add_*` constructors are
  thin orchestration (M1 commits to both names and shape).
- Validation has a named boundary (`validate()`), with M2's canonical
  inventory: key existence, value-in-arena, and value-host agreement
  (key uniqueness per self
  object is not a runtime check — it is enforced where the op is
  submitted: the builder and the staging call reject a live key, M2's
  keyed section) — and, per backing, **structural checks** (endpoint
  ids in arena range, intrusive
  link well-formedness, membership consistency between an arrow's links
  and its determining fields for `by`-derived arrows, and the
  intrusive-ownership partition check for *primitive* intrusive arrows,
  which have no determining field to be consistent with). A store with
  no keyed arrows still validates; keyless backings contribute the
  structural half. M2 callers invoke it explicitly; flush invokes it
  unconditionally.
- Nested stores are addressed by index paths (M2).
- Constructor replay inherits M2's combined collision rule for merged
  constructors under composition (colliding `ctor_arg` contributions
  are shared when the resolved target classes agree, instance-prefixed
  when they differ): flush replays through whatever constructors the
  composed schema generated, under that rule.
- CSR-style backings (M3) assume a build/flush boundary; M3's constraint
  annotations make the requirement explicit.

M4 builds nothing new for storage. It adds a *staging* type in front of
the existing construction machinery and a flush that replays staged
changes through it.

## Core model

```rust
let mut tx = store.edit();           // borrows &Store; reads and old
                                     // handles keep working while staging
let a = tx.add_node();               // provisional ref: opaque, tx-stamped
let b = tx.add_node();
tx.add_edge(a, b);                   // provisional refs
tx.add_edge(old_node, a);            // old-store elements by handle or id
tx.remove_edge(e);                   // removal: staged, applied at flush
tx.set_payload(e, w);                // re-point an optional/payload arrow
let out = tx.flush()?;               // validate + rebuild -> FlushOutcome
                                     // { store, report }; old store
                                     // untouched, old handles still valid
```

The pass discipline: work with a metagraph (handles live, reads happen),
stage edits against it, then end the pass — drop every handle, flush, and
continue with the new metagraph. Two entry points support this:

- `store.edit()` borrows the old store sharedly; old handles stay valid
  through and after the flush. For when old and new must coexist
  (diffing, migration).
- `store.into_edit()` *consumes* the old store. The borrow checker then
  requires every handle to be dead before the pass can end — the "let go
  of all references" discipline is enforced at compile time — and flush
  can reuse the old store's allocations instead of cloning. This is the
  normal form; `edit()` is the escape hatch. Allocation reuse carries an
  ordering constraint (spike-verified for flat, non-phased arenas): all
  derived data — kept sets, old→new maps, rebuilt link chains — must be
  computed before any in-place write, and writes must go forward (write
  slot ≤ read slot), so no slot is overwritten before its old value has
  been read. Reuse is scoped to that case for now: nested stores
  (embedded arenas whose host ids are themselves remapped mid-sweep)
  and phased stores (the rebuild goes through a fresh build-phase
  store, a different shape from the frozen one) clone until a stated
  cross-level discipline is wanted and verified. `into_edit()` is also
  the *committed* mode in error handling (flush semantics, below): a
  failed flush forfeits the consumed store unless the change set was
  checked first.

- **The transaction borrows the old store sharedly.** This is the
  ergonomic point of the design: you can traverse the graph (with real
  handles, real read traits) while staging its replacement. The
  interleaving problem of `&mut` construction — every mutation
  invalidating every handle — never arises.
- **Provisional refs** (`Ref<Node> ≈ Existing(NodeId) | Pending(u32)`)
  name entities inside the change set: new ones by submission order, old
  ones by id. They are opaque tokens, not handles — no read traits. The
  `Existing` half shares identity with old-store handles through the
  stable id, and under `edit()` the transaction can resolve one to a
  handle on request (`into_edit()` offers no such conversion — its
  discipline is that no handle exists during the pass); only `Pending`
  refs lack identity, and only until flush.
- **Ref locality is a runtime guarantee, deliberately.** Every ref is
  stamped with a token the transaction mints at `edit()`/`into_edit()`.
  Staging ops check the stamp eagerly for `Pending` refs: a ref minted by
  another transaction, or left over from before a flush, is a programming
  error caught at the staging call. `Existing` halves carry old-store ids
  and are validated at flush like any other id. Compile-time branding was
  considered and rejected: the only sound version (generativity) requires
  a callback API and makes refs un-storable outside the closure, which
  would destroy the change set's status as a first-class, self-contained
  value — the foundation for future directions (inspection,
  serialization, possibly merge) even though merging stays out of scope
  here. Staging never touches a store: the change set is a queue, not a
  private clone, so a staged `remove` cannot invalidate refs already
  handed out — everything is applied at once, at flush.
- **Handles convert to refs with a store-identity check.** Anywhere an
  `Existing` ref is expected, an old-store handle or raw id is accepted;
  the conversion asserts (a hard `assert!`, not debug-only) the handle's
  store pointer against the transaction's store. Lifetimes alone do not
  enforce this (a handle into an unrelated longer-lived store coerces),
  and a foreign handle's id would silently resolve against the wrong
  arena — silent mis-resolution is a worse failure than the check's cost.
- **Flush** is a fixed seven-step pipeline, not a raw replay of the queue.
  The change set mixes constructive ops (which replay through hooks) with
  removals (which are set-theoretic — rebuild-without — and never replay),
  so ordering must be stated:
  1. **Resolve removals**: compute the kept set per arena; validate every
     `Existing` ref (unknown old id → `FlushError`).
  2. **Dangler detection** over kept records *and* staged constructive
     ops, in old-id space — this precedes compaction so the report names
     ids the user actually staged with. Violations are listed by
     change-set op index; op indices count *every* queued op, including
     `add_<entity>`. A violation on a kept record has no op of its own,
     so it is attributed to the removal op that caused it, and violations
     distinguish `Kept(old id)` from `Staged(op index)` elements.
     Detection honors staged corrections: a kept record dangles only if
     its *effective* endpoint — after the whole change set applies,
     last `set_` wins by submission order — is removed. A staged
     `set_` re-pointing the endpoint away from a removed node cures
     that side (a `set_` cannot cure the far side — it never moves the
     other endpoint); a staged removal of the record cures both. A
     `set_` naming a removed node as its new endpoint is itself the
     violation, attributed to the `set_` op (the op that named the
     removed node), while an endpoint never re-pointed keeps the
     removal-op attribution. Every read of an old id in this pass is
     range-guarded
     before use — an out-of-range `Existing` ref is the `unknown old
     id` error, never an index panic (spike 7's crate gets this wrong:
     step 1 flags the id but does not return before step 2 indexes it —
     repro /tmp/spike7-oob-repro — so the guard is stated here for
     codegen to inherit).
     (Duplicate keys never reach this pipeline: a keyed insert onto a
     live key is rejected at the staging call — checked against the
     borrowed old store and the already-staged inserts, a staging panic
     like a foreign ref — so uniqueness holds by construction at flush;
     M2's keyed section. "Already-staged inserts" includes the keys an
     already-staged *inducing* constructor will insert, not only
     explicitly staged insert ops — a standalone add keyed after a
     staged `add_subedge` in the same change set is a live-key case
     even though no insert op was staged; probe-verified,
     /tmp/m4-probe-induct-flush.)
  3. **Compact** kept arenas, assign new ids, build the per-arena old→new
     maps. Each map is an old-id-indexed array of `Option<new id>`, one
     entry per old arena slot, `None` for removed records; new records
     have no old id and no entry.
  4. **Replay constructive ops** through the same backing hooks
     construction uses, translating `Existing` refs through the maps.
     The kept skeleton is copied first, with a fixed sub-order: kept
     records (a record before anything that references it), then each
     backing's kept membership carried in kept iteration order — fed
     into build-phase accumulators for phased backings (probe-verified:
     the carry reads only the old store's *frozen* state and writes
     only through the hooks, so replay never reads build-phase CSR
     state, and the pipeline presupposes a frozen old store;
     /tmp/m4-probe-phased-flush) — and intrusive lists rebuilt by
     walking old chains (order contracts, below). Copying kept records
     is not purely mechanical when correction ops exist: a kept record
     whose determining field is re-pointed by a staged `set_` is born
     into the new store *unlinked* under its effective endpoint — the
     walk-rebuild skips its removed old host's chain — and the staged
     `set_` then links it at replay (the delete hook's absence answer
     is legitimate; Interactions below). A `Pending` effective
     endpoint is predicted from kept counts plus preceding adds and
     *asserted* at replay, never trusted — the count covers every
     record a preceding op appends, including any its constructor
     induces (Induction below); in the stdlib patterns the
     `set_`-targetable arenas and the induced arenas are disjoint, so
     the formula degenerates to kept counts plus preceding adds. Staged ops then
     replay against that rebuilt state, in submission order — a `set_`
     un-links and re-links against the *rebuilt* lists, never the old
     ones. Replay order is observable: a re-pointed membership takes
     its replay-time position (push-front for an intrusive list), so
     the direct-construction equivalent of a flush replays the same
     verbs in the same order — a "born final" build does not match,
     and that is not a flush bug (probe-verified,
     /tmp/m4-probe-set-unlink). With nested stores, root-level ops replay before nested
     ops, submission order preserved within each level, so provisional
     paths resolve top-down (probe-verified,
     /tmp/m4-probe-nested-pending). In the flat case the grouping is
     vacuous.
  5. **Induction** runs during replay, because replay goes through the
     same constructors as direct construction.
  6. **Backing finalization.** Backings with a build phase (CSR-style
     range adjacency, M3) compact here — accumulate structures are
     frozen into their read form, per store, once, after all replay —
     *per store* is the one root store, nested stores being
     `Vec<Data>` fields on host records, never nested `Store`
     structs, and the single freeze runs after every level's replay;
     several phased backings on one store compact in the `implement!`
     block's binding order at that single freeze (M3's pin, inherited —
     "CSR and phase representation"). Read access to such backings is
     only valid post-finalization; the
     M3 constraint annotations (`requires_compaction_at_boundary`,
     `freeze_scope: per_store`, `read_precondition: post_compaction`)
     state this per backing. Steps 6–7 are `finish()` in M3's terms —
     freeze, then validate, in that order.
  7. **`validate()`**: key existence, value-in-arena, value-host
     agreement, and each backing's
     structural checks — M2's canonical inventory (premises, above; key
     uniqueness is enforced at staging, not here).
  Order contracts: kept elements keep their relative iteration order; new
  arena records append in replay order — both read *per arena*: no arena
  receives records from two levels, so the level grouping of step 4
  never reorders anything observable, and an arena's new records follow
  that level's submission order. Iteration order itself is a
  per-backing contract (M3 annotation: intrusive_list iterates
  reverse-insertion, CSR forward-insertion); "kept order" means kept
  relative order under the backing's own rule, so generic algorithms may
  not assume an order beyond the annotation. Intrusive link fields of
  kept records are never replayed — but they are also never *per-field
  remapped*: remapping each link independently silently truncates a list
  when a middle link's target was removed (the map yields `None`, read
  as end-of-list). Instead each kept list is **rebuilt by walking the
  old chain**, skipping removed records, remapping the kept ids, and
  preserving kept relative order — which degenerates to copy-and-remap
  when nothing was removed. Genuinely new inserts push-front onto the
  rebuilt heads, so new members iterate first and the reverse-insertion
  contract holds for them. `validate()` is necessary, not sufficient: a
  semantically wrong carry — a rebuilt chain truncated by a bad walk —
  passes every structural check (links stay in range), so the carry
  rules above are load-bearing, not backstopped (demonstrated in
  /tmp/m4-probe-phased-flush).
- **Flush consumes the transaction, and errors are pre-destruction.**
  `flush(self) -> Result<FlushOutcome, FlushError>` (spike 7's shape —
  the store plus the id-remap report, Removal below): every
  violation in the surface below is detected in steps 1–2, before any
  destructive work, so a failed flush has touched nothing. Recovery
  differs by entry point, deliberately. Under `edit()` the old store
  was only borrowed and is untouched — open a new transaction and
  re-stage, guided by the error's op indices. Under `into_edit()` the
  consumed store is forfeited — that is the price of the committed
  mode — unless the change set was validated first: the pipeline's
  steps 1–2 are factored as a pure predicate,
  `tx.check(&self) -> Result<(), FlushError>` (the same
  check-then-write factoring M2 applies to the keyed insert hook), so
  a change set can be verified before `into_edit()`'s flush commits.
  A `validate()` failure at step 7 after a green `check()` indicates a
  codegen bug, but it surfaces as a defensive `Err` arm
  (`ValidateFailed`), not a panic — unreachable in correct generated
  code, and strictly easier to debug than a panic when it isn't (all
  four flush probes carry the arm). Apart from that arm, the error
  surface is exactly the steps 1–2 violations.
- **Flush errors are runtime data**, not compile errors: a `FlushError`
  lists each violation (dangling endpoint) by change-set
  op index. No spans; the op index is the debugging handle. The full
  surface: dangling endpoint (named in old ids), unknown
  old id, `set_` or `remove` of a removed element, double removal, and
  ops staged under a nested path whose record was removed (the change
  set must be coherent; a silent discard would hide staging bugs). The
  defensive step-7 `ValidateFailed` arm (above) rides alongside the
  surface, unreachable in correct code. The
  removed-path violation names both sides — the staged ops' own
  indices, ascending, and the removal op's index — and a `remove`
  staged under a doomed path counts among them ("remove of a removed
  element" in nested clothing). Staged keyed inserts get their own
  step-2 coherence checks in old-id space, attributed to the insert
  op: a target removed by the change set is a dangling endpoint, and
  a host/target pair that will not agree post-flush is rejected
  before compaction. Violation lists are deterministically
  ordered (step order, then op index; never `HashMap` iteration order
  — anything derived from a carried map is collected and sorted before
  formatting), so error text is stable across runs.
  Distinct from these are **staging-time programming errors, which
  panic at the staging call** rather than joining `FlushError`: a
  foreign or stale ref stamp, removal of a `Pending` ref, `set_` of a
  `Pending` ref (a staged record's determining fields are fixed by its
  own `add` op), and a keyed
  insert onto a live key (checked against the borrowed old store and
  the already-staged inserts — including keys an already-staged
  inducing constructor will insert, /tmp/m4-probe-induct-flush;
  M2's builder does the same at its insert,
  and a silent `HashMap` overwrite there is exactly the failure the
  check exists to prevent — a staged removal does not free a key, so
  re-pointing one is two flushes; the check is probe-verified
  implementable, /tmp/m2-probe-staged-key). The dividing line: panics
  are for ops whose acceptance would silently corrupt or mis-wire the
  change set — each is deterministically detectable at the staging
  call, and each, if missed, is silently wrong; `FlushError` is for
  individually legal ops that are invalid against the store or the
  change set as a whole.

## Write traits mirror read traits

`implement!` gains a write side: per arrow instantiation, a marker-generic
trait implemented by the store's transaction type. One `Tx` per
`implement!` call: the transaction names the store type, entity ids, and
hooks, so only `implement!` codegen can emit it — on a phased store the
same one `Tx` borrows the frozen old store and rebuilds through
`GraphStore<Build>`, on a nested store it serves every level through
`within()` (below).

**Write-trait identity mirrors read-trait identity (M1), with one named
exception.** Arrow-named write traits are root-sourced like read traits:
the root pattern that declares an arrow owns its write traits as it owns
its read trait and emits the write trait items where it emits the read
trait, and the registration courier's metadata gains a write slot — per
arrow-named write trait, its (trait name, method name, ref associated-type
names) — alongside the read tuple, so `implement!` resolves write traits
the same way it resolves read traits. Mechanically the slot is a sixth,
braced element on M1's metadata tuple — the same kind of extension M2's
key slot was; the courier rule and channel are unchanged. The exception
is the existence ops: `Add`/`Remove` traits name an *entity*, and no
arrow root can name a pattern's entities, so they are owned by the
pattern block that declares the entity. The declaring block emits
`Add<Entity>`/`Remove<Entity>` trait items per non-root entity —
unconditionally, whether or not an entity arrow targets it (constructors
are emitted per entity unconditionally, and a generic transform needs the
same reach: Digraph's Edge has no entity arrow, yet spike 8's
`replace_node` bounds on `RemoveEdge`) — plus one per-entity marker in
the same markers module, minted and re-minted per composition instance
exactly like the per-arrow markers, so two instances of one pattern give
two `AddNode` impls on one `Tx` under two markers, never a conflict.
Default names derive mechanically from the declared thing the op writes:
**existence ops name after the entity** — `AddNode<M>`, `RemoveNode<M>`,
method `add_node()` — while **link and payload ops name after the
arrow** — `InsertOutEdges<M>`, `SetSource<M>`, methods
`insert_out_edges()` / `set_source()`. One trait per verb, so a copy
transform bounds on `AddNode + AddEdge` without dragging in `Remove`.
Method names are root-fixed like read-side methods; instantiation-local
spellings (`add_subnode`) are inherent methods on the transaction,
exactly as a distinct read spelling would be (M1's inherent-alias rule).
No override clause exists in v1 — defaults cannot collide (arrows are
unique per pattern; traits live under the pattern's marker module).
**Canonical arity.** Every write-trait method takes the self ref —
`insert_<arrow>(Ref<A>, …)`, `set_<arrow>(Ref<A>, Ref<B>)` — because
the trait item is emitted by the root, which cannot know placement
(root-marking does not propagate through instantiation; placement is a
composition fact). Where the computed placement leaves no self id (an
unfused-root self), the canonical form is unemittable and `implement!`
emits no trait impl for that verb: the op exists only as the
transaction's inherent method in its placement-pinned self-less form —
M3's pin that unfused-root ops are store-level methods, probe-pinned —
so generic transforms see exactly the canonical forms. `backing!`
contributes the hook mechanics each verb lowers into, never names;
`compose_pattern!` re-mints markers (per-arrow and per-entity alike)
and prefixes inherent aliases, never traits — both forced by epic
invariant 2 (trait identity comes from the declaring pattern, not the
store).

The write-side column names trait methods in their canonical arity;
the placement-variant forms are inherent methods on the transaction
(the canonical-arity rule above), never trait impls.

| arrow shape | read side | write side (on the transaction) |
|---|---|---|
| entity | root accessor, handles | `Add<Entity>`/`Remove<Entity>` (declaring-block-owned traits, per-entity marker; local spelling `add_<entity>(<total one-arrow args>) -> Ref<E>`, `remove_<entity>(Ref<E>)`) |
| total `one` | `e.target()` | constructor argument of `add_<entity>`; and `Set<Arrow>` — `set_<arrow>(Ref<A>, Ref<B>)`, re-pointing in place (the `by`-derived rows depend on this) |
| optional `one` | `e.payload() -> Option<_>` | `Set<Arrow>` — `set_<arrow>(Ref<A>, Ref<B>)` |
| `many`, primitive | `p.tags()` | `Insert<Arrow>` — `insert_<arrow>(Ref<A>, Ref<B>)`, membership asserted directly; at an unfused-root self the trait impl is not emitted and the op is inherent-only and self-less: `insert_<arrow>(Ref<B>)` (no self id exists — M2's general rule) |
| `many`, existence-derived | `g.nodes()` | none beyond `Add<Entity>`/`Remove<Entity>` — membership is record existence |
| `many`, derived (`by`) | `n.out_edges()` | none — membership follows the determining `one` arrow's constructor argument or `set_`; no insert op is generated, so membership cannot contradict the field |
| keyed partial | `sg.subnode(mn)` | `Insert<Arrow>` — canonical trait form `insert_<arrow>(Ref<A>, Ref<K>, Ref<B>)` — for a colocated-root self `Ref<A>` is the host class's ref (spike 11's shape), for a non-root self the entity's own — function semantics; at an unfused-root self the trait impl is not emitted and the op is inherent-only: `insert_<arrow>(Ref<K>, Ref<B>)` (one self, no id — the edge-induced composition, M2); a live key is rejected at the staging call, like a foreign ref |

Derived read arrows (`:= sum`) have no write side, exactly as they have
no backing. A `by`-derived `many` arrow's hooks fire when the determining
field is replayed (constructor or `set_` op), so replay order needs no
special casing for membership. Primitive insertions into a derived
relation are impossible by construction — the method does not exist —
which is what the schema's `by` declaration buys (M1 syntax section).

Markers are the same per-instance markers as the read side — plus the
per-entity markers the existence traits carry — so a
generic algorithm can bound on `OutEdges<M>` and `InsertOutEdges<M>`
together — e.g. a graph-transform pass that reads one store and stages
another without knowing either pattern. This has been spike-verified: a
generic copy and a generic node-replacement transform were compiled and
run against a hand-written transaction at two marker instantiations.
What the verification pinned down:

- Write traits carry **associated ref types** (`type NodeRef;`) — a
  generic algorithm cannot name the concrete `Ref<Node>`. For a pure
  copy into an empty destination this is the *only* addition needed:
  `AddNode<M> + InsertOutEdges<M>` with an equality bound
  `NodeRef = AddNode::NodeRef`, and refs themselves need only `Copy`.
  (This is the spike's hand-written surface; the generated surface
  recasts the copy's bounds — see the node-creation paragraph below.)
- A **handle→ref mapping** trait (the transaction converts old-store
  handles and ids to `Existing` refs) enters exactly when the transform
  *names old-store elements* — splicing, rewiring, delta edits — bringing
  the full equality web (`Handle = Src::Node`, `NodeRef = ...`). The
  plain associated-type equality bounds suffice; no GAT is needed on the
  write side. Its home is pinned: the runtime crate owns a single
  marker-generic `HandleToRef<M>` with plain associated types `Handle`
  and `Ref`, and `implement!` emits one impl per entity on the
  transaction, marked by that entity's per-entity marker — an
  algorithm-crate declaration (spike 8's shape) would not unify across
  crates in a bound web. Flush is deliberately *not* marker-generic.
- **Handle keyability is a load-bearing bound.** Generic transforms
  correlate nodes across iteration views and edge-endpoint views through
  a handle-keyed map, so `Copy + Eq + Hash` on handles is part of the
  minimal bound set — handle identity (store pointer + index path) is
  doing real work, not just pleasing the test suite.
- **Signature-faithful copy is not order-agnostic.** Re-inserting edges
  in iteration order under a reverse-insertion backing (intrusive_list)
  reverses every list; the transform must collect-and-reverse, which is
  correct only given the backing's iteration-order annotation (M3). A
  forward-insertion backing (CSR) copies verbatim. Generic algorithms
  must be written against the annotation, never an assumed order.
- Two transform shapes exist and must not be mixed: *copy-into-empty*
  (destination starts blank; insert everything read) and
  *rewrite-deltas* (destination's old store already contains the graph;
  kept-kept edges are staged as nothing). "Insert everything you read"
  against a non-empty destination double-inserts.

Node creation is *not* generic across patterns: the constructor argument
list is the entity's total `one` arrows, which differ per schema. A
generic transform therefore bounds on a per-pattern node-constructor
trait plus per-arrow ops, and covers primitive arrows only — copying a
`by`-derived arrow's membership happens implicitly through the
determining fields. For a link-record entity (Digraph's Edge) the
per-pattern constructor trait is the whole insert path:
`AddEdge::add_edge(source, target)` fires the `by`-derived membership
hooks from the constructor, so a generic copy bounds on
`AddNode + AddEdge` and no `InsertOutEdges` at all. Spike 8's
hand-written transaction modeled `out_edges` as primitive with a
create-shaped `insert_out_edge(source, target) -> EdgeRef`; the
generated surface splits that compound — membership-assert
`Insert<Arrow>` exists for genuinely primitive `many` arrows (a
`tags: Post -> Tag`), and record creation is the per-entity `Add`
trait's job.

## Nested stores

A "nested store" is the set of arenas the placement rule (M2) embeds in
a host record — `Vec<Data>` fields on the host class's record struct,
never a nested `Store` struct; handles keep one root-store reference
throughout. A change-set entry is an `(index path, operation)` pair —
the same addressing scheme handles already use. The transaction API
exposes scoped sub-transactions: `tx.within(node)` stages operations
against the subgraph store embedded in that (possibly provisional)
node's record. Since the change set is already path-addressed, `within`
records a path prefix and ops are queued as `(path, op)` pairs — no
borrow is held, so root and nested staging interleave freely.

**Path segments are refs, never resolved ids** (probe-verified,
/tmp/m4-probe-nested-pending): each segment is
`Existing(old id) | Pending(op index)`, because a provisional host has
no id yet — it *is* its submission position. Flush resolves segments
top-down (pipeline step 4): `Existing` through the per-level compaction
maps — host ids shift under removal, and this translation is
load-bearing — and `Pending` through the root pass's op-index replay
table; a table miss is an internal bug and panics. One op queue serves
all levels, and op indices count every op, root and nested alike, so
staging under a provisional host interleaves with root staging freely.
Because "no borrow is held" cannot mean a borrowed sub-transaction in
safe Rust without giving up the change set's status as a plain struct,
`within` returns an owned scope value (token + path prefix) and nested
staging calls thread `&mut Tx` explicitly, the token re-checked per
call. `within()` scopes the host-relative ops of the embedded store;
ops whose signature already names the host (the colocated-root keyed
form) stage at root level and do not combine with it. For id-free
embedded shapes `validate()` is vacuous — there are no stored ids to
check — and the real structural check is path coherence, in flush
step 2, before compaction so errors name old ids; codegen emits no
per-backing checks for backings with nothing stored to check.

## Induction

The edge-induced subgraph requires that adding a subedge induces subnodes
for its main edge's endpoints. Induction is constructor logic, not schema
vocabulary: the builder's `add_subedge(me)` — the edge-induced
composition's self is the single subgraph at an unfused root (M2's
Tests), so there is no `sg` to pass — reads `me`'s endpoints and
ensures a subnode for each, deduplicated through the `subnode` hash index.
This holds by construction rather than by check, because constructors are
the single write path (M2 builder shape): every subedge in existence came
through `add_subedge`, whether during initial build, during loading from
data (below), or during change-set replay — flush replays staged ops
through the same constructors, so replay behavior is identical to direct
construction.

The induction rule is *pattern semantics*, and it lives with the pattern
as ordinary Rust — no more a special extension than any capability
implementation (`in_edges` knowing its link fields, `add_edge` knowing to
push-front). The toolkit never invents logic; the macro splices schema
facts into code written once, and the inducing constructor is code
written once, composing generated primitives (`add_sub_node`, the keyed
lookup) and the builder's id-based read view (`main_edge_source(me)` and
`main_edge_target(me)`, the view's contract spellings — M2's Builder
shape; handles cannot exist against the `&mut` builder, which is why
the view exists).

It is *undeclared* — the schema says nothing about it — but that is the
normal case, not a gap: a declaration exists so the macro has something
to splice, and there is nothing to splice here (no marker, no backing
fields, no trait impl, no validation rule). Compare `sum`: declared
precisely because markers forced the macro to generate the impl. The
inducing constructor is the write-side analogue of the derived read
arrows, minus the declaration. If M5+ introspection (serializers,
fuzzers) ever needs the rule stated, a declaration can be added then —
constructor behavior is the contract either way.

One API consequence: undeclared constructor code still needs to *read* —
it follows `main_edge` to `source`/`target` — but during construction the
builder owns the store mutably, and pattern code can't touch generated
private fields. The builder therefore exposes a **read view of the store
under construction as generated id-based getters** (field reads by id,
returning ids and copies — never handles; specified in M2's builder
shape, which needs it for the same inducing constructor). Handles are
rejected during construction — there is one store, read and mutated at
once, and the borrow checker forbids exactly the coexistence induction
needs; a read-only handle over the *old* store becomes viable under
`edit()`/flush, where the old store is only ever read; id getters work
everywhere. The same view serves flush replay, which reads copied old
data plus already-replayed ops.

Note the edge-induced form is the special case: a general subgraph may
also carry standalone nodes (`add_sub_node` called directly, never
induced by an edge). Constructor-level induction supports both; a
flush-time derivation pass would have had to invent a way to permit the
standalone
case.

## Construction from data (loading)

Loading a metagraph from external data is not a separate path: a loader
drives the builder. Referential integrity is enforced by construction —
constructor arguments are typed ids, so a dangling endpoint is
inexpressible rather than rejected — and by the flush validation boundary
(key existence, value-in-arena, value-host agreement) for what types can't see. Data formats and
their codegen are M5+ introspection territory; all M4 needs is that the
loader and the hand builder share one funnel.

## Removal

Removal is a first-class staged operation (`remove_<entity>(ref)`), and it
is the reason the change set is a replay queue rather than a private clone
mutated during staging: a queued removal cannot invalidate provisional
refs already handed out, because nothing is applied until flush.

- Flush rebuilds without the removed elements and compacts; ids may
  shift. The flush report carries per-arena old→new id maps for users who
  retained raw ids (`NodeId(pub u32)`): each map is an old-id-indexed
  array of `Option<new id>` with one entry per old arena slot (`None`
  for removed records; new records have no old id and no entry). Handles
  never cross stores, so this is type-safe; the maps exist for raw ids
  only.
- The pass discipline makes invalidation explicit: under `into_edit`,
  the old store is consumed and the borrow checker guarantees no handle
  survives the pass boundary. (This rests on handles holding real `&`
  references; the guarantee is NLL liveness, not scope, and any future
  raw-pointer handle redesign would lose it.)
- **Cascade policy is minimal.** A removal that would leave dangling
  references (removing a node with incident edges still present — whether
  kept from the old store or staged in this change set) is a `FlushError`
  listing the danglers — the change set must remove dependents
  explicitly. One exception: a nested store embedded in a removed record
  goes with the record, contents and all. Keyed maps follow the same
  field-vs-membership split, probe-verified
  (/tmp/m4-probe-keyed-carry): an entry's *key* behaves as a
  reference — removing a key record with live entries is a `FlushError`
  naming them (sorted by (host, key), attributed to the removal op);
  remove the dependent target explicitly — while its
  *target* behaves as membership — removing a target record clears the
  entries pointing at it, as list membership follows a removed record.
  *Live* means surviving the change set, symmetrically in both
  directions: entries under a host removed in the same change set are
  cascade-covered (their keys do not keep the key record alive — there
  is no per-entry removal op to cure them with), while a *staged
  insert* keyed by the removed key record counts as live (the change
  set names the key from a new place) and forces the error.
  Flush accordingly carries keyed maps as state, never rebuilding them
  (a keyed arrow need not have a companion field to recompute from):
  entries copy with kept records and remap — keys through the key
  arena's old→new map, and path-id targets through the *composition*
  of host and per-host sub maps (no single map translates a path id);
  the composition applies at two sites, carry and staged-insert
  replay. Two staging-time pins ride with this: a keyed insert's
  host and target must be `Existing` (a `Pending` host can never hold
  a path-id target that would agree with it — the mismatch is
  unrepresentable, so the staging call panics, symmetric with
  removal-of-`Pending`), and the duplicate-key check has two arms —
  existing entry or already-staged insert under the same host. Re-
  keying is deliberately two flushes (remove the target, then insert
  the new entry); the single-flush equivalent panics at staging.
- Tombstones are rejected: they would complicate every backing contract
  (intrusive lists, size hints, CSR ranges) for a benefit
  rebuild-without already provides.

## Explicit non-goals

- No in-place mutation of a live store, ever (the `into_edit` flush
  overwrites a store it consumed, which is dead, not live). No
  generational ids.
- No handle migration across a flush. Handles name (store, path); a new
  store is a new identity. The id-remap report exists for raw ids, not
  handles.
- No per-membership un-insert in v1: primitive-`many` memberships and
  keyed entries are removed only by removing the entity that carries
  them, or by re-pointing a `one` arrow via `set_`. Un-linking a single
  membership without removing its record is a deliberate deferral, not
  an oversight.
- No concurrency or merging: two transactions on one store are
  independent forks; merging change sets is out of scope. (The ref stamp
  makes cross-fork contamination a staging error rather than a silent
  mis-wiring.)

## Interactions with other milestones

- **Backings (M1–M3)**: the backings are unchanged, but the hook
  interface grows what M2's round-3 caveat predicted: `set_` requires
  per-backing *delete* hooks (un-linking a `by`-derived intrusive
  membership before re-linking). The field-write half is M2's
  deliverable (M2's builder shape moves field writes behind named
  hooks, landed as the `__set_<arrow>` methods; M3's field
  declaration notes "set_-time re-pointing is M4
  vocabulary") — M4's growth beyond it is the delete verb and the
  re-point orchestration. The growth is per backing kind and is
  probe-verified (/tmp/m4-probe-set-unlink): a `by`-derived intrusive
  list gains `__unlink_<arrow>` — the delete verb in the landed hook
  convention, a doc-hidden store method beside `__insert_<arrow>`,
  its body authored in the `on_unlink(a, b)` clause of M3's clause
  inventory
  (the probe's `unlink_<list>_hook` free-function spelling was
  hand-emission, like spike 11's) — walk-and-splice, O(list length),
  correct at head and tail (a `by` on both endpoints of a mutable
  field grows two), and it is *total*: it returns whether the record
  was found, never panics on absence, because flush can legitimately
  hold a kept record that is currently in no list (the
  correction-op case, step 4). The re-point hook (`__set_<arrow>`
  — M2's landed field-write hook, grown with the
  orchestration) takes only `(record, new value)`: the old host is
  read from the record's own determining field — that is exactly the
  `by`-derivation invariant — then un-link, write, re-link. The
  orchestration never threads an index path: `set_`-able arrows are
  root-level-only under M3's contributed-field-location restriction
  (embedded-class records carry no backing contributions), so the
  record and its lists are always flat-addressed. A phased
  backing needs nothing (membership is derived at freeze); a
  hash_index needs entry removal only when un-insert lands (non-goal,
  v1). Hooks remain the single write path: flush
  replay lowers `set_` through the same grown hooks that the
  builder's `set_<one-arrow>` method uses, replaying after the
  kept-skeleton rebuild (pipeline step 4). Backings that require a
  build phase (CSR) are *satisfied* by flush, which is always a full
  rebuild; pipeline step 6 (backing finalization) is where their
  compaction runs, and the M3 constraint annotations
  (`append_only_build`, `freeze_scope: per_store`,
  `read_precondition: post_compaction`)
  document the requirement.
- **Payloads (M5+)**: `set_<one-arrow>` is the payload update path.
- **Cost model**: flush is a deep copy of the entire store tree plus
  replay. Right for the target graph sizes; the report struct is where
  incremental reuse (persistent structures, chunked arenas) would attach
  if it ever becomes necessary.

## Implementation checklist

Ordered by dependency; each step names its acceptance instrument. The
flat non-phased core is spike-verified (spike 7, 32 tests), and every
later mechanism is probe-verified — each step's acceptance is the
corresponding probe's tests reproduced against *generated* code, with
the probe crate under `/tmp` as the reference implementation.

**Stage 1 — flat, non-phased core (spike 7's territory)**

- [x] 1. Change-set data model: `Tx` over `&Store` (`edit()`),
      token-stamped refs (`Existing | Pending`, `Copy`), one flat op
      queue whose indices count every op, staging methods with local
      spellings. Acceptance: refs expose no read methods (compile-fail
      fixture); the change set is a plain struct.
- [x] 2. Staging panics: foreign/stale stamp, removal of a `Pending`
      ref, `set_` of a `Pending` ref, keyed insert onto a live key
      (two arms: existing entry or staged insert, same host).
      Acceptance: one `should_panic` test each.
- [x] 3. Flush against the M1 builder shape (steps 1–5+7; step 6
      vacuous): range-guarded step 2 over effective endpoints, honor
      semantics, attribution (removal op vs naming op), compact +
      per-arena maps, skeleton sub-order with rebuild-by-walk,
      submission-order replay. Acceptance: spike 7's 32 tests
      reproduced, including Königsberg signature equality, plus the
      out-of-range `Existing` ref returning the listed error, not a
      panic (the spike-7 bug, /tmp/spike7-oob-repro).
- [x] 4. `tx.check()` — steps 1–2 as a pure predicate — plus consuming
      `flush(self)` and the deterministic error surface (collect,
      sort, format). Acceptance: byte-identical `Display` across runs
      (probe 4's determinism test); the committed-mode contract
      exercised (`check()` green → `into_edit` flush commits).
- [x] 5. `into_edit()` allocation reuse, flat non-phased:
      derive-before-write, forward writes. Acceptance: spike 7's
      `into_edit` tests.

**Stage 2 — generated write surface**

- [ ] 6. Write-trait codegen: courier write slot (the arrow-named
      verbs, root-emitted trait items); per-entity `Add<Entity>`/
      `Remove<Entity>` traits and per-entity markers emitted by the
      declaring block; canonical arity (the self ref always present),
      placement-variant forms inherent-only; the runtime-crate
      `HandleToRef<M>` mapping trait with per-entity impls; inherent
      local spellings on the transaction. Acceptance: spike 8's copy
      and node-replacement scenarios recast against the generated verb
      surface — the copy bounds on `AddNode + AddEdge` (membership
      follows the constructor's hooks), the equality web spelled as in
      spike 8's write.rs/map.rs (the de facto name contract), same
      assertions, at two marker instantiations, against generated
      code.
- [ ] 7. `set_` + delete hooks: `__unlink_<arrow>` (total; the
      `on_unlink(a, b)` clause of M3's clause inventory —
      intrusive_list authors the body) and the
      re-point orchestration emitted per binding; builder
      `set_<one-arrow>` for all `one` arrows (field-write hooks are
      M2's deliverable, not M4's). Acceptance: probe 3's tests as
      generated equivalents (head/middle/tail unlink, honor case,
      double `set_` last-wins, same-verbs-in-order normalization).
- [ ] 8. Unfused-root forms: self-less `insert_<arrow>(Ref<B>)`
      (primitive and keyed), store-level ops. Acceptance: the
      root-intrusive (/tmp/m3-probe-r3-root-intrusive) and
      edge-induced (/tmp/m2-probe-edge-induced) probes' tests as
      generated equivalents.

**Stage 3 — nesting, induction, keyed, phased**

- [ ] 9. Nested stores: `within()` owned scope threading `&mut Tx`,
      ref-segment paths, per-level maps + op-index replay table,
      root-before-nested grouping, `RemovedPathHost`. Acceptance:
      probe 2's tests (one-flush hierarchy edit, host-id translation,
      per-arena ordering). The replay-table lookup is emitted
      bounds-checked (`get`, internal-bug message): probe 2's miss
      test forged a stamped ref from same-crate tests, which a
      downstream `tests/` crate cannot do, so the miss path is
      exercised by a same-crate unit test or carried as a
      codegen-review invariant — this step's notes say which.
- [ ] 10. Induction: the inducing `add_subedge` constructor over
      generated primitives + the id-based read view, running during
      replay through the same constructors. The induction×flush-replay
      probe (/tmp/m4-probe-induct-flush, this review round) must land
      green before this step starts; it also exercises step 8's
      self-less keyed forms and step 11's carried keyed map.
      Acceptance: the edge-induced probe's tests plus that probe's,
      as generated equivalents.
- [ ] 11. Removal + keyed carry: id-remap report (old ids only),
      survivors-only cascade, composed remap at carry and
      staged-insert replay, `Existing`-only keyed host/target.
      Builds on step 9's per-level maps — the composed remap
      translates path-id targets through host map ∘ per-host sub
      maps. Acceptance: probe 4's 15 tests plus spike 7's 14 removal
      tests as generated equivalents.
- [ ] 12. Phased stores: flush into `GraphStore<Build>`,
      kept-fed-then-replay carry (old *frozen* reads only),
      `finish()` = steps 6–7. Acceptance: probe 1's 12 tests as
      generated equivalents.

**Stage 4 — hardening**

- [ ] 13. Negative suite: trybuild fixtures (read-on-ref, wrong-marker
      bounds, handle survival under `into_edit`), the `should_panic`
      staging tests collected, a downstream-crate transform
      portability test (M1 step 9's pattern).
- [ ] 14. Doc pass: crate-root docs for transaction/flush/pass
      discipline, macro docs updated (M1 step 11's pattern).

Dependencies: 1 → 2 → 3 → (4, 5); 6 → (7, 8); 3 → (9, 12); 9 → 11;
(8, 11) → 10. Stage 2 is independent of Stage 1's steps 4–5.

## Risks

- **Provisional refs are a second handle-like concept.** Users will try
  to call read methods on them. Naming (`Ref` vs `Handle`) and docs carry
  the weight; the type system already refuses. Locality violations are
  caught at staging, but the error must name which transaction the ref
  came from (or when it died) to be useful.
- **Flush error quality.** Errors are runtime data and point at op
  indices ("op 14: edge endpoint not in store"); the index alone is a
  weak debugging handle. If this proves painful, ops can record the
  staging call site's `Location` via `#[track_caller]`, so the error
  names the line that queued the offending op.
- **Full-rebuild cost hiding in benchmarks.** Fine at Königsberg scale;
  measure once hierarchy depth grows, not before.
