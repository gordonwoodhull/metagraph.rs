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
intrusive arrows sourced at embedded classes rejected (M3 reclassifies:
a uniform contributed-field-location restriction, not an
intrusive-specific rule); intrusive arrows sourced at unfused roots
legal — the unfused root's record is the store, so the head field is
store-level and the insert op drops the self argument (M3's placement
rules; write-ops rule below) — probe-verified
(/tmp/m3-probe-r3-root-intrusive, 5/5), with the landed M2 check still
rejecting the arm until M3's intrusive step relaxes it; field backing
into an embedded class rejected (its id needs a host path — the
addressability half, implement-time checks below; hook-maintained
path-id values are legal, spike 11); arena arrows into an embedded
class hosted by another class rejected (M3: this is arena's declared
`hosts_arena` requirement);
`<=>` transitivity is vacuous beyond two members *by design* — every
legal colocation class has exactly one element member (Structural
validation pins this; the spike exercised two-member classes only);
constructor argument name collisions across instances of one fusion
class share one argument — feeding both members' fields from it —
when the two arguments' resolved target classes agree (the partner map
usually guarantees this), and take the instance prefix when they
differ (combined rule, reconciling round 3's dedup-keeps-first with
M3 review 2's instance-prefixing, 2026-10-11; the earlier "un-fed
field" frontier stays closed either way: Rust record literals cannot
leave a field unset, and both halves name where every field's value
comes from, so the compose-time rejection clause disappears). The
sharing half is plan-pinned, not spike-verified (spike 1 never
exercised a true collision — its field emission would hit a duplicate
field first); the prefixing half is probe-verified for the
targets-differ case (/tmp/m3-probe-ctor-merge, 2/2 + E0063 negcheck);
a combined-rule probe — one fused record, one shared-arg constructor
(targets agree via `<->`), one prefixed constructor (targets differ),
asserting field routing — is queued in plans/spikes.md; further composition of a
composed pattern untested (the emitted courier has the right shape,
but the member-schema grammar would need to nest); sum (`:=`)
sibling sets whose targets resolve to different classes after fusion
are unchecked — a generated-code compile error, not a validation
message (no planned test composes a cross-instance sum);
cross-crate composition of a pattern with crate-local roots requires
the implementing crate to depend directly on every member pattern's
declaring crate (the declaring-crate resolver's one constraint —
renames handled, re-export routing rejected; Open entities).

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
`pub use` shape, embedding the full composed schema (instances, fusions;
the per-instance markers are minted at emission and their paths derived
from the composed pattern's path by the M1 rule, not embedded — the
spike's shape, and what makes `implement!` resolution
configuration-free). A composed pattern is then indistinguishable from a
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
name — the spike's `mod` payload is the snake-case module name derived
from the composed pattern's name, not a path — remaining-courier queue,
fusion declarations, schemas collected so far) rides verbatim in the
courier's rest tail, with fusions staying opaque token soup until the
final hop; the *composed* schema embedded in the final courier has its
own structured grammar (`instances`, per-member schemas, `fusions`),
which `__implement_gen!` distinguishes from a flat schema by first
keyword; and member schemas re-emit their `root`/`(open)` entity markers
so the placement rule and closure checks read them after the hops.
Review 4 addition: each member schema rides with its declaring package
as `("pkg", {schema})` in the collected set — the root-resolver
constraint (Open entities: the implementing crate must depend directly
on every member's declaring crate) is only enforceable if each member's
baked package survives the hops, and implement-time root-path resolution
is keyed by member instance. Neither spike exercised this (spike 1's
ComposedSchema carried no pkg; the root-resolver probe carried one
schema, not per-member schemas) — first-compile at implementation time,
and the milestone's highest-risk unverified codegen.

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
  instances of one fusion class share one argument feeding both
  members' fields when the two arguments' resolved target classes
  agree — the partner map usually guarantees this — and take the
  instance prefix when they differ; combined rule, round 3 reconciled
  with M3 review 2, 2026-10-11: sharing half plan-pinned, prefixing
  half probe-verified for the targets-differ case,
  /tmp/m3-probe-ctor-merge). Counts match placement: a root-level entity gets a global
  `*_count()`; an embedded entity's count is per-host
  (`sub_node_count(host: OuterNodeId)` — spike 1's shape). Iterators
  match placement too: M1's arena iterator is a single global index
  range, so iteration over an embedded class gains a host parameter
  and walks that host record's embedded arena, yielding nested
  handles (root store reference plus index path) — this is the
  permissive reading that keeps the Hierarchy test's
  `sub.subnodes: arena` / `sub.subedges: arena` bindings legal: the
  arrows are sourced at `sub`'s colocated root, so the embedded arena
  is reachable through the same index path the self record occupies
  (the frontier's "arena arrows into an embedded class hosted by
  another class" rejection targets exactly the other shape — an arena
  arrow whose self is *not* the host). The store
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
  enumerate bindings.) This extends M1's binding grammar: M1's
  `Binding.arrow` is a bare `Ident` (`parse_bindings`), and the
  duplicate/unknown-arrow checks compare `Ident` equality. The shared
  grammar module gains the qualified `instance.arrow` form (dotted pair
  of idents), the duplicate/unknown checks compare qualified keys, and
  the module's round-trip test grows accordingly. The flat bare-`Ident`
  form stays legal and means what it meant in M1.

