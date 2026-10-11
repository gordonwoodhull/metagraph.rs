# Milestone 2: composition, fusion, keyed arrows, the subgraph pattern

**Goal**: `compose_pattern!` works, and the edge-induced subgraph pattern —
the motivating higher-order graph — is expressible and testable.

Specification level: partial. Syntax sketches are directional, not final;
M1 will reshape details. Spike-verified (hand-built, tested): the
structural-validation phase assignments, the edge-chain refinement sketch,
and the courier chain itself end to end (three-member composition, fusion
classes, per-instance marker re-minting, placement codegen, nested
handles, generic `bfs` over composed handles — 16 tests). **Uncompiled:**
keyed partial arrows and the `hash_index` backing (the first backing whose
hooks maintain derived lookup state); the Subgraph pattern exactly as
sketched (the compiled version was simplified to `subnodes` + `main_node`,
and its local root lived in the stdlib — declaring a local root alongside
the pattern is unverified); the inducing `add_subedge` constructor and the
id-based read view it reads through (Builder shape, below); and the
bipartite test (M1 machinery only, but never run). Spike limits, now the
milestone's known frontier, stated in full: embedding depth 1 only;
intrusive arrows sourced at embedded classes or unfused roots rejected;
field backing into an embedded class rejected (its id needs a host path);
arena arrows into an embedded class hosted by another class rejected;
`<=>` transitivity exercised only on two-member classes; constructor
argument name collisions across instances of one fusion class silently
deduped (keeps first); and further composition of a composed pattern
untested (the emitted courier has the right shape, but the member-schema
grammar would need to nest).

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
with real spans. The design's central ergonomic claim is spike-verified:
fused handles plus per-instance markers let an unmodified generic `bfs`
run over composed handles, and unqualified method calls infer in every
case the qualification rules below say they should.

Spike-pinned mechanics: the chain's configuration (composed name, module
path, remaining-courier queue, fusion declarations, schemas collected so
far) rides verbatim in the courier's rest tail, with fusions staying
opaque token soup until the final hop; the *composed* schema embedded in
the final courier has its own structured grammar (`instances`, per-member
schemas, `fusions`), which `__implement_gen!` distinguishes from a flat
schema by first keyword; and member schemas re-emit their `root`/`(open)`
entity markers so the placement rule and closure checks read them after
the hops.

Fusion has two flavors (epic invariant 4):

- `<=>` (colocation fusion) is symmetric and transitive: fusion is
  computed as equivalence classes over entities. The fused record sums
  all members' capability fields; the fused handle implements all
  members' traits, each under its own per-instance marker.
