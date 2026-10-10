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
- Runtime multiplicity is not declared here: `sub` appears once at the type
  level; at runtime each outer node owns its own subgraph store, colocated
  in its record.

### Open entities

`pattern!` gains the ability to declare entities it does not own, to be
closed by fusion at composition time. `Subgraph` is defined *over* a main
graph without naming it:

```rust
pattern! {
    Subgraph {
        entities: SG, SN, SE, MN(open), ME(open);
        arrows {
            subnodes:  SG -> SN, many;
            subedges:  SG -> SE, many;
            main_node: SN -> MN, one;
            main_edge: SE -> ME, one;
            subnode:   SG -> SN, optional, key(MN);
            subedge:   SG -> SE, optional, key(ME);
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
- Removal: stores remain append-only. The hash index backing makes removal
  conspicuously absent — defer consciously, document loudly.
- Does `compose_pattern!` emit a new *pattern* (composable further,
  implementable many times) or a concrete composition? Assumption: a new
  abstract pattern; `implement!` stays the only place backings bind.

## Depends on

M1 complete: courier grammar, marker infrastructure, insert-hook interface.
M2 is the stress test of all three; expect the hook interface to grow
(keyed insert) and the courier grammar to gain fusion-class tokens.
