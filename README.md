# metagraph

A Rust toolkit for constructing higher-order graphs. Graph *types* are
assembled from atomic pieces: abstract single-arrow patterns declare
entities and relations, composition glues patterns together by fusing
entities, and `implement!` binds each arrow to a storage backing and
emits the concrete store, handles, and trait impls. Trait identity comes
from the arrow pattern, not the store, so generic algorithms port across
schemas and across implementations of the same schema.

The base directed graph is not built in: it is the degenerate
composition — six single-arrow patterns instantiated in one `pattern!`
block. Bipartite graphs, hierarchical graphs (a node that is also a
subgraph), edge-refinement graphs (an edge that is also a chain), and
edge-induced subgraphs are the same machinery applied to bigger piles of
arrows.

This is a research toolkit mid-build. The read side and the write side
(mutation by rebuild) are implemented and tested; see
[Roadmap](#roadmap) for what is deliberately not here yet.

## The pipeline

```
pattern!           abstract type graph: entities + single-arrow relations
                   (cardinalities one / many / keyed partial). No storage.
compose_pattern!   glues pattern instances by entity fusion:
                   colocation (<=>, is-also: one record, summed capabilities)
                   or reference (<->, open entities closed by id unification)
implement!         binds each arrow to a backing (arena, intrusive_list,
                   csr_adjacency, hash_index, field), emits store + handles
                   + trait impls + validate() + the transaction write side
```

A sketch of the whole arc (adapted from the crate-root docs, where these
examples compile as doctests):

```rust
use metagraph::implement;
use metagraph::stdlib::{Nodes, OutEdges, Target};

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        // `source`/`target` are `one` arrows: they default to `field`.
    }
}

let mut s = GraphStore::new();
let a = s.add_node();
let b = s.add_node();
let e = s.add_edge(a, b);    // used again in the write example below
assert!(s.validate().is_ok());

let g = s.graph();                        // the root handle
let a = g.nodes().next().unwrap();
let e = a.out_edges().next().unwrap();    // reverse insertion order
assert_eq!(e.target().id(), b);
```

## Reading and writing

Handles are wide pointers — a root store reference plus an index path
into nested arenas — so the borrow checker freezes a store once handles
exist. Reading is handle traversal against the generated arrow traits
(`Nodes`, `OutEdges`, `Source`, …); generic algorithms
(`metagraph::algo::bfs`, `dfs`) are written against those traits and
apply to any store whose pattern instantiates the same arrows.

Stores are immutable once built. Mutation goes through a transaction:
`edit()` borrows the store and stages ops (`add_*`, `remove_*`,
`insert_<arrow>`, `set_<arrow>`); `flush()` rebuilds — removals first,
then constructive ops in submission order — running dangler detection
and `validate()` on the way, and returns the new store plus an old→new
id-remap report. The old store is untouched; a rejected change set
leaves nothing half-applied.

```rust
let mut tx = s.edit();
tx.remove_edge(e);
let c = tx.add_node();
tx.add_edge(a, c);
let out = tx.flush().expect("coherent change set");

assert_eq!(out.report.edge_map, vec![None, Some(0)]);
assert_eq!(out.store.edge_count(), 2);
out.store.validate().expect("flushed store valid");
```

Complete, tested examples live in the test suite:
`tests/koenigsberg.rs` (read side), `tests/tx_flush.rs` and
`tests/tx_set_unlink.rs` (write side), `tests/hierarchy_subgraphs.rs`
(composition), and `tests/downstream/` (trait portability across
crates).

## Feature inventory

- **Backings**: `arena`, `field`, `intrusive_list`, `hash_index`,
  `csr_adjacency` (phased: build-phase buckets compact at `freeze()`),
  plus derived membership (`by <one-arrow>`) and derived reads
  (`:= sum(a, b)`) that take no backing at all.
- **Composition**: colocation fusion (`<=>`) and reference fusion
  (`<->`), nested stores (arenas embedded in host records), subgraph and
  edge-refinement patterns in `stdlib` beside the six base arrows.
- **Keyed arrows**: partial maps with `hash_index`, store-level or
  per-record, carried (never rebuilt) across a flush with a
  survivors-only cascade on key removal.
- **Transactions**: staged verbs with handle / raw-id / pending-ref
  arguments; staging *panics* for ops that would silently corrupt the
  change set, `FlushError` for ops invalid against the store as a whole;
  deterministic, byte-stable error reports; `check()` predicates a flush
  without building; `into_edit()` consumes a non-phased store and reuses
  its allocations; a `__stage_custom` escape hatch for user-defined
  induction (e.g. edge-induced subgraphs).
- **Write-side traits** for generic transforms: `Add<E>` / `Remove<E>`
  per entity, `Insert<Arrow>` / `Set<Arrow>` per arrow, and
  `tx::HandleToRef` bridging old-store handles into a transaction — all
  marker-generic, one marker parameter per entity or arrow.
- **Validation**: every store emits `validate()` summoning one authored
  clause per backing binding, behind a uniform range pre-pass over every
  contributed link field.
- **User-declared roots**: the stdlib is not a closed set — any crate
  may declare a root single-arrow pattern; roots self-register their
  trait metadata and `implement!` resolves them uniformly.

## Design invariants

The load-bearing ones (the full list is in `plans/epic.md`):

- **Composition only ever adds.** No renunciation; unwanted relations
  are never imported because patterns assemble from single arrows.
- **Abstract before concrete.** `pattern!` / `compose_pattern!` never
  mention storage; backings bind at `implement!`. The same abstract
  pattern can be implemented multiple ways.
- **Markers disambiguate.** Generated traits are generic over one marker
  per arrow instance, scoped per composition instance and named by
  module path — never by name concatenation. Unqualified calls resolve
  when unique; on fused entities, qualification is the norm.
- **Stores are immutable; mutation is rebuild.** No generational arenas;
  removal is rebuild-without. Handle identity is (store pointer, id),
  never the id alone.
- **Structural rejection, not semantic laws.** The toolkit rejects
  impossible patterns (fusion cycles without indirection, unclosed open
  entities, conflicting arena placement) but does not check semantic
  constraints like bipartiteness or acyclicity.

## Non-goals and current limits

- No in-place mutation of a live store, ever; no handle migration across
  a flush (the remap report is for raw ids, not handles).
- No per-membership un-insert: a `many` membership or keyed entry is
  removed by removing the record that carries it, or by re-pointing a
  `one` arrow via `set_`.
- No concurrency or change-set merging; two transactions on one store
  are independent forks (cross-fork refs panic at staging).
- Phased (`csr_adjacency`) stores have borrowed-only transactions — no
  `into_edit()` — and composed stores with a phased binding emit no
  transaction yet.
- Nested adds return no ref, and embedded intrusive chains stay outside
  the transaction surface.
- Not yet published to crates.io; depend on it by path or git.

## Roadmap

Two further milestones are planned in detail but not yet landed
(`plans/milestone-5-data.md` and `plans/milestone-6-crossing.md`,
currently on their own branches):

- **M5 — implement-time data.** User payload types bind at `implement!`
  as data on records — not entities, not arrows, so patterns stay
  type-abstract and the same pattern implements with data, without data,
  or with different data (layout locations, labels, cached computations).
  Reads ride a `.data()` gateway; writes ride the ordinary ladder —
  constructor argument, builder assignment, staged field-sets through
  flush. The maintenance boundary: *arrows are maintained by the
  toolkit; data is maintained by you.*
- **M6 — the crossing.** Ports, portals, and the hosted inner graph
  traversing as a graph: an edge leaves the outer graph, attaches at a
  port on a host node, crosses a portal, and continues among the inner
  graph's own nodes, with every hop a stock root-trait call. Three
  growths get there: optional-`one` arrows, intrusive list heads at
  embedded records, and `set_`/flush remap extended into nested stores.

Beyond those, threads we can see but haven't scoped (see
`plans/epic.md`):

- Automatic backing selection (backings declare applicability and cost
  hints; `implement!` picks, user overrides).
- An interop backend — implement the six base arrows over petgraph or a
  remote graph — to prove the traits are backend-portable.
- Derived arrows beyond `sum`: inverse, transitive closure, arrow
  composition.
- Compile-time schema introspection for codegen of serializers,
  visualizers, fuzzers.
- Arrow-shaped payloads (trait-visible weights as arrows into value
  arenas) — M5 covers data-on-records; this spelling stays open.

## Design documents

`plans/` holds the working documents: `epic.md` (vision, invariants,
terminology), per-milestone specs, and `spikes.md` (twelve throwaway
implementability probes). `HANDOFF-*.md` files are milestone handoff
records. Where plan text and landed code disagree, the code is right.

## Development

Workspace of two crates: `metagraph` (runtime: handles, stdlib
patterns/backings, transaction machinery) and `metagraph-macros`
(`pattern!`, `backing!`, `compose_pattern!`, `implement!`).

```
cargo test --workspace
```

runs everything: the unit and integration suites, `trybuild`
compile-fail fixtures (`tests/compilefail/`), downstream portability
crates (`tests/downstream/`), and doctests.