- `<->` (reference fusion) closes open entities across instances. It
  unifies id types only: no records merge, no arenas move, and runtime
  multiplicity is unconstrained — many SubNode records may reference the
  same inner node. An arrow targeting a reference-fused entity is an
  ordinary id field into the other store. Codegen resolves this through
  an explicit **partner map**: at every id/handle/field-type use, an open
  entity resolves to its fusion partner's class (spike-verified; fields
  into open entities work as ordinary resolved id fields). Open entities
  are closed by `<->`, and may only be *targets* of arrows in their
  declaring pattern: under reference closure there is no record to hold
  a sourced arrow's field.
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
  entity of the host instance; an instance whose root is unfused keeps
  its arenas at the store's root level. Reference fusion never affects
  placement. The root-marked entity owns no arena, flat or composed
  (M1's naming rule): its "record" is the (sub)store, and instances
  contribute arenas for their non-root entities only. In the `Hierarchy` example: `inner`'s
  arenas live at the root (its root is unfused — one shared inner graph),
  while `sub`'s arenas (SubNode/SubEdge plus their indexes) embed in each
  outer node record because `outer.Node <=> sub.Subgraph`. Handles borrow
  through the nesting: a nested handle is the root store reference plus an
  index path. Placement is a per-instance decision read off the root's
  fusion class, and embedding is expressed as `Vec<Data>` *fields on the
  host class's record struct*; constructors of embedded classes take the
  host id as their first argument and return path ids, and insert hooks
  address records through nested paths
  (`store.outer_node_arena[h].sub_subnode_arena[i]`).
- **Naming under composition is mechanical and collision-first.** Entity
  names declared in more than one instance get instance-prefixed types
  (`OuterNode` vs `InnerNode`); arena fields are always
  instance-qualified (`outer_node_arena`, `sub_subnode_arena`); iterators
  likewise (`OuterOutEdgesIter`). Write-op names derive from the
  instantiation's local names (M1) and take the instance prefix on
  collision (`add_outer_node`). The store entry point for a composed
  pattern with several unfused roots is one instance-prefixed accessor
  per unfused root entity (`outer_graph()`, `inner_graph()`).
- **Store-less instances still have write ops.** An instance whose root
  is colocation-fused into a host element and whose remaining entities
  are `<->`-closed owns no arenas at all (`chain` in the Refinement
  sketch) — but its `many` arrows still need insert operations, and
  constructor merging covers only total `one` arrows. The insert op is
  emitted on the store hosting the fused *self* record, named
  `insert_<arrow>` from the instantiation's local arrow name by the usual
  convention — `insert_elems(OuterEdgeId, InnerEdgeId)` for the chain
  sketch (the spike's hand-written code called it `insert_chain_elem`;
  that was a deviation from the convention, not a precedent); its backing
  hooks write through to whichever arena in the store tree holds the
  target records.
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
            subnode:   SubNodeOf,     Subgraph -> SubNode, key(MainNode);
            subedge:   SubEdgeOf,     Subgraph -> SubEdge, key(MainEdge);
        }
    }
}
```

Read-side method names here come from the roots, not the local arrow
names: `subnodes` is called `.nodes()`. The local names drive the
generated write ops (`insert_<arrow>`) and marker names. See M1's
trait-identity section for the alias rule.

### Keyed partial arrows

New cardinality: `optional, key(K)` — a partial lookup parameterized by an
entity of another sort (`subnode(&self, main: MainNode) -> Option<SubNode>`).
This is the first relation whose method takes an argument beyond `&self`;
M1's root-trait grammar reserves the parameter slot for it. The keyed
cardinality is declared on the root, which owns the method and its key
parameter; the instantiation supplies only the key-entity binding
(`key(MainNode)`), a local clause like `by` — it never restates the
cardinality (see M1's trait-identity section).

New built-in backing: `hash_index` — per-self-object map from key id to
target id, maintained by the builder's insert hooks. It is the first
backing whose hooks maintain *derived lookup state* (intrusive_list's
hooks already mutate records; hash_index's maintain a secondary index),
and it validates the hook interface designed in M1. One configuration is
beyond the spike frontier: a keyed arrow *targeting an embedded class*
(Hierarchy with `sub.subnode` bound) stores target ids the way field
backing does — exactly the configuration spike 1's codegen rejects for
fields, because the target's id is a host-anchored path. Spike 11 (keyed
lookup over path ids, see spikes.md) must land before any Hierarchy test
binds the keyed arrows. The edge-induced test is unaffected: with an
unfused `sub` root, SubNode lives at root level and its ids are flat.

Checks attach to the **`validate()` boundary** M1 establishes: key
existence (every key id names a real record) and key uniqueness per self
object (a keyed arrow is a function), joined by the intrusive-ownership
partition check and per-backing structural checks (Structural
validation). In M2 there is no flush, so the
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
Three conventions keep M4 additive rather than a rewrite, plus one
read-side addition it will reuse:

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
- **The builder exposes a read view of the store under construction.**
  Pattern-level constructor logic needs to *read* — the inducing
  `add_subedge` (Tests, below) follows `main_edge` to `source`/`target`
  — but during construction the builder owns the store mutably, and
  pattern code can't touch generated private fields. The read view is
  generated id-based getters: field reads by id, returning ids and
  copies — never handles. Handles are rejected here on purpose: a
  whole-store handle's shared borrow cannot coexist with the `&mut`
  field access that mutates a nested store (the borrow checker forbids
  it exactly in the case induction needs). A restricted root handle is
  viable only in the flat case, where the store being read is not the
  store being mutated; id getters work in both. M4's flush replay reads
  through the same view (copied old data plus already-replayed ops).
  Uncompiled, like the induction constructor that needs it.

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

The phase assignments below have been spike-verified: each check was
implemented as a plain function over a hand-written schema struct, with
a pass and a fail fixture per check, plus the Hierarchy and Refinement
compositions as realistic inputs.

One check runs at `pattern!` time (needs only the declaring pattern):

- Open entities are arrow *targets* only in their declaring pattern.

Two run at `compose_pattern!` time — all member schemas and fusions are
in hand at the final hop, which the courier chain guarantees:

- Fusion classes must be closed: no open entity survives composition.
- Every colocation-fusion class has exactly one arena placement. This
  rejects both double-closed entities (`a.X <=> b.N` *and* `a.X <=> c.N`,
  where transitivity merges arenas from two instances) and cross-
  multiplicity colocations (fusing an entity whose arena embeds per-record
  with one whose arena lives at root): each asks one id type to index two
  arenas.
- Open entities are closed by `<->` only.

Two run at `implement!` time, because they need backing bindings:

- No record may contain itself without intervening indirection. (Moved
  from compose time, where the spike found it *vacuous*: containment
  edges come from colocation placement — always indirect, since arenas
  are `Vec`-backed — and from backing contributions that embed records
  inline; id-typed fields count as indirection. With only the M1–M3
  built-ins every edge is indirect, so the check can never fire without
  per-backing contribution typing (inline vs id), which is
  implement!-time information.)
- Field contributions must have a home after placement. One `implement!`
  call generates **one root store**; every instance's arenas live in
  that store's tree (root-level, or embedded per host record by the
  placement rule), so a contribution has a home iff its target record
  sort is placed within the tree. Its errors quote the offending tokens —
  spans point at the `implement!` call site. Multi-store layouts (e.g. a
  separately-owned shared inner store) are out of scope: making this
  check non-vacuous there requires vocabulary the schema does not have —
  a store partition (which fusion class lives in which store), a
  directional store-reachability relation, and the emitting store per
  binding (derivable from the self-record's class). That vocabulary
  belongs to multi-store layouts, which remain out of scope; record it
  as the prerequisite when they arrive.

Two kinds of check run at `validate()` time (runtime; caller-invoked in
M2, forced by M4's flush). This is the canonical inventory — M4's flush
runs exactly these:

- **Key checks** (keyed partial arrows): key existence (every key id
  names a real record) and key uniqueness per self object (a keyed
  arrow is a function).
- **Per-backing structural checks.** Endpoint ids in arena range;
  intrusive link well-formedness (in-range links). For `intrusive_list`
  there is additionally an exclusive-membership contract — built in
  while the backings are a fixed table (M1–M2), surfaced as a constraint
  annotation when M3's `backing!` DSL lands. How the contract is
  *checked* depends on membership kind. For a `by`-derived arrow,
  membership consistency against the determining field covers it: each
  record appears exactly once in the list its determining field names,
  so shared membership cannot be expressed consistently at all. For a
  *primitive* intrusive arrow (`chain.elems` in the refinement sketch)
  there is no determining field, so the check is the
  **intrusive-ownership partition check**: walk every list once with a
  per-walk seen set and a global seen set — an id appearing in *two*
  lists is shared membership (which silently truncates the old chain);
  an id repeating *within* one list is a cycle (which makes traversal
  diverge). The check distinguishes the two failure shapes in its
  report. Its inputs are all available by `implement!` time: the self
  entity (head field) and target entity (link field), the field names
  from the backing's contribution list, and the store holding the target
  arena. (Without the partition check both corruptions are silent — the
  spike reproduced both.)

Marker collision is not a check: per-instance re-minting makes it
impossible by construction.

### Tests

- Edge-induced subgraph: build a main graph, build subgraphs through the
  builder, call `validate()`, check `subnode`/`main_node` round-trips,
  conditional lookups return `None` for non-members. The builder's
  `add_subedge` induces subnodes for the edge's endpoints (deduplicated
  via the hash index); induction is constructor logic, not schema
  vocabulary (see M4), and the inducing constructor reads through the
  builder's id-based read view (Builder shape, above). Both the
  constructor and the read view are uncompiled.
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

  This sketch has been hand-built and run (spike): all structural checks
  pass under the one-root-store layout, a chain walk is literally the
  `out_edges` walk (a generic `OutEdges<M>` traversal applied unchanged),
  and the unchecked intrusive-ownership hazard was reproduced — which is
  where the `validate()` partition check above comes from.

## Deferred details

Pinned down during the milestone, not before:

- How fusion classes are named in generated docs.
- The key-entity restriction for keyed arrows: any entity in scope, or
  only entities reachable via nearby total `one` arrows.
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
