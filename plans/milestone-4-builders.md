# Milestone 4: write-side builders — mutation by rebuild

**Goal**: change an existing graph without ever mutating a live store. The
write side is generated from the schema the same way the read side is: one
interface per arrow, marker-generic, so generic graph-transformation
algorithms are as portable as generic traversals.

Specification level: preliminary, with one exception: the flat-`Digraph`
case (change-set queue, tx-stamped refs, the flush pipeline, removal with
the id-remap report) has been hand-built and tested against the reference
store (32 tests, including Königsberg signature equality between
tx-built and directly-built stores). Everything specific to write traits,
nested staging, and induction remains uncompiled.

## Premises (what M1–M3 established)

- Stores are immutable once built; construction uses `&mut` methods on the
  store (epic invariant 5). Handles borrow the store and identify by store
  pointer + index path. Terminology: the store in its `&mut` construction
  phase is *the builder*; the staging type M4 adds is *the transaction*;
  flush re-runs the builder machinery.
- Hooks are the single write path: every mutation primitive (field writes,
  intrusive links, hash-index maintenance) lives in a named per-backing
  hook function over `&mut` store state, and the `add_*` constructors are
  thin orchestration (M1 commits to both names and shape).
- Validation has a named boundary (`validate()`): key existence, key
  uniqueness per self object — and, per backing, **structural checks**
  (endpoint ids in arena range, intrusive link well-formedness,
  membership consistency between an arrow's links and its determining
  fields). A store with no keyed arrows still validates; keyless
  backings contribute the structural half. M2 callers invoke it
  explicitly; flush invokes it unconditionally.
- Nested stores are addressed by index paths (M2).
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
let new_store = tx.flush()?;         // validate + rebuild; old store
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
  ordering constraint (spike-verified): all derived data — kept sets,
  old→new maps, rebuilt link chains — must be computed before any
  in-place write, and writes must go forward (write slot ≤ read slot),
  so no slot is overwritten before its old value has been read.

- **The transaction borrows the old store sharedly.** This is the
  ergonomic point of the design: you can traverse the graph (with real
  handles, real read traits) while staging its replacement. The
  interleaving problem of `&mut` construction — every mutation
  invalidating every handle — never arises.
- **Provisional refs** (`Ref<Node> ≈ Existing(NodeId) | Pending(u32)`)
  name entities inside the change set: new ones by submission order, old
  ones by id. They are opaque tokens, not handles — no read traits. The
  `Existing` half shares identity with old-store handles through the
  stable id, and the transaction (which borrows the old store) can
  resolve one to a handle on request; only `Pending` refs lack identity,
  and only until flush.
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
  3. **Compact** kept arenas, assign new ids, build the per-arena old→new
     maps. Each map is an old-id-indexed array of `Option<new id>`, one
     entry per old arena slot, `None` for removed records; new records
     have no old id and no entry.
  4. **Replay constructive ops in submission order** through the same
     backing hooks construction uses, translating `Existing` refs through
     the maps; root-level ops before nested ops at each step, so
     provisional index paths resolve top-down. (Trivially satisfied in
     the flat case — there is only the root level; the constraint becomes
     real with nested stores.)
  5. **Induction** runs during replay, because replay goes through the
     same constructors as direct construction.
  6. **Backing finalization.** Backings with a build phase (CSR-style
     range adjacency, M3) compact here — accumulate structures are
     frozen into their read form, per store, once, after all replay.
     Read access to such backings is only valid post-finalization; the
     M3 constraint annotations (`requires_compaction_at_boundary`,
     `freeze_scope: per_store`, `read_precondition: post_compaction`)
     state this per backing.
  7. **`validate()`**: key existence, key uniqueness, and each backing's
     structural checks (below).
  Order contracts: kept elements keep their relative iteration order; new
  arena records append in replay order. Iteration order itself is a
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
  contract holds for them.
- **Flush errors are runtime data**, not compile errors: a `FlushError`
  lists each violation (dangling endpoint, duplicate key) by change-set
  op index. No spans; the op index is the debugging handle. The full
  surface: dangling endpoint (named in old ids), duplicate key, unknown
  old id, `set_` or `remove` of a removed element, double removal, and
  ops staged under a nested path whose record was removed (the change
  set must be coherent; a silent discard would hide staging bugs).
  Distinct from these are **staging-time programming errors, which
  panic at the staging call** rather than joining `FlushError`: a
  foreign or stale ref stamp, and removal of a `Pending` ref. The
  dividing line: `FlushError` is for change sets that are coherent as
  staged but invalid against the store; panics are for change sets that
  could never have meant anything.

## Write traits mirror read traits

`implement!` gains a write side: per arrow instantiation, a marker-generic
trait implemented by the store's transaction type.

| arrow shape | read side | write side (on the transaction) |
|---|---|---|
| entity | root accessor, handles | `add_<entity>(<total one-arrow args>) -> Ref<E>`; `remove_<entity>(Ref<E>)` |
| total `one` | `e.target()` | constructor argument of `add_<entity>` |
| optional `one` | `e.payload() -> Option<_>` | `set_<arrow>(Ref<A>, Ref<B>)` |
| `many`, primitive | `p.tags()` | `insert_<arrow>(Ref<A>, Ref<B>)` — membership asserted directly |
| `many`, existence-derived | `g.nodes()` | none beyond `add_<entity>`/`remove_<entity>` — membership is record existence |
| `many`, derived (`by`) | `n.out_edges()` | none — membership follows the determining `one` arrow's constructor argument or `set_`; no insert op is generated, so membership cannot contradict the field |
| keyed partial | `sg.subnode(mn)` | `insert_<arrow>(Ref<A>, Ref<K>, Ref<B>)` — function semantics; duplicate key is a flush error |

