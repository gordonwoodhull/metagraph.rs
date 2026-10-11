# Milestone 2: composition, fusion, keyed arrows, the subgraph pattern

**Goal**: `compose_pattern!` works, and the edge-induced subgraph pattern —
the motivating higher-order graph — is expressible and testable.

Specification level: partial. Syntax sketches are directional, not final;
M1 will reshape details.

## In scope

### `compose_pattern!`

Declares named pattern instances and entity fusions:

```rust
compose_pattern! {
    Hierarchy {
        instances: outer: Digraph, inner: Digraph, sub: Subgraph;
        sub.MainNode <-> inner.Node;
        sub.MainEdge <-> inner.Edge;
        outer.Node   <=> sub.Subgraph;  // every outer node is-also a subgraph
    }
}
```

`compose_pattern!` emits a new abstract *pattern* — composable further,
implementable many times — and `implement!` stays the only place backings
bind.

**Mechanism: the courier chain.** `compose_pattern!` is a proc macro and
cannot read its member patterns' tokens, so it collects their schemas
through the same courier channel M1 establishes. It emits an invocation of
the first member's courier with a doc-hidden `__compose_step!` proc macro
as the forwarding target, carrying the queue of remaining member couriers,
the fusion declarations, and the schemas collected so far; each step
appends one member schema and emits the next courier invocation. The final
hop computes fusion classes, mints the per-instance marker modules, and
emits the composed pattern's own courier — the same `#[macro_export]` +
`pub use` shape, embedding the full composed schema (instances, fusions,
marker paths). A composed pattern is then indistinguishable from a
hand-written one: same markers-module layout, same courier rule, same
path-derivation rule, composable and implementable by the same machinery.
The courier rule never grows; composition only reuses the target
parameter. Because all member schemas are collected at compose time, the
structural validation checks (below) run at the outermost proc-macro layer
with real spans.

Fusion has two flavors (epic invariant 4):

- `<=>` (colocation fusion) is symmetric and transitive: fusion is
  computed as equivalence classes over entities. The fused record sums
  all members' capability fields; the fused handle implements all
  members' traits, each under its own per-instance marker.
- `<->` (reference fusion) closes open entities across instances. It
  unifies id types only: no records merge, no arenas move, and runtime
  multiplicity is unconstrained — many SubNode records may reference the
  same inner node. An arrow targeting a reference-fused entity is an
  ordinary id field into the other store. Open entities are closed by
  `<->`, and may only be *targets* of arrows in their declaring pattern:
  under reference closure there is no record to hold a sourced arrow's
  field.
- Colocation fusion forces 1:1 correspondence at the type level: every
  object of the fused entity plays every role, possibly vacuously (an
  atomic node is a node whose subgraph is empty). Constructors merge
  accordingly: adding an object of a colocation-fused class takes the
  union of every member's total `one` arrow arguments. Optionality is
  *not* fusion — it is an ordinary pointer field and stays out of
  `compose_pattern!`.
- **Colocation fusion determines arena placement, per instance.** An
  instance's arenas are embedded in the host record exactly when the
  instance's `root`-marked entity is colocation-fused with an element
  entity of the host instance; unfused roots keep root arenas. Reference
  fusion never affects placement. In the `Hierarchy` example: `inner`'s
  arenas live at the root (its root is unfused — one shared inner graph),
  while `sub`'s arenas (SubNode/SubEdge plus their indexes) embed in each
  outer node record because `outer.Node <=> sub.Subgraph`. Handles borrow
  through the nesting: a nested handle is the root store reference plus an
  index path.
- Runtime multiplicity is not declared here: `sub` appears once at the type
  level; at runtime each outer node owns its own subgraph store, colocated
  in its record.
- Backings bind per instance: `implement!` on a composed pattern keys the
  backing block by `instance.arrow` (`outer.out_edges:`,
  `sub.subnodes:`). Two instances of the same pattern may choose different
  backings.

### Open entities