### Open entities

Patterns may declare entities they do not own, to be closed by reference
fusion at composition time — the *declaration* machinery shipped in M1
(`(open)` parses, rides the shared schema grammar into the courier,
open-as-source is rejected at `pattern!` time, and open entities are
rejected at `implement!` time with a message naming `compose_pattern!`).
M2 adds the closure semantics. `Subgraph` is defined
*over* a main graph without naming it. Open entities may only be arrow
*targets* within their declaring pattern (closing by `<->` merges no
records, so a sourced arrow's field would have no home); open-as-source
with colocation closure and constructor merging is deferred. Per the
trait-identity invariant, every arrow instantiates a root arrow pattern;
`Subgraph`'s novel arrows (`main_node`, `subnode`, …) are declared as
their own single-arrow roots. **Root placement decision: the four roots
are declared *alongside* the pattern — local roots are the goal, not the
stdlib fallback.** That goal requires a resolver M1 does not have: M1's
`resolve_schema_path` prefixes every non-`crate::`-rooted instantiation
path with the *runtime* crate root, so the sketched bare `MainNode` /
`SubNodeOf` spellings resolve to `::metagraph_exper::MainNode` at the
`implement!` site and fail outside the runtime crate, while
`crate::`-rooted spellings pass verbatim but bind `crate` at the
implement! site — correct only when pattern, compose, and implement all
share one crate (and `tests/` integration tests are a separate crate
from `src/`). A proc macro cannot ask what crate it is expanding in, so
M2 begins with the **declaring-crate resolver**, spike-verified
(/tmp/m2-probe-root-resolver, six-crate workspace, green): `pattern!`
reads `CARGO_PKG_NAME` from its process environment at expansion time —
Cargo sets it for the crate being compiled, and the proc macro runs in
that crate's rustc process — and bakes the declaring crate's package
name into the courier schema as a string literal; the final hop maps it
to the implementing crate's local name with the `proc-macro-crate` scan
M1 already uses for the runtime root (`Itself` → `crate::…`,
`Name(n)` → `::n::…`, renames handled; missing dependency fails with a
clean `compile_error!`). The `$crate`-hygiene alternative also works but
only under verbatim token splicing — re-parsing the substituted path as
`syn::Path` is untested, and the composed-schema grammar *does* parse
paths, so env+`proc_macro_crate` is the mechanism. Spike-pinned
constraints: the schema must ride the courier hops as **one group token
tree** (never re-tokenized); the **implementing crate must depend
directly on every member pattern's declaring crate** (renames fine;
composition through a re-exporting intermediate is not routed — the
re-export chain would have to be recorded in the schema, and isn't);
courier transcribers spell the forwarding target `$($fwd:tt)::+` (a
`path` fragment can't be followed by `!`), nested macro invocations are
brace-delimited for item position, a `macro_export` courier is invoked
by textual name inside its own crate (`crate::name!` is
future-incompat deny), and courier bodies containing `$` metavariables
are built as source text via `from_str`, never `quote!`. Spike 1's
codegen resolved root paths by last segment, papering over path
prefixes — a shortcut this resolver replaces:

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
whose method takes an argument beyond `&self`; M1's registration-courier
metadata already reserves the slot for it (the fifth tuple element,
parsed as `key` and always `-` in M1 — the reservation is in the
*metadata*, not in the trait grammar, which cannot yet declare a keyed
root). The keyed cardinality travels as a third value in the metadata's
cardinality slot (`keyed`, beside `one`/`many`), with the key's
associated-type name in the key slot. The keyed cardinality is declared on
the root, which owns the method and its key parameter; the instantiation
supplies only the key-entity binding (`key(MainNode)`), a local clause
like the instantiation-side `by` — it never restates the cardinality
(`by` never appears on a root, which declares a bare capability;
`key(K)` must, since the root owns the keyed cardinality — see M1's
trait-identity section). `key(K)` does not combine with `by` or with
`:= sum` (keyed arrows bind explicitly and are never derived), and an
unbound keyed arrow draws the same missing-binding error as an unbound
`many` arrow.

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
backing in the fixed table. M1's wrong-cardinality validation covers
both directions *for M1's three backings* (`field` on `many`,
`intrusive_list` on `one` — trybuild fixtures exist for both); the
keyed directions are new match arms in the same check — `hash_index`
on a non-keyed arrow, a keyed arrow bound to anything else — plus a
new unbound-keyed-arrow error path.

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
  the self owns no record or id, so the map is a store-level field.
  The M1 *plan*'s backing contract allowed a store-level contribution
  site, but M1's *code* implements only self-record and target-record
  contributions (`field_decls` is keyed by entity; the store struct's
  fields are exactly the per-entity arenas) — the store-level site is
  new M2 codegen machinery, not reuse of an existing one. The insert
  op omits the self argument: `insert_subnode(key, target)`.
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
panics on a live key (the panic's emission is unverified — the probe
documents last-write-wins, and the staged-key probe covers only M4's
staging API; the `#[should_panic]` acceptance test in Tests below is
the pin), and M4's transaction does the same at the staging
call — checked against the borrowed old store and the already-staged
inserts, a staging panic like a foreign ref, deterministically
detectable where the op is submitted and never a flush error. Uniqueness
thereafter holds by construction. How entries fare across removal and
flush — the key side as a reference, the target side as membership,
maps carried and never rebuilt — is pinned in M4's cascade policy
(M4, Removal; probe-verified, /tmp/m4-probe-keyed-carry).

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
  these same hooks. Note what M1's *code* actually provides: named hooks
  exist only for `intrusive_list` (the M1 plan's "hooks: constructor
  arg" for `field` was never realized as a function — field-backed
  values move into the record literal inline), and no `set_<one-arrow>`
  op exists anywhere. M2 therefore *grows* the hook surface it claims
  to reuse: field writes move behind named hooks and hash_index gains
  its insert hook, so that every storage mutation is replayable. This
  is a deliberate M1-code change, not just an addition.
- **Builder methods are named and shaped as if they were already
  write-trait methods** (`add_<entity>`, `insert_<arrow>`,
  `set_<one-arrow>`), so M4 can lift them into generated write traits —
  defined from the arrows the same way the read traits are — without
  renaming. The lift keeps two layers distinct: the *trait* methods are
  root-fixed (M4's write-trait identity rule, carried in the courier's
  write slot), while the builder's local spellings survive as the
  transaction's inherent methods — the same alias split the read side
  makes.
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
  generated name (the collision rule under Naming above). The name
  function is pinned by these two examples rather than a general rule —
  review 4 leaves the bare same-instance spelling, and whether flat
  uncomposed stores emit the view at all, as implementer decisions. This is
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

Where M4 is *not* additive, stated plainly so M2 doesn't over-commit:
M2 bakes in append-only identity — dense positional `u32` ids
(`NodeId(pub u32)` is pinned by tests), index-range iterators, and
index-path addressing. M4 removal forces a choice M2 cannot pre-make:
tombstones (every iterator, count, and value-in-arena check grows
liveness filtering — every backing's access protocol changes) or
compaction (every stored id, map key/value, and intrusive link needs
cascade rewriting). Index-path change-set addressing "doubles as" M4's
scheme only for append+mutate. Likewise the duplicate-key rejection:
M2's check lives in the mutating insert hook, but M4's staging needs
the same determination as a pure predicate over (old store, staged
ops) — "deterministically detectable where the op is submitted, never
a flush error" — so the hook should be factored check-then-write from
the start, with the predicate callable without performing the write.
And M4's `set_<one-arrow>` will need per-backing *delete* hooks
(un-linking a `by`-derived intrusive membership), a structural growth
of the hook interface beyond the keyed insert M2 adds.

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

Two run at `pattern!` time (each needs only the declaring pattern).
Both already ship in M1 — the no-arrow-targets-root check with a
pinned trybuild fixture, the open-targets-only check without one — so
M2's work here is the compile-fail fixture for the open-as-source
check, not the checks themselves:

- Open entities are arrow *targets* only in their declaring pattern.
- No arrow targets the root-marked entity: it owns no id type to store
  (M1's naming rule). Before this rule existed the failure surfaced as
  a generated-code compile error (spike 10 observed it); M1 turned it
  into a check with a real message. Arrows *sourced* at
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
- Every colocation-fusion class has exactly one arena placement, and is
  a *docking*: exactly one root-marked entity, with element members
  from exactly one instance *other than* the root's. This rejects
  double-closed entities (`a.X <=> b.N` *and* `a.X <=> c.N`, where
  transitivity merges arenas from two instances), cross-multiplicity
  colocations (fusing an entity whose arena embeds per-record with one
  whose arena lives at root: each asks one id type to index two
  arenas), two root-marked entities in one class, peer-shaped
  cross-instance colocation (`a.X <=> c.Y`, elements of two instances
  — spike 1's message: "colocation class fuses elements of multiple
  instances"), same-instance element-element fusions (`outer.Node <=>
  outer.Edge` — sort-sharing within one instance, admitted by the
  at-most-one-instance reading alone and defined by nothing), and the
  self-hosting regress (an instance's root fused with its own element,
  which would embed the instance's arenas into their own records). The
  host shape — one instance's element hosting other instances' roots,
  `outer.Node <=> sub.Subgraph` — is the legal cross-instance form.
  A consequence, accepted as a design decision (2026-10 review): every
  legal colocation class has exactly two members — one element member
  plus at most one root — because any transitively formed three-member
  class contains either two elements or two roots, both rejected (the
  element pair by the peer-shaped and same-instance arms together,
  which is what makes "exactly one element member" follow rather than
  merely "elements from one instance"). (The docking re-anchoring and
  its two arms landed with the M3 round-4 merge as plan text; the arms'
  code and trybuild fixtures — DISCHARGED with M3's implementation:
  `classes_legal` rejects both shapes, fixtures `compose_coloc_same_instance`
  and `compose_coloc_self_hosting`.) `<=>`
  transitivity is therefore vacuous in practice, and one host element
  can host exactly *one* instance's arenas: two Subgraph instances
  both fused to `outer.Node` over the same inner graph is rejected.
  Runtime multiplicity (each outer node owning its own embedded copy
  of one pattern's arenas) is unaffected; what is rejected is multiple
  *patterns* on one host.
- Generated names are unique: fusion-class names (first element
  member's post-collision name), instance-prefixed entity names,
  arena fields, iterators, read-view getters, and root accessors must
  not collide after all prefixes and class namings are applied. The
  naming rules above pin the tiebreaks for *entity* declarations;
  this check catches what they don't (two fusion classes deriving
  one name; a fused-class name meeting a third instance's prefixed
  entity; read-view getters meeting root accessors or each other).

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
  must be id-addressable (a `field`-backing rule — below). This check
  **subsumes M1's flat-codegen rejection** of `field`/`intrusive_list`
  arrows sourced at the root-marked entity ("the root owns no record"):
  M1's version runs against the raw member schema and would reject the
  Refinement sketch's `elems` arrow, whose self record exists only
  post-fusion (`outer.Edge <=> chain.Chain` gives chain's root the
  fused outer-edge record). The re-scoped rule runs *after*
  fusion-class and host resolution: a root-marked self has a record
  exactly when it is colocation-fused into a host element — and an
  *unfused* root has one too: the store itself (M3's three-case self
  taxonomy), so contributed fields of arrows sourced at an unfused root
  land as store-level fields, the contribution site M1's backing
  contract has always allowed. That lifts the old "intrusive arrows
  sourced at … unfused roots" rejection in plan (probe-verified,
  /tmp/m3-probe-r3-root-intrusive, 5/5 — store-level head, self-less
  op, root-handle read), while the landed check still rejects the arm
  until M3's intrusive step relaxes it; the `field` backing at an
  unfused root stays rejected on the ground M3 pins instead — its
  `ctor_arg` requires an `add_<A>`-orchestrated self, and the unfused
  root owns no constructor.
  A verbatim port of M1's check into composed codegen kills the
  milestone's own Refinement test — the spike solved this in code;
  this paragraph is the plan recording the rule. One
  `implement!` call generates **one root store**;
  every instance's arenas live in that store's tree (root-level, or
  embedded per host record by the placement rule), so for contributions
  whose self record exists the home half is vacuous in M2, exactly as
  spike 4 pinned ("every arena is in the one store, so every
  contribution trivially has a home"); like the no-self-containment
  check's vacuity above, the boundary is kept for the layouts that make
  it bite. With the unfused root's home recognized (the store), the
  home half is vacuous across all of M2's single-store layouts — kept
  for the multi-store shapes that make it bite. The embedded-class
  half of the old frontier item stays a codegen frontier: the record
  exists, so it is not a home failure. The live
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
  names a real record), value-in-arena (every mapped target id is a
  live record), and value-host agreement (an embedded-target map holds
  host-anchored path ids; a value whose host component names a
  *different* host record is in-bounds in its own host's arena, so
  value-in-arena alone passes it — spike 11's `validate` needed this
  third check, and since M4's flush runs exactly this inventory, the
  omission would be load-bearing). Key uniqueness per self object is
  not a runtime check: a keyed arrow is a function because inserts
  onto a live key are rejected at the call (Keyed partial arrows).
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
  id-based read view (Builder shape, above). `add_subedge` is
  **hand-written user code in the test** — an inherent `impl` block on
  the generated store, exactly as /tmp/m2-probe-edge-induced has it —
  not macro-generated; there is no constructor-extension grammar, and
  none is planned before M4. The mechanism is
  probe-verified end to end (/tmp/m2-probe-edge-induced); the generated
  code is uncompiled. With the declaring-crate resolver spike-verified
  (Open entities), this test lives in `tests/` as an integration test;
  Subgraph and its four local roots are declared in a dev-dependency
  fixture crate shared by this test and the Hierarchy test (each
  `tests/*.rs` is its own crate — review 4 pin), and a fully cross-crate
  composed-implement test (M1 step 9's
  precedent, including a renamed-dependency variant) exercises the
  resolver's `Name(n)` arm as the mechanism's acceptance test.
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
- Negative tests — the milestone's largest spec section is validation,
  and positive tests alone leave it untested. Following M1 step 8's
  precedent: trybuild compile-fail fixtures for the structural checks
  (the open-as-source `pattern!`-time check, each of the compose-time
  five — unknown fusion endpoint, `<->` arity/endpoint violations,
  double reference closure, unclosed open entity, placement conflicts —
  the generated-name-uniqueness check, the implement!-time home and
  addressability rejections, and the new keyed-cardinality mismatches;
  the M3 round-4 merge's docking arms — same-instance element-element
  and self-hosting rejections — joined this list and LANDED with M3's
  implementation: `compose_coloc_same_instance` /
  `compose_coloc_self_hosting`);
  a `#[should_panic]` test for duplicate-key rejection at the insert
  call; and `validate()` failure tests driving the canonical inventory —
  ghost keys, out-of-range and cross-host path-id values, corrupted
  intrusive chains hitting both halves of the partition check (shared
  membership vs. cycle, distinguished in the report).

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
  only entities reachable via nearby total `one` arrows. (Both
  review-4 probes keyed on an open entity — `key(MainNode)` — so "any
  entity declared in the pattern" is the minimal reading.)
- Derived arrows are scoped to the instance that declares them;
  cross-instance derivations are out of scope. (The per-instance scoping
  is load-bearing: it keeps `sum`'s sibling rule away from
  fusion-collapsed arrow sets.)

## Depends on

M1 complete: courier mechanics (including the target-parameterized rule
this milestone's composition chain reuses), marker infrastructure,
root registration couriers (the Subgraph sketch's local roots need them),
the insert-hook interface, and the `validate()` boundary. All five are
verified present in merged M1 (3c016e7). M1 additionally shipped three
things earlier drafts of this plan treated as M2 work: the `(open)`
entity declaration machinery, the registration metadata's key slot, and
both `pattern!`-time structural checks. Two things this plan describes
as M1 commitments are M1-*plan* commitments its code does not yet
provide, and are M2 work: named hooks for anything but `intrusive_list`
(field writes currently move into the record literal inline) and the
store-level backing contribution site. M2 is the stress
test of all five; expect the hook interface to grow (keyed insert —
and, per Builder shape, field-write hooks as well).

## Implementation steps (review 4)

Ordered so the declaring-crate resolver precedes any test touching
Subgraph's local roots, the courier chain precedes fusion codegen, the
read view precedes the edge-induced goal test, and negative tests
follow the checks they exercise.

1. **Qualified binding grammar.** `Binding` gains the `instance.arrow`
   dotted-pair form; duplicate/unknown-arrow checks compare qualified
   keys; `bindings_tokens` round-trips; bare-Ident stays legal; the
   `outer.*` sugar is *not* grammar. Acceptance: round-trip unit
   tests; full M1 suite green. The mechanical `union` → `sum` code
   rename (macros/src/grammar.rs, src/stdlib.rs) rides with this
   grammar pass — plan text is renamed throughout, M1's shipped
   grammar spells `union` until then.
2. **Declaring-crate resolver.** `pattern!` bakes `CARGO_PKG_NAME`
   into the courier schema (new grammar slot, folded into step 1's
   grammar work, including the per-member `("pkg", {schema})`
   carriage pinned under Mechanism above); resolution at the
   implement/compose hop via `proc_macro_crate`. Acceptance: the
   probe's matrix recast in-repo — same-crate, direct cross-crate,
   renamed dependency, one compose hop, indirect-dependency trybuild
   failure. Depends: 1. Precedes every test touching Subgraph's local
   roots.
3. **Hook surface growth.** Named field-write hooks in flat codegen;
   check-then-write factorization where M4 needs a pure predicate.
   Acceptance: koenigsberg assertions unchanged and green; expanded
   code shows named hooks.
4. **Keyed arrow grammar (macro side).** Root-side `key(K)` with
   `type Key` and `Option` return, no GAT; instantiation-side
   `key(LocalEntity)`; `keyed` as a third cardinality value;
   pattern-time checks. Acceptance: round-trip tests; a root
   `pattern!` emits the probe's trait shape and compiles.
5. **Composition courier chain.** `compose_pattern!` +
   `__compose_step!` + rest-tail payload grammar; flat-vs-composed
   dispatch by first keyword in `__implement_gen!`; composed courier
   emission with per-instance marker minting as fresh unit structs.
   Acceptance: spike's compose_chain equivalent — three-instance
   collection, diamond markers distinct by TypeId. Depends: 1, 2.
6. **Compose-time structural validation.** The five final-hop checks
   plus generated-name uniqueness and the ctor-argument combined rule.
   Acceptance: per-check pass/fail fixtures plus Hierarchy and
   Refinement schemas as realistic inputs. Depends: 5.
7. **Fusion codegen.** Port the spike's `expand_gen_composed`:
   union-find classes, partner map, per-instance placement, path ids
   and nested handles, instance-prefixed names, merged constructors,
   per-host counts, re-scoped home/addressability checks. Acceptance:
   Hierarchy test with arena bindings (keyed deferred to step 8) —
   two-level traversal, generic bfs, per-host counts. Depends: 5, 6.
   Spike-compiled wholesale; expect drift at the seams against the
   evolved M1 baseline (steps 1, 3), not in the core.
8. **`hash_index` backing + store-level contribution site.** Fourth
   built-in backing; insert hook in both configurations;
   duplicate-key check-then-write panic; keyed read impls with the
   E0637-pinned emission; new cardinality error arms. Acceptance: the
   full Hierarchy test (`hash_index` ×2) and a `#[should_panic]`
   duplicate-key test. Depends: 3, 4, 7. High unverified-codegen
   concentration: every macro-side detail is first-compile here.
9. **Builder read view.** Id-based getters on the store; one keyed
   lookup per keyed arrow; instance-prefix on collision. Acceptance:
   probe's exact spellings exercised by a small induction scenario.
   Depends: 7, 8.
10. **Edge-induced goal test.** Subgraph + local roots in the
    dev-dependency fixture crate; the `EdgeInduced` composition; one
    `implement!`; hand-written inducing `add_subedge`; `validate()`;
    plus the cross-crate composed-implement test with a renamed
    dependency. Acceptance: goal test green in `tests/`. Depends: 2,
    4, 8, 9.
11. **Edge-chain refinement test.** `Chain` pattern; `Refinement`
    composition; `insert_elems(OuterEdgeId, InnerEdgeId)`; chain walk
    as generic `OutEdges<M>` traversal. Acceptance: plan's Refinement
    sketch green with `validate()` Ok. Depends: 6, 7.
12. **`validate()` canonical inventory.** Key existence,
    value-in-arena, value-host agreement; per-backing structural
    checks; the intrusive-ownership partition check. Acceptance:
    `validate()` Ok on every positive-test store from steps 7–11.
    Depends: 7, 8.
13. **Negative tests.** trybuild fixtures for every check from steps
    2, 4, 6, 7, 8; runtime `validate()` failure tests driving the
    canonical inventory. Depends: the checks they exercise.
14. **Bipartite plain-pattern test.** M1 machinery only; documents
    invariant 6's limit. Depends: none; placed late for suite
    hygiene.
15. **Doc pass.** `compose_pattern!`, keyed arrows, marker
    qualification spellings; generated-code doc comments carry
    instance prefixes. Acceptance: M1 step-11 standard.

Unverified-codegen concentration: step 2's per-member package
carriage (highest — no spike coverage, on the goal test's path);
steps 8–9 (high — the plan itself marks the keyed macro side and the
read view uncompiled); step 7 (moderate — spike-compiled design
ported onto an evolved baseline); steps 2, 6, 12 otherwise low
(probe-verified mechanisms, mostly porting).
