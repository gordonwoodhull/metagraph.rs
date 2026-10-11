# Milestone 2: composition, fusion, keyed arrows, the subgraph pattern

**Goal**: `compose_pattern!` works, and the edge-induced subgraph pattern —
the motivating higher-order graph — is expressible and testable.

Specification level: partial. Syntax sketches are directional, not final;
M1 will reshape details. Spike-verified (hand-built, tested): the
structural-validation phase assignments, the edge-chain refinement sketch,
and the courier chain itself end to end (three-member composition, fusion
classes, per-instance marker re-minting, placement codegen, nested
handles, generic `bfs` over composed handles — 16 tests). **Uncompiled:**
the macro side of keyed partial arrows and the `hash_index` backing (the
mechanism is hand-built and tested in both configurations — spike 11
covers the embedded target: path-id values, store-level hook, concrete-key
trait shape; a review probe, /tmp/m2-probe-edge-induced, covers the
unfused-root self: store-level map, two-arg insert, root-generic
associated-`Key` trait, `add_subedge(me)` induction; the grammar and
codegen path are neither); the Subgraph pattern exactly as
sketched (the compiled version was simplified to `subnodes` + `main_node`,
and its local root lived in the stdlib — declaring a local root alongside
the pattern is unverified); the inducing `add_subedge` constructor and the
id-based read view it reads through (mechanism probe-verified; Builder
shape, below) as *generated* code; and the bipartite test (M1 machinery
only, but never run). Spike limits, now the
milestone's known frontier, stated in full: embedding depth 1 only;
intrusive arrows sourced at embedded classes or unfused roots rejected;
field backing into an embedded class rejected (its id needs a host path);
arena arrows into an embedded class hosted by another class rejected;
`<=>` transitivity exercised only on two-member classes; constructor
argument name collisions across instances of one fusion class dedup
the argument list, keeping the first — but the dropped member's field
is then un-fed (feeding it from the shared argument needs
cross-instance id-sort unification; leaving it unset is inexpressible),
and spike 1 never exercised a true collision (its field emission would
hit a duplicate field first); and further composition of a composed
pattern untested (the emitted courier has the right shape, but the
member-schema grammar would need to nest).

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
structural validation checks (below) run at the chain's final hop — the
outermost proc-macro layer. Spans there are honest only for the fusion
tokens, which the user wrote in `compose_pattern!`; member-schema tokens
have been re-spanned once per hop (epic, Cross-cutting risks), so
compose-time errors quote the offending tokens in the message text rather
than leaning on spans. The design's central ergonomic claim is
spike-verified:
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
  instance's `root`-marked entity is colocation-fused with an *element
  entity* (a non-root member — epic glossary) of the host instance; an
  instance whose root is unfused keeps its arenas at the store's root
  level. Reference fusion never affects placement. The root-marked
  entity owns no arena, flat or composed (M1's naming rule): its
  "record" is the store itself (an unfused root — it takes part in no
  `<=>` fusion, so there is exactly one self per store and no id names
  it) or the host class's fused record (a colocated root — one self per
  host record, discriminated by the host's id), and instances
  contribute arenas for their non-root entities only. In the
  `Hierarchy` example: `inner`'s
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
  likewise (`OuterOutEdgesIter`). A colocation-fusion class joining
  *differently* named members — the common case, `outer.Node <=>
  sub.Subgraph` — is named by its first element member (non-root member)
  in the composition's instance order: the class's type, id, record, and
  constructor all take that member's name (spike 1's rule; `OuterNode`,
  `add_outer_node`) — its *post-collision* name, since
  instance-prefixing applies before class naming (`OuterNode` is
  outer.Node's prefixed form; spike 1's codegen prefixes from
  cross-instance declaration count, then names the class from the
  canonical member). Write-op names otherwise derive from the
  instantiation's local names (M1) and take the instance prefix on
  collision. Merged constructors take the union of every member's total
  `one`-arrow arguments in that same order — composition-instance order,
  then each member's schema order (argument-name collisions across
  instances of one fusion class silently dedup, keeping the first:
  frontier). Counts match placement: a root-level entity gets a global
  `*_count()`; an embedded entity's count is per-host
  (`sub_node_count(host: OuterNodeId)` — spike 1's shape). The store
  entry point for a composed pattern with several unfused roots is one
  instance-prefixed accessor per unfused root entity
  (`outer_graph()`, `inner_graph()`).
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
  level; at runtime each outer node owns its own embedded copy of `sub`'s
  arenas, as fields on its record. That embedded arena set is all a
  "subgraph store" ever is — `Vec<Data>` fields on the host record, never
  a nested `Store` struct: handles keep one root-store reference
  throughout (invariant 4), which is what makes index-path identity
  sound. M4's "nested store" language means exactly this.
- Backings bind per instance: `implement!` on a composed pattern keys the
  backing block by `instance.arrow` (`outer.out_edges:`,
  `sub.subnodes:`). Two instances of the same pattern may choose different
  backings. (Spike 1's tests abbreviated whole instances with `outer.*`
  sugar; that is test-local shorthand, not grammar — the planned tests
  enumerate bindings.)

### Open entities

`pattern!` gains the ability to declare entities it does not own, to be
closed by reference fusion at composition time. `Subgraph` is defined
*over* a main graph without naming it. Open entities may only be arrow
*targets* within their declaring pattern (closing by `<->` merges no
records, so a sourced arrow's field would have no home); open-as-source
with colocation closure and constructor merging is deferred. Per the
trait-identity invariant, every arrow instantiates a root arrow pattern;
`Subgraph`'s novel arrows (`main_node`, `subnode`, …) are declared as
their own single-arrow roots. In principle neither placement needs
privilege — the stdlib, or alongside the pattern — since local roots
register themselves through the same courier mechanism (M1). What the
spike compiled is the stdlib placement; declaring a local root
*alongside* the pattern is unverified (frontier), and spike 1's codegen
resolved root paths by last segment, papering over path prefixes — a
shortcut the real resolver must not inherit:

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

New cardinality: keyed partial, spelled `key(K)` — a partial lookup
parameterized by an entity of another sort. This is the first relation
whose method takes an argument beyond `&self`; M1's root-trait grammar
reserves the parameter slot for it. The keyed cardinality is declared on
the root, which owns the method and its key parameter; the instantiation
supplies only the key-entity binding (`key(MainNode)`), a local clause
like the instantiation-side `by` — it never restates the cardinality
(`by` never appears on a root, which declares a bare capability;
`key(K)` must, since the root owns the keyed cardinality — see M1's
trait-identity section).

The root side, sketched (`SubNodeOf` is the stdlib candidate; `SubEdgeOf`
has the same shape):

```rust
pattern! {
    SubNodeOf {
        entities: root Subgraph, SubNode, MainNode;
        arrows { subnode: Subgraph -> SubNode, key(MainNode); }
    }
}
```

The root's key entity is an ordinary entity of the root pattern,
rebound per instantiation exactly as source and target are. The root
cannot name the instantiation's key sort, so its trait carries the key
as an associated type (probe-verified, /tmp/m2-probe-edge-induced):

```rust
pub trait Subnode<M> {
    type SubNode;
    type Key;
    fn subnode(&self, key: Self::Key) -> Option<Self::SubNode>;
}
```

One emission rule, pinned by a follow-up probe (/tmp/m2-probe-e0637;
the edge-induced probe pins the positive form): associated-type
bindings in the generated impls must name the impl lifetime
(`type SubNode = SubNode<'s>` inside `impl<'s>`; `'_` there is E0637). The
registration-courier metadata grows the keyed variant accordingly —
beyond (method name, GAT name, associated type names, cardinality), a
keyed root publishes the key's associated type name, so codegen types
the parameter by resolving the instantiation's `key(LocalEntity)`
binding (M1, registration courier).

New built-in backing: `hash_index` — per-self-object map from key id to
target id, maintained by the builder's insert hooks. It is the first
backing whose hooks maintain *derived lookup state* (intrusive_list's
hooks already mutate records; hash_index's maintain a secondary index),
and it validates the hook interface designed in M1. Keyed arrows bind
explicitly, like `many` arrows; `hash_index` is the only keyed-legal
backing in the fixed table, and M1's wrong-cardinality validation covers
both directions (`hash_index` on a non-keyed arrow, a keyed arrow bound
to anything else).

Two configurations, both verified by hand-written emission:

- *Embedded target* — Hierarchy with `sub.subnode` bound, the map's
  values host-anchored path ids (spike 11, 5 tests): path ids derive
  `Copy`/`Eq`/`Hash` unchanged and round-trip as map values; the map
  lives on the host record next to the embedded arena; the insert hook
  takes `&mut` *store* state, which M1's hook contract already requires
  (the arena append and the map insert cannot share one `&mut`
  host-record borrow); `<->` collapses the key type to the partner's
  plain id (`InnerNodeId`, no wrapper newtype).
- *Unfused-root self* — the edge-induced composition (Tests, below),
  the arrow sourced at `sub`'s unfused root (review probe, 5 tests):
  the self owns no record or id, so the map is a store-level field —
  the contribution site M1's backing contract has always allowed — and
  the insert op omits the self argument: `insert_subnode(key, target)`.
  Write ops for arrows sourced at an unfused root follow this rule
  generally: there is exactly one self, and it has no id to pass. The
  colocated-root self is the host shape's mirror: one self per host
  record, discriminated by the host's id — the keyed insert takes the
  host argument and the map sits on the host record (spike 11's
  `insert_subnode_hook(&mut Store, host, main, sub)`), while a non-root
  self passes its own id, as always.

What remains unverified is the macro side: the keyed grammar and its
codegen. The Hierarchy keyed bindings are un-gated; the edge-induced
test never needed the gate (its `sub` root is unfused → flat SubNode
ids).

**Duplicate keys are rejected where the op is submitted.** A keyed arrow
is a function — but the plain `HashMap` emission silently overwrites on
a live key: last write wins, the old target is orphaned in its arena,
and `validate()` cannot see it (the probe reproduced exactly this
failure). So the insert hook checks the map first, the builder method
panics on a live key, and M4's transaction does the same at the staging
call — checked against the borrowed old store and the already-staged
inserts, a staging panic like a foreign ref, deterministically
detectable where the op is submitted and never a flush error. Uniqueness
thereafter holds by construction. How entries fare across removal and
flush — the key side as a reference, the target side as membership,
maps carried and never rebuilt — is recorded with M4's cascade policy
as completion points (M4, Removal).

Runtime checks attach to the **`validate()` boundary** M1 establishes:
key existence (every key id names a real record) and value-in-arena
(every mapped target id is a live record), joined by the per-backing
structural checks (Structural validation). In M2 there is no flush, so the
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
  generated id-based getters on the store, taking and returning ids and
  copies — never handles: one field-read getter per bound `one` arrow,
  named after the arrow whose field it reads — qualified by the
  read-through arrow when the read crosses an instance boundary (the
  probe's spelling: `main_edge_source(me)`, reading through the partner
  map where the arrow crosses an open entity) — and one keyed lookup
  per keyed arrow (`subnode_by_key(key) -> Option<target id>`); getter
  names take the instance prefix on collision, like every other
  generated name (the collision rule under Naming above). This is
  user-callable surface — the inducing constructor is user code — so
  the getter names
  are contract, not convenience. Handles are rejected outright: during
  construction there is exactly one store, being read and mutated at
  once, so no handle — whole-store or restricted — can coexist with
  the `&mut` builder (the borrow checker refuses exactly the coexistence
  induction needs; a read-only handle over a *different* store becomes
  viable in M4, whose flush reads the old store while building the new
  one). M4's flush replay reads through the same view (copied old data
  plus already-replayed ops). Uncompiled as generated code; the
  mechanism is probe-verified (/tmp/m2-probe-edge-induced).

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

Two run at `pattern!` time (each needs only the declaring pattern):

- Open entities are arrow *targets* only in their declaring pattern.
- No arrow targets the root-marked entity: it owns no id type to store
  (M1's naming rule). Before this rule existed the failure surfaced as
  a generated-code compile error (spike 10 observed it); as a check it
  gets a real message quoting the offending tokens. Arrows *sourced* at
  the root are fine — that is what the root accessor's handle is for.

Five run at `compose_pattern!` time — all member schemas and fusions are
in hand at the final hop, which the courier chain guarantees. The set is
spike 1's verified final-hop checklist (its root-per-instance item moved
to `pattern!` time, where M1's grammar enforces it) plus spike 4's
classes-closed check:

- Fusion endpoints exist: every `instance.Entity` names a real member.
  (Grammar validation, but it belongs here because only the final hop
  sees all member schemas.)
- `<->` pairs exactly one open entity with one closed entity, and
  neither endpoint is root-marked (a root owns no id type to unify);
  `<=>` never touches open entities.
- Open entities are closed by exactly one `<->`. Double reference
  closure (`a.X <-> b.N` *and* `a.X <-> c.N`) would unify `b.N`'s and
  `c.N`'s ids through one open entity and is rejected.
- Fusion classes must be closed: no open entity survives composition.
- Every colocation-fusion class has exactly one arena placement, and
  its element members come from at most one instance. This rejects
  double-closed entities (`a.X <=> b.N` *and* `a.X <=> c.N`, where
  transitivity merges arenas from two instances), cross-multiplicity
  colocations (fusing an entity whose arena embeds per-record with one
  whose arena lives at root: each asks one id type to index two
  arenas), two root-marked entities in one class, and peer-shaped
  cross-instance colocation (`a.X <=> c.Y`, elements of two instances
  — spike 1's message: "colocation class fuses elements of multiple
  instances"). The host shape — one instance's element hosting other
  instances' roots, `outer.Node <=> sub.Subgraph` — is the legal
  cross-instance form.

Two run at `implement!` time, because they need backing bindings:

- No record may contain itself without intervening indirection. (Moved
  from compose time, where the spike found it *vacuous*: containment
  edges come from colocation placement — always indirect, since arenas
  are `Vec`-backed — and from backing contributions that embed records
  inline; id-typed fields count as indirection. With only the M1–M3
  built-ins every edge is indirect, so the check can never fire without
  per-backing contribution typing (inline vs id), which is
  implement!-time information.)
- Field contributions must have a home after placement, and the home
  must be id-addressable (a `field`-backing rule — below). One
  `implement!` call generates **one root store**;
  every instance's arenas live in that store's tree (root-level, or
  embedded per host record by the placement rule), so for contributions
  whose self record exists the home half is vacuous in M2, exactly as
  spike 4 pinned ("every arena is in the one store, so every
  contribution trivially has a home"); like the no-self-containment
  check's vacuity above, the boundary is kept for the layouts that make
  it bite. The home half fires exactly when the would-be home does not
  exist — a contribution whose self owns no record — which houses the
  frontier's "intrusive arrows sourced at … unfused roots" rejection
  (the embedded-class half of that frontier item stays a codegen
  frontier: the record exists, so it is not a home failure). The live
  half is addressability, and it is a `field`-backing rule: a
  `field`-backed arrow whose value would reference an embedded class is
  rejected, because the embedded class's ids are host-anchored paths
  the constructor argument cannot supply — this is where the frontier
  rejection "field backing into an embedded class (its id needs a host
  path)" lives. Hook-maintained contributions are exempt: hash_index's
  map is written by its insert hook at a known host, not by constructor
  arguments, and its path-id values are verified legal (spike 11) — the
  Hierarchy keyed bindings stay un-gated. Its errors quote the
  offending tokens — spans point at the `implement!` call site.
  Multi-store layouts (e.g. a separately-owned shared inner store)
  are out of scope: making the placement half non-vacuous there requires
  vocabulary the schema does not have — a store partition (which fusion
  class lives in which store), a directional store-reachability
  relation, and the emitting store per binding (derivable from the
  self-record's class). That vocabulary belongs to multi-store layouts,
  which remain out of scope; record it as the prerequisite when they
  arrive.

Two kinds of check run at `validate()` time (runtime; caller-invoked in
M2, forced by M4's flush). This is the canonical inventory — M4's flush
runs exactly these:

- **Key checks** (keyed partial arrows): key existence (every key id
  names a real record) and value-in-arena (every mapped target id is a
  live record). Key uniqueness per self object is not a runtime check:
  a keyed arrow is a function because inserts onto a live key are
  rejected at the call (Keyed partial arrows).
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

- Edge-induced subgraph — the milestone's goal test. The composition is
  two instances, reference closure only:

  ```rust
  compose_pattern! {
      EdgeInduced {
          instances: main: Digraph, sub: Subgraph;
          sub.MainNode <-> main.Node;
          sub.MainEdge <-> main.Edge;
      }
  }
  ```

  One `implement!` call generates one root store holding the main graph
  and exactly one subgraph over it: `sub`'s root is unfused, so `sub`'s
  arenas sit at the store's root level and every SubNode/SubEdge
  belongs to that single subgraph — the root itself owns no arena,
  record, or id under the naming rule, whatever its fusion (bindings:
  `main`'s arrows as in M1's
  test; `sub.subnodes`/`sub.subedges: arena`; `sub.subnode`/
  `sub.subedge: hash_index`). Runtime multiplicity — many subgraphs over
  one main graph — is the Hierarchy test's hosted shape, not this one.
  Build the main graph, populate the subgraph through the builder, call
  `validate()`, check `subnode`/`main_node` round-trips, and check that
  conditional lookups return `None` for non-members. The builder's
  inducing `add_subedge(me)` mirrors one main edge as a subedge and
  induces subnodes for its endpoints, deduplicated via the hash index;
  induction is constructor logic, not schema vocabulary (see M4), and
  the inducing constructor reads the endpoints through the builder's
  id-based read view (Builder shape, above) — the mechanism is
  probe-verified end to end (/tmp/m2-probe-edge-induced); the generated
  code is uncompiled.
- Hierarchy: outer graph whose nodes contain subgraphs of the inner
  graph; traverse both levels. The test binds the keyed arrows —
  `sub.subnode: hash_index`, `sub.subedge: hash_index`, the spike-11
  configuration — alongside `sub.subnodes`/`sub.subedges: arena`, with
  `outer`/`inner` bound as in M1's test.
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

- The single-arrow roots the Subgraph pattern leans on —
  `MainNode`/`MainEdge` (`one`), `EdgesOf` (`many`; the arena binding
  requires it), `SubNodeOf`/`SubEdgeOf` (keyed) — are not yet sketched.
  Naming decided: a root pattern takes its arrow's name with no suffix
  where the name is free (`MainNode`/`MainEdge` — their entity
  namesakes are open and emit nothing; spike 1's `MainNodeOf` was a
  deviation), and the `-Of` suffix stays where the bare name is taken
  (`SubNodeOf`, `SubEdgeOf` beside the entities; `EdgesOf` beside the
  stdlib root). The trait is arrow-named regardless (M1's trait
  identity rule), so `SubNodeOf` owns `Subnode<M>`.
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
