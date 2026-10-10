# Milestone 2: composition, fusion, keyed arrows, the subgraph pattern

**Goal**: `compose_pattern!` works, and the edge-induced subgraph pattern —
the motivating higher-order graph — is expressible and testable.

Specification level: partial. Syntax sketches below are directional, not
final; M1 will teach us things about the courier grammar that reshape them.

## In scope

### `compose_pattern!`

Declares named pattern instances and entity fusions:

```rust
compose_pattern! {
    Hierarchy {
        instances: outer: Digraph, inner: Digraph, sub: Subgraph;
        sub.main_N <=> inner.N;
        sub.main_E <=> inner.E;
        outer.N    <=> sub.G;      // every outer node is-also a subgraph
    }
}
```

- `<=>` is symmetric and transitive: fusion is computed as equivalence
  classes over entities. The fused record sums all members' capability
  fields; the fused handle implements all members' traits.
- Fusion forces 1:1 correspondence at the type level: every object of the
  fused entity plays every role, possibly vacuously (an atomic node is a
  node whose subgraph is empty). Optionality is *not* fusion — it is an
  ordinary pointer field and stays out of `compose_pattern!`.
- **Fusion determines arena placement.** An entity fused with a
  store-bearing entity has its arenas embedded in the host record; unfused
  entities get root arenas. In the `Hierarchy` example: `inner.N`/`inner.E`
  arenas live at the root (one shared inner graph), while the `sub` store
  (SN/SE arenas plus their indexes) lives inside each outer node record.
  Handles borrow through the nesting; lifetimes are unchanged (one shared
  borrow of the root store).
- Runtime multiplicity is not declared here: `sub` appears once at the type
  level; at runtime each outer node owns its own subgraph store, colocated
  in its record.
- Backings bind per instance: `implement!` on a composed pattern keys the
  backing block by `instance.arrow` (`outer.out_edges:`,
  `sub.subnodes:`). Two instances of the same pattern may choose different
  backings.

### Open entities

`pattern!` gains the ability to declare entities it does not own, to be
closed by fusion at composition time. `Subgraph` is defined *over* a main
graph without naming it. Per the trait-identity invariant, every arrow
instantiates a root arrow pattern; `Subgraph`'s novel arrows (`main_node`,
`subnode`, …) are declared as their own single-arrow roots, either in the
stdlib or alongside:

```rust
pattern! {
    Subgraph {
        entities: SG, SN, SE, MN(open), ME(open);
        arrows {
            subnodes:  stdlib::Nodes,     SG -> SN;
            subedges:  EdgesOf,           SG -> SE;   // new root; stdlib candidate
            main_node: MainNode,          SN -> MN;   // local root arrow
            main_edge: MainEdge,          SE -> ME;   // local root arrow
            subnode:   SubNode,           SG -> SN, key(MN);
            subedge:   SubEdge,           SG -> SE, key(ME);
        }
    }
}
```

### Keyed partial arrows

New cardinality: `optional, key(K)` — a partial lookup parameterized by an
entity of another sort (`subnode(&self, main: MN) -> Option<SN>`). This is
the first relation whose method takes an argument beyond `&self`.

New built-in backing: `hash_index` — per-self-object map from key id to
target id, maintained by insert hooks. First backing with nontrivial
mutation hooks; validates the hook interface designed in M1.

### Marker qualification becomes real

Composed patterns instantiate the same arrow more than once on fused
entities. Trait impls are emitted per instance marker; ambiguous unqualified
method calls are rejected by the compiler naturally. The generated API must
provide a usable qualification spelling (full path
`<Handle as Nodes<Marker>>::nodes(&g)` at minimum; consider instance-scoped
inherent aliases).

### Structural validation (first checks)

- Fusion classes must be closed: no open entity survives composition.
- No record may contain itself without intervening indirection (cycle check
  over fusion classes + containment edges; `Vec`/hash arenas count as
  indirection).
- Ambiguous same-name arrows on a fused entity must be distinguishable by
  marker.

### Tests

- Edge-induced subgraph: build a main graph, build subgraphs, check
  `subnode`/`main_node` round-trips, conditional lookups return `None` for
  non-members.
- Hierarchy: outer graph whose nodes contain subgraphs of the inner graph;
  traverse both levels.
- Bipartite graph as a plain `pattern!` (two node sorts, arrows picked
  à la carte) — proves the no-renunciation thesis without any composition.
- Edge-chain refinement (`outer.E <=> chain.Chain`, `chain.Elem <=>
  inner.E`) if the milestone is going well; otherwise deferred to M3.

## Open questions going in

- Exact `<=>` syntax and how fusion classes are named in generated docs.
- Whether `subnode`'s key parameter is restricted to total `one` arrows
  originating nearby, or any entity in scope.
- Qualification ergonomics: bare full-path syntax may be too ugly; consider
  generating per-instance inherent methods (`g.nodes_left()`) as sugar.
- Root handles for composed patterns: with two unfused graph entities
  (`outer.G`, `inner.G`), what does the store's entry-point API look like —
  one accessor per root entity, or handles constructed directly from ids?
- Mutation API for nested stores: handles hold `&store`, so insertion into a
  node's subgraph needs an `&mut` path through the root store (e.g.
  `store.subnode_mut(outer_node)`); the shape of this API is undesigned.
- Derivations (`:= union`) are scoped to the instance that declares them;
  whether derived arrows spanning instances are ever needed is unknown.
- Removal: stores remain append-only. The hash index backing makes removal
  conspicuously absent — defer consciously, document loudly.
- Does `compose_pattern!` emit a new *pattern* (composable further,
  implementable many times) or a concrete composition? Assumption: a new
  abstract pattern; `implement!` stays the only place backings bind.

## Depends on

M1 complete: courier grammar, marker infrastructure, insert-hook interface.
M2 is the stress test of all three; expect the hook interface to grow
(keyed insert) and the courier grammar to gain fusion-class tokens.
