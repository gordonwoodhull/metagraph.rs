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
        sub.main_N <-> inner.N;
        sub.main_E <-> inner.E;
        outer.N    <=> sub.G;      // every outer node is-also a subgraph
    }
}
```

`compose_pattern!` emits a new abstract *pattern* — composable further,
implementable many times — and `implement!` stays the only place backings
bind.

Fusion has two flavors (epic invariant 4):

- `<=>` (colocation fusion) is symmetric and transitive: fusion is
  computed as equivalence classes over entities. The fused record sums
  all members' capability fields; the fused handle implements all
  members' traits, each under its own per-instance marker.
- `<->` (reference fusion) closes open entities across instances. It
  unifies id types only: no records merge, no arenas move, and runtime
  multiplicity is unconstrained — many SN records may reference the same
  inner node. An arrow targeting a reference-fused entity is an ordinary
  id field into the other store. Open entities are closed by `<->`, and
  may only be *targets* of arrows in their declaring pattern: under
  reference closure there is no record to hold a sourced arrow's field.
- Colocation fusion forces 1:1 correspondence at the type level: every
  object of the fused entity plays every role, possibly vacuously (an
  atomic node is a node whose subgraph is empty). Constructors merge
  accordingly: adding an object of a colocation-fused class takes the
  union of every member's total `one` arrow arguments. Optionality is
  *not* fusion — it is an ordinary pointer field and stays out of
  `compose_pattern!`.
- **Colocation fusion determines arena placement, per instance.** An
  instance's arenas are embedded in the host record exactly when the
  instance's root entity is colocation-fused with an element entity of
  the host instance; unfused roots keep root arenas. Reference fusion
  never affects placement. In the `Hierarchy` example: `inner`'s arenas
  live at the root (its root is unfused — one shared inner graph), while
  `sub`'s arenas (SN/SE plus their indexes) embed in each outer node
  record because `outer.N <=> sub.G`. Handles borrow through the nesting:
  a nested handle is the root store reference plus an index path.
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
their own single-arrow roots, either in the stdlib or alongside:

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
target id, maintained by the builder's insert hooks. First backing with
nontrivial mutation hooks; validates the hook interface designed in M1.
Because stores are immutable once built, there is no removal problem and
no dynamic validation: a keyed arrow is a function, so the builder checks
at flush time that every key id exists and that keys are unique per self
object, and reports violations as build errors. (`NodeId`'s `pub u32`
stays — the tests depend on it — so key integrity is enforced by the
builder, not by the type.)

### Builder shape

M2 keeps the M1 construction model: the store owns itself during building
and exposes `&mut` methods (`add_*`); no separate builder type, no change
sets, no removal, no write-side traits — those are M4. Three conventions
keep M4 additive rather than a rewrite:

- **Hooks are the single write path.** All storage mutation — field
  writes, intrusive links, hash-index maintenance — lives in per-backing
  hook functions over `&mut` store state; the `add_*` constructors are
  thin orchestration over them. (Pattern-level constructor logic such as
  induction sits in the orchestration layer, not inside backing hooks.)
  M4's flush replays change sets through these same hooks.
- **Builder methods are named and shaped as if they were already
  write-trait methods** (`add_<entity>`, `insert_<arrow>`,
  `set_<one-arrow>`), so M4 can lift them into generated write traits —
  defined from the arrows the same way the read traits are — without
  renaming.
- **Validation has a named boundary.** Flush-time checks (key existence,
  key uniqueness per self object) attach to a single `validate()` point
  even if M2 callers never invoke it separately; M4 hangs full
  referential-integrity validation there.

The index-path handle representation doubles as M4's change-set
addressing scheme: a change-set entry is an `(index path, operation)`
pair.

### Marker qualification becomes real

Composed patterns instantiate the same arrow more than once on fused
entities. Markers are re-minted per composition instance (M1 trait-
identity section) and named by module path
(`hierarchy::markers::outer::OutEdges`), so two instances of one pattern
never produce conflicting impls — the diamond case is two markers, not a
conflicting impl. Unqualified calls on fused entities simply fail to
infer; the usable spellings are partial turbofish
(`bfs::<_, _, markers::outer::OutEdges, _>(h)`) and full path
(`<Handle as Nodes<markers::outer::OutEdges>>::nodes(&g)`). Instance-
scoped inherent aliases (`g.nodes_outer()`) may be added as sugar if
qualification proves too ugly, under a hard rule: an inherent alias never
shares a name with an arrow method (inherent methods silently win
resolution over trait methods).

### Structural validation (first checks)

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
- Open entities are arrow targets only in their declaring pattern, and are
  closed by `<->` only.
- Field contributions must have a home after placement: reject a backing
  whose fields target a record that lives in another instance's arena,
  unreachable from the store being generated.
- Not a check, but documented: intrusive link fields are sound only when
  each object belongs to at most one relation instance (e.g. chains must
  partition their elements). Runtime data is invisible to the schema, so
  this becomes a backing constraint annotation in M3.

Marker collision is not a check: per-instance re-minting makes it
impossible by construction.

### Tests

- Edge-induced subgraph: build a main graph, build subgraphs through the
  builder, check `subnode`/`main_node` round-trips, conditional lookups
  return `None` for non-members. The builder's `add_subedge` induces
  subnodes for the edge's endpoints (deduplicated via the hash index);
  induction is constructor logic, not schema vocabulary (see M4).
- Hierarchy: outer graph whose nodes contain subgraphs of the inner graph;
  traverse both levels.
- Bipartite graph as a plain `pattern!` (two node sorts, arrows picked
  à la carte) — demonstrates no-renunciation assembly without composition.
  It also marks the limit of invariant 6: bipartiteness itself is a
  semantic constraint the arrows cannot state, and the test should not
  pretend otherwise.
- Edge-chain refinement (`outer.E <=> chain.Chain`, `chain.Elem <=>
  inner.E`) if the milestone is going well; otherwise deferred to M3.

## Deferred details

Pinned down during the milestone, not before:

- How fusion classes are named in generated docs.
- The key-entity restriction for keyed arrows: any entity in scope, or
  only entities reachable via nearby total `one` arrows.
- The store entry-point API for composed patterns with several unfused
  root entities (`outer.G`, `inner.G`): one accessor per root entity, or
  handles constructed directly from ids.
- Derived arrows are scoped to the instance that declares them;
  cross-instance derivations are out of scope. (The per-instance scoping
  is load-bearing: it keeps union's sibling rule away from
  fusion-collapsed arrow sets.)

## Depends on

M1 complete: courier mechanics, marker infrastructure, insert-hook
interface. M2 is the stress test of all three; expect the hook interface
to grow (keyed insert). The courier grammar does not grow: fusion-class
tokens travel inside the schema channel (M1 courier section).
