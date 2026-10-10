# Milestone 4: write-side builders — mutation by rebuild

**Goal**: change an existing graph without ever mutating a live store. The
write side is generated from the schema the same way the read side is: one
interface per arrow, marker-generic, so generic graph-transformation
algorithms are as portable as generic traversals.

Specification level: preliminary. The model and API shape below are
coherent end to end, but nothing here has been compiled. Two explicit
decision points remain (induction, removal); both are marked.

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
tx.set_payload(e, w);                // re-point an optional/payload arrow
let new_store = tx.flush()?;         // validate + rebuild; old store
                                     // untouched, old handles still valid
```

- **The transaction borrows the old store sharedly.** This is the
  ergonomic point of the design: you can traverse the graph (with real
  handles, real read traits) while staging its replacement. The M2
  interleaving problem — `&mut` construction invalidating every handle —
  never arises.
- **Provisional refs** (`Ref<Node>` ≈ `Existing(NodeId) | Pending(u32)`)
  name entities inside the change set: new ones by submission order, old
  ones by id. They are opaque tokens, not handles — no read traits, no
  `Eq`/`Hash` against handles — and they die at flush.
- **Flush** clones the old store's arenas (the whole nested structure,
  recursively), replays the staged operations in submission order through
  the *same backing hooks* construction uses, runs induction rules, calls
  the validation boundary, and returns the new store. Kept elements keep
  their ids and iteration order; new elements append in replay order;
  `intrusive_list`'s reverse-insertion contract applies to new inserts.
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
| `many` | `n.out_edges()` | `insert_<arrow>(Ref<A>, Ref<B>)` |
| keyed partial | `sg.subnode(mn)` | `insert_<arrow>(Ref<A>, Ref<K>, Ref<B>)` — function semantics; duplicate key is a flush error |

Derived arrows (`:= union`) have no write side, exactly as they have no
backing. Markers are the same per-instance markers as the read side, so a
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

## Decision point 1: induction

The edge-induced subgraph requires that inserting a subedge induces
subnodes for its main edge's endpoints. Where does that rule live?

- **Option A — schema-declared, flush-time.** A second derivation form
  beyond `union`, e.g.
  `subnodes := induced_by(subedges, via main_edge, endpoints [source, target])`.
  Declarative, order-independent, validated like any derivation. Cost: new
  schema vocabulary, and the path language needs bounds (which chains of
  arrows may appear in an induction rule?).
- **Option B — insert-time upsert hook.** `insert_subedge` calls
  `ensure_subnode` per endpoint, deduplicated via the hash index. Already
  demonstrated by the M2 tests driving builder primitives; no new
  vocabulary. Cost: order-dependent, crosses backing boundaries, and the
  rule is invisible to the schema (no introspection, no validation).

Recommendation: A as the semantic model, implemented narrowly — only the
path shapes the subgraph pattern needs — with B remaining available as
hand-written hooks for patterns the vocabulary can't express.

## Decision point 2: removal

- **Option A — no removal in v1.** Change sets are add/set-only.
  Consistent with the append-only lineage; flush and validation stay
  simple.
- **Option B — removal with compaction + id-remap report.** Flush rebuilds
  without the removed elements and returns per-arena `old → new` id maps.
  Handles never cross stores, so this is type-safe; the hazard is users
  retaining raw ids (`NodeId(pub u32)`), who must translate via the report.
- **Option C — tombstones.** Ids stay stable; arenas become
  `Vec<Option<Record>>` and every read-side iterator skips holes.
  Rejected: it complicates every backing contract (intrusive lists, size
  hints, CSR ranges) for a benefit rebuild-without already provides.

Recommendation: ship A, with flush's return type a report struct designed
to grow B's id maps without breaking callers.

## Explicit non-goals

- No in-place mutation, ever. No generational ids.
- No handle migration across a flush. Handles name (store, path); a new
  store is a new identity. The id-remap report (option B above) exists
  for raw ids, not handles.
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
5. Induction for the subgraph pattern (narrow form of decision 1-A).
6. Removal, if option B is taken, with the id-remap report.

## Risks

- **Provisional refs are a second handle-like concept.** Users will try
  to call read methods on them. Naming (`Ref` vs `Handle`) and docs carry
  the weight; the type system already refuses.
- **Flush error quality.** Op indices are weaker than spans. If this
  proves painful, change-set ops can record callsite `Location` (via
  `#[track_caller]`) cheaply.
- **Full-rebuild cost hiding in benchmarks.** Fine at Königsberg scale;
  measure once hierarchy depth grows, not before.