`pattern!` gains the ability to declare entities it does not own, to be
closed by reference fusion at composition time. `Subgraph` is defined
*over* a main graph without naming it. Open entities may only be arrow
*targets* within their declaring pattern (closing by `<->` merges no
records, so a sourced arrow's field would have no home); open-as-source
with colocation closure and constructor merging is deferred. Per the
trait-identity invariant, every arrow instantiates a root arrow pattern;
`Subgraph`'s novel arrows (`main_node`, `subnode`, …) are declared as
their own single-arrow roots, either in the stdlib or alongside — local
roots register themselves through the same courier mechanism (M1), so
neither placement needs privilege:

```rust
pattern! {
    Subgraph {
        entities: root Subgraph, SubNode, SubEdge, MainNode(open), MainEdge(open);
        arrows {
            subnodes:  stdlib::Nodes, Subgraph -> SubNode;
            subedges:  EdgesOf,       Subgraph -> SubEdge;   // new root; stdlib candidate
            main_node: MainNode,      SubNode -> MainNode;   // local root arrow
            main_edge: MainEdge,      SubEdge -> MainEdge;   // local root arrow
            subnode:   SubNodeOf,     Subgraph -> SubNode, optional, key(MainNode);
            subedge:   SubEdgeOf,     Subgraph -> SubEdge, optional, key(MainEdge);
        }
    }
}
```

### Keyed partial arrows

New cardinality: `optional, key(K)` — a partial lookup parameterized by an
entity of another sort (`subnode(&self, main: MainNode) -> Option<SubNode>`).
This is the first relation whose method takes an argument beyond `&self`;
M1's root-trait grammar reserves the parameter slot for it.

New built-in backing: `hash_index` — per-self-object map from key id to
target id, maintained by the builder's insert hooks. It is the first
backing whose hooks maintain *derived lookup state* (intrusive_list's
hooks already mutate records; hash_index's maintain a secondary index),
and it validates the hook interface designed in M1.

Checks attach to the **`validate()` boundary** M1 establishes: key
existence (every key id names a real record) and key uniqueness per self
object (a keyed arrow is a function). In M2 there is no flush, so the
caller invokes `validate()` explicitly — the tests do, and the docs
recommend it as the last construction step; an unchecked store is possible
and is the caller's responsibility. M4's flush invokes `validate()`
unconditionally, which is where the boundary becomes forced. (`NodeId`'s
`pub u32` stays — the tests depend on it — so key integrity is enforced
by validation, not by the type.)

### Builder shape

M2 keeps the M1 construction model: the store owns itself during building
and exposes `&mut` methods (`add_*`); no separate builder type, no change
sets, no removal, no write-side traits — those are M4. (Terminology: the
store in its `&mut` construction phase is *the builder*; M4's staging
type is *the transaction*. M4's flush re-runs the builder machinery.)
Three conventions keep M4 additive rather than a rewrite:

- **Hooks are the single write path.** All storage mutation — field
  writes, intrusive links, hash-index maintenance — lives in the named
  per-backing hook functions M1 commits to, over `&mut` store state; the
  `add_*` constructors are thin orchestration over them. (Pattern-level
  constructor logic such as induction sits in the orchestration layer,
  not inside backing hooks.) M4's flush replays change sets through
  these same hooks.
- **Builder methods are named and shaped as if they were already
  write-trait methods** (`add_<entity>`, `insert_<arrow>`,
  `set_<one-arrow>`), so M4 can lift them into generated write traits —
  defined from the arrows the same way the read traits are — without
  renaming.
- **Validation has a named boundary** (`validate()`, above), so M4 hangs
  full referential-integrity validation on a point that already exists.

The index-path handle representation doubles as M4's change-set
addressing scheme: a change-set entry is an `(index path, operation)`
pair.

### Marker qualification becomes real

Composed patterns instantiate the same arrow more than once on fused
entities. Markers are re-minted per composition instance by the
composition chain's final hop (fresh unit structs, not re-exports —
re-exported shared markers would produce conflicting impls on fused
handles) and named by module path
(`hierarchy::markers::outer::OutEdges`), so two instances of one pattern
never produce conflicting impls — the diamond case is two markers, not a
conflicting impl. Marker paths derive from the composed pattern's path by
the M1 rule, so `implement!` resolves them from any crate without
configuration. Unqualified calls on fused entities simply fail to
infer; the usable spellings are partial turbofish
(`bfs::<_, _, markers::outer::OutEdges, _>(h)`) and full path
(`<Handle as Nodes<markers::outer::OutEdges>>::nodes(&g)`). Instance-
scoped inherent aliases (`g.nodes_outer()`) may be added as sugar if
qualification proves too ugly, under a hard rule: an inherent alias never
shares a name with an arrow method (inherent methods silently win
resolution over trait methods).

### Structural validation (first checks)

Four of these run at `compose_pattern!` time — all member schemas and
fusions are in hand at the final hop, which the courier chain guarantees:

- Fusion classes must be closed: no open entity survives composition.
- No record may contain itself without intervening indirection (cycle check
  over fusion classes + containment edges; `Vec`/hash arenas count as
  indirection).
- Every colocation-fusion class has exactly one arena placement. This
  rejects both double-closed entities (`a.X <=> b.N` *and* `a.X <=> c.N`,
  where transitivity merges arenas from two instances) and cross-
  multiplicity colocations (fusing an entity whose arena embeds per-record
  with one whose arena lives at root): each asks one id type to index two
  arenas.
- Open entities are arrow targets only in their declaring pattern (also
  checkable at `pattern!` time), and are closed by `<->` only.

One runs at `implement!` time, because it needs backing bindings:

- Field contributions must have a home after placement: reject a backing
  whose fields target a record that lives in another instance's arena,
  unreachable from the store being generated. Its errors quote the
  offending tokens — spans point at the `implement!` call site.

Not a check, but documented: intrusive link fields are sound only when
each object belongs to at most one relation instance (e.g. chains must
partition their elements). Runtime data is invisible to the schema, so
this becomes a backing constraint annotation in M3.

Marker collision is not a check: per-instance re-minting makes it
impossible by construction.

### Tests

- Edge-induced subgraph: build a main graph, build subgraphs through the
  builder, call `validate()`, check `subnode`/`main_node` round-trips,
  conditional lookups return `None` for non-members. The builder's
  `add_subedge` induces subnodes for the edge's endpoints (deduplicated
  via the hash index); induction is constructor logic, not schema
  vocabulary (see M4).
- Hierarchy: outer graph whose nodes contain subgraphs of the inner graph;
  traverse both levels.
- Bipartite graph as a plain `pattern!` (two node sorts, arrows picked
  à la carte) — demonstrates no-renunciation assembly without composition.
  It also marks the limit of invariant 6: bipartiteness itself is a
  semantic constraint the arrows cannot state, and the test should not
  pretend otherwise.
- Edge-chain refinement, modeled *without* an element record sort:

  ```rust
  pattern! {
      Chain {
          entities: root Chain, Elem(open);
          arrows { elems: stdlib::OutEdges, Chain -> Elem, many; }  // intrusive_list
      }
  }

  compose_pattern! {
      Refinement {
          instances: outer: Digraph, inner: Digraph, chain: Chain;
          outer.Edge <=> chain.Chain;   // outer edge is-also chain
          chain.Elem <-> inner.Edge;    // elements are inner edges, by reference
      }
  }
  ```

  There is no Elem arena at all: chain membership is threaded through the
  inner-edge records themselves, exactly the way `out_edges` threads
  through edge records — a head field on the fused outer-edge record, a
  `next` field on each inner edge. The placement rule fires vacuously:
  `chain`'s root is colocation-fused with an element of `outer`, but
  `chain` owns no arenas (Elem is open, closed by `<->`), so nothing
  embeds, and the only colocation class has a single placement. Each
  inner edge carries one `next` link, so each inner edge belongs to at
  most one chain — the intrusive-ownership constraint above, and the
  right semantics for refinement. Because `elems` instantiates
  `stdlib::OutEdges`, walking a chain *is* `out_edges`; generic
  traversals apply to chains unchanged.

  The general rule this demonstrates: intrusive links model *exclusive*
  membership with no allocation; *shared* membership (one object in
  several relations) requires link records with their own arena, and that
  is where arena-placement conflicts genuinely arise. The sketch's first
  draft colocated an element entity (`chain.Elem <=> inner.Edge`) and was
  rejected by the exactly-one-placement check — the element sort was a
  phantom, and reference fusion plus an intrusive arrow says what was
  meant.

## Deferred details

Pinned down during the milestone, not before:

- How fusion classes are named in generated docs.
- The key-entity restriction for keyed arrows: any entity in scope, or
  only entities reachable via nearby total `one` arrows.
- The store entry-point API for composed patterns with several unfused
  root entities (`outer.Graph`, `inner.Graph`): one accessor per root
  entity, or handles constructed directly from ids.
- Derived arrows are scoped to the instance that declares them;
  cross-instance derivations are out of scope. (The per-instance scoping
  is load-bearing: it keeps union's sibling rule away from
  fusion-collapsed arrow sets.)

## Depends on

M1 complete: courier mechanics (including the target-parameterized rule
this milestone's composition chain reuses), marker infrastructure,
root registration couriers (the Subgraph sketch's local roots need them),
the insert-hook interface, and the `validate()` boundary. M2 is the stress
test of all five; expect the hook interface to grow (keyed insert).