Derived read arrows (`:= union`) have no write side, exactly as they have
no backing. A `by`-derived `many` arrow's hooks fire when the determining
field is replayed (constructor or `set_` op), so replay order needs no
special casing for membership. Primitive insertions into a derived
relation are impossible by construction — the method does not exist —
which is what the schema's `by` declaration buys (M1 syntax section).

Markers are the same per-instance markers as the read side, so a
generic algorithm can bound on `OutEdges<M>` and `InsertOutEdges<M>`
together — e.g. a graph-transform pass that reads one store and stages
another without knowing either pattern. Two additions make that actually
work:

- Write traits carry **associated ref types** (`type NodeRef;`) — a
  generic algorithm cannot name the concrete `Ref<Node>`.
- A **handle→ref mapping**: the transaction converts old-store handles
  and ids to `Existing` refs, exposed as a trait so generic code can
  relate the read side's handle types to the write side's ref types
  (associated-type equality bounds suffice).

Node creation is *not* generic across patterns: the constructor argument
list is the entity's total `one` arrows, which differ per schema. A
generic transform therefore bounds on a per-pattern node-constructor
trait plus per-arrow ops, and covers primitive arrows only — copying a
`by`-derived arrow's membership happens implicitly through the
determining fields.

## Nested stores

A change-set entry is an `(index path, operation)` pair — the same
addressing scheme handles already use. The transaction API exposes scoped
sub-transactions: `tx.within(node)` stages operations against the subgraph
store embedded in that (possibly provisional) node's record. Since the
change set is already path-addressed, `within` records a path prefix and
ops are queued as `(path, op)` pairs — no borrow is held, so root and
nested staging interleave freely. Flush replays root-level operations
before nested ones (pipeline step 4), so provisional paths resolve
top-down.

## Induction

The edge-induced subgraph requires that adding a subedge induces subnodes
for its main edge's endpoints. Induction is constructor logic, not schema
vocabulary: the builder's `add_subedge(sg, me)` reads `me`'s endpoints and
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
written once, composing generated primitives (`add_subnode`, the keyed
lookup) and read traits (`me.source()`, `me.target()`).

It is *undeclared* — the schema says nothing about it — but that is the
normal case, not a gap: a declaration exists so the macro has something
to splice, and there is nothing to splice here (no marker, no backing
fields, no trait impl, no validation rule). Compare `union`: declared
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
returning ids and copies — never handles). Handles are rejected here on
purpose: a whole-store handle's shared borrow cannot coexist with the
`&mut` field access that mutates a nested store (the borrow checker
forbids it exactly in the case induction needs). A restricted root handle
is viable only in the flat case, where the store being read is not the
store being mutated; id getters work in both. The same view serves flush
replay, which reads copied old data plus already-replayed ops.

Note the edge-induced form is the special case: a general subgraph may
also carry standalone nodes (`add_subnode` called directly, never induced
by an edge). Constructor-level induction supports both; a flush-time
derivation pass would have had to invent a way to permit the standalone
case.

## Construction from data (loading)

Loading a metagraph from external data is not a separate path: a loader
drives the builder. Referential integrity is enforced by construction —
constructor arguments are typed ids/handles, so a dangling endpoint is
inexpressible rather than rejected — and by the flush validation boundary
(key existence and uniqueness) for what types can't see. Data formats and
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
  goes with the record, contents and all.
- Tombstones are rejected: they would complicate every backing contract
  (intrusive lists, size hints, CSR ranges) for a benefit
  rebuild-without already provides.

## Explicit non-goals

- No in-place mutation, ever. No generational ids.
- No handle migration across a flush. Handles name (store, path); a new
  store is a new identity. The id-remap report exists for raw ids, not
  handles.
- No concurrency or merging: two transactions on one store are
  independent forks; merging change sets is out of scope. (The ref stamp
  makes cross-fork contamination a staging error rather than a silent
  mis-wiring.)

## Interactions with other milestones

- **Backings (M1–M3)**: unchanged. Hooks are the single write path, so
  every backing works under flush replay without an interface change.
  Backings that require a build phase (CSR) are *satisfied* by flush,
  which is always a full rebuild; pipeline step 6 (backing finalization)
  is where their compaction runs, and the M3 constraint annotations
  ("append-only build phase", per-store freeze, post-compaction read
  precondition) document the requirement.
- **Payloads (M5+)**: `set_<one-arrow>` is the payload update path.
- **Cost model**: flush is a deep copy of the entire store tree plus
  replay. Right for the target graph sizes; the report struct is where
  incremental reuse (persistent structures, chunked arenas) would attach
  if it ever becomes necessary.

## Task sequence (sketch)

1. Change-set data model + provisional refs for flat `Digraph`; flush =
   the six-step pipeline over a cloned store. Königsberg built via
   transaction must equal the M1 store in every observable way (same test
   assertions).
2. Write traits generated per arrow, with associated ref types; a generic
   read→write transform (nodes via a per-pattern node-constructor bound,
   edges via per-arrow ops, primitive arrows only) as the portability
   proof.
3. Flush-error surface: the full list from the core model (dangling
   endpoint, duplicate key, unknown old id, `set_`/`remove` of a removed
   element, double removal, removal of a `Pending` ref, ops under a
   removed nested path).
4. Nested stores: path-addressed ops and sub-transactions; hierarchy
   edit tests (stage a new outer node *and* its subgraph in one flush).
5. Induction for the subgraph pattern: the inducing `add_subedge`
   constructor as a builder extension over the generated primitives and
   the id-based read view.
6. Removal with compaction and the id-remap report; cascade-policy tests
   (dangling removals rejected, nested stores dropped with their record).

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
