# Milestone 4: write-side builders — mutation by rebuild

**Goal**: change an existing graph without ever mutating a live store. The
write side is generated from the schema the same way the read side is: one
interface per arrow, marker-generic, so generic graph-transformation
algorithms are as portable as generic traversals.

Specification level: preliminary. The model and API shape below are
coherent end to end, but nothing here has been compiled.

## Premises (what M1–M3 established)

- Stores are immutable once built; construction uses `&mut` methods on the
  store (epic invariant 5). Handles borrow the store and identify by store
  pointer + index path.
- Hooks are the single write path: every mutation primitive (field writes,
  intrusive links, hash-index maintenance) lives in a per-backing hook over
  `&mut` store state, and the `add_*` constructors are thin orchestration.
- Validation has a named boundary (`validate()`): key existence, key
  uniqueness per self object.
- Backings declare phase constraints (M3 annotations); CSR-style backings
  already assume a build/flush boundary.
- Nested stores are addressed by index paths (M2).

M4 builds nothing new for storage. It adds a *staging* type in front of
the existing construction machinery and a flush that replays staged
changes through it.

## Core model

```rust
let mut tx = store.edit();           // borrows &Store; reads and old
                                     // handles keep working while staging
let a = tx.add_node();               // provisional ref: opaque, tx-local
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
  normal form; `edit()` is the escape hatch.

- **The transaction borrows the old store sharedly.** This is the
  ergonomic point of the design: you can traverse the graph (with real
  handles, real read traits) while staging its replacement. The M2
  interleaving problem — `&mut` construction invalidating every handle —
  never arises.
- **Provisional refs** (`Ref<Node>` ≈ `Existing(NodeId) | Pending(u32)`)
  name entities inside the change set: new ones by submission order, old
  ones by id. They are opaque tokens, not handles — no read traits — and
  they die at flush. The `Existing` half shares identity with old-store
  handles through the stable id, and the transaction (which borrows the
  old store) can resolve one to a handle on request; only `Pending` refs
  lack identity, and only until flush. Refs are local to the transaction
  that issued them — a deliberate property: the change set is a
  first-class, self-contained value, which is the foundation for future
  directions (inspection, serialization, possibly merge) even though
  merging stays out of scope here. Staging never touches a store: the
  change set is a queue, not a private clone, so a staged `remove` cannot
  invalidate refs already handed out — everything is applied at once, at
  flush.
- **Flush** rebuilds from the old store's arenas (consumed and reused
  under `into_edit`, cloned under `edit`), replays the staged operations
  in submission order through the *same backing hooks* construction uses,
  drops removed elements and compacts, runs induction rules, calls the
  validation boundary, and returns the new store plus a report carrying
  per-arena old→new id maps. Kept elements keep their iteration order;
  new elements append in replay order; `intrusive_list`'s
  reverse-insertion contract applies to new inserts.
- **Flush errors are runtime data**, not compile errors: a `FlushError`
  lists each violation (dangling endpoint, duplicate key) by change-set
  op index. No spans; the op index is the debugging handle.

## Write traits mirror read traits

`implement!` gains a write side: per arrow instantiation, a marker-generic
trait implemented by the store's transaction type.

| arrow shape | read side | write side (on the transaction) |
|---|---|---|
| entity | root accessor, handles | `add_<entity>(<total one-arrow args>) -> Ref<E>` |
| total `one` | `e.target()` | constructor argument of `add_<entity>` |
| optional `one` | `e.payload() -> Option<_>` | `set_<arrow>(Ref<A>, Ref<B>)` |
| `many`, primitive | `p.tags()` | `insert_<arrow>(Ref<A>, Ref<B>)` — membership asserted directly |
| `many`, derived (`by`) | `n.out_edges()` | none — membership follows the determining `one` arrow's constructor argument or `set_`; no insert op is generated, so membership cannot contradict the field |
| keyed partial | `sg.subnode(mn)` | `insert_<arrow>(Ref<A>, Ref<K>, Ref<B>)` — function semantics; duplicate key is a flush error |

Derived arrows (`:= union`) have no write side, exactly as they have no
backing. During flush replay, a derived `many` arrow's hooks fire when
the determining field is replayed (constructor or `set_` op), so replay
order needs no special casing for membership. Primitive insertions into a
derived relation are impossible by construction — the method does not
exist — which is what the schema's `by` declaration buys (M1 syntax
section).

Markers are the same per-instance markers as the read side, so a
generic algorithm can bound on `OutEdges<M>` and `InsertOutEdges<M>`
together — e.g. a graph-transform pass that reads one store and stages
another without knowing either pattern.

## Nested stores

A change-set entry is an `(index path, operation)` pair — the same
addressing scheme handles already use. The transaction API exposes scoped
sub-transactions: `tx.within(node)` stages operations against the subgraph
store embedded in that (possibly provisional) node's record. Flush
replays root-level operations before nested ones, so provisional paths
resolve top-down.

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
under construction** (id-based getters or a restricted root handle) so
constructor code can follow arrows. The same view serves flush replay,
which reads copied old data plus already-replayed ops.

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
  retained raw ids (`NodeId(pub u32)`). Handles never cross stores, so
  this is type-safe; the maps exist for raw ids only.
- The pass discipline makes invalidation explicit: under `into_edit`,
  the old store is consumed and the borrow checker guarantees no handle
  survives the pass boundary.
- **Cascade policy is minimal.** A removal that would leave dangling
  references (removing a node with incident edges still present) is a
  `FlushError` listing the danglers — the change set must remove
  dependents explicitly. One exception: a nested store embedded in a
  removed record goes with the record, contents and all.
- Tombstones are rejected: they would complicate every backing contract
  (intrusive lists, size hints, CSR ranges) for a benefit
  rebuild-without already provides.

## Explicit non-goals

- No in-place mutation, ever. No generational ids.
- No handle migration across a flush. Handles name (store, path); a new
  store is a new identity. The id-remap report exists for raw ids, not
  handles.
- No concurrency or merging: two transactions on one store are
  independent forks; merging change sets is out of scope.

## Interactions with other milestones

- **Backings (M1–M3)**: unchanged. Hooks are the single write path, so
  every backing works under flush replay without an interface change.
  Backings that require a build phase (CSR) are *satisfied* by flush,
  which is always a full rebuild; the M3 constraint annotation
  ("append-only build phase") documents this.
- **Payloads (M5+)**: `set_<one-arrow>` is the payload update path.
- **Cost model**: flush is a deep copy of the entire store tree plus
  replay. Right for the target graph sizes; the report struct is where
  incremental reuse (persistent structures, chunked arenas) would attach
  if it ever becomes necessary.

## Task sequence (sketch)

1. Change-set data model + provisional refs for flat `Digraph`; flush =
   clone + replay + validate. Königsberg built via transaction must equal
   the M1 store in every observable way (same test assertions).
2. Write traits generated per arrow; a generic read→write transform
   (copy a graph through trait bounds only) as the portability proof.
3. Flush-error surface: dangling endpoint, duplicate key, unknown old id.
4. Nested stores: path-addressed ops and sub-transactions; hierarchy
   edit tests (stage a new outer node *and* its subgraph in one flush).
5. Induction for the subgraph pattern: the inducing `add_subedge`
   constructor as a builder extension over the generated primitives.
6. Removal with compaction and the id-remap report; cascade-policy tests
   (dangling removals rejected, nested stores dropped with their record).

## Risks

- **Provisional refs are a second handle-like concept.** Users will try
  to call read methods on them. Naming (`Ref` vs `Handle`) and docs carry
  the weight; the type system already refuses.
- **Flush error quality.** Errors are runtime data and point at op
  indices ("op 14: edge endpoint not in store"); the index alone is a
  weak debugging handle. If this proves painful, ops can record the
  staging call site's `Location` via `#[track_caller]`, so the error
  names the line that queued the offending op.
- **Full-rebuild cost hiding in benchmarks.** Fine at Königsberg scale;
  measure once hierarchy depth grows, not before.
