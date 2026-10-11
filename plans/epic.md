# Epic: metagraph — a toolkit for constructing higher-order graphs

## Vision

A Rust toolkit where graph *types* are assembled from atomic pieces:

- **`pattern!`** declares an abstract type graph: entities and single-arrow
  relations ("capabilities") with cardinalities (`one`, `many`, keyed
  partial). No storage. Traits are generated from arrows.
- **`compose_pattern!`** glues pattern instances together by declaring
  entity fusions: colocation (`<=>`, one record, summed capabilities — a
  pushout over type graphs) and reference (`<->`, open-entity closure by id
  unification). Optional containment uses ordinary pointers instead of
  fusion.
- **`implement!`** binds each arrow of a (possibly composed) pattern to a
  *backing* — a storage strategy contributing fields, mutation hooks, and an
  access protocol — and emits the concrete store, handles, and trait impls.

The base directed graph is not built in: it is assembled from six
single-arrow patterns by inline instantiation in one `pattern!` block —
which *is* the degenerate form of composition (entity unification by
name; M3 verified that `compose_pattern!` cannot re-express it, and why
— see invariant 4). Bipartite graphs, hierarchical graphs (node is-also
subgraph), edge-refinement graphs (edge is-also chain), and edge-induced
subgraphs are all the same machinery applied to bigger piles of arrows.

## Design invariants (hold across all milestones)

1. **Composition only ever adds.** No renunciation; unwanted relations are
   never imported because patterns are assembled from single arrows.
2. **Abstract before concrete.** `pattern!` and `compose_pattern!` never
   mention storage. Backings bind at `implement!` time; the same abstract
   pattern can be implemented multiple ways, and generic algorithms work
   across implementations because trait identity comes from the arrow
   pattern, not the store.
3. **Markers disambiguate.** Generated traits are generic over a marker per
   arrow instance, scoped per composition instance and named by module path
   (`hierarchy::markers::outer::OutEdges`), never by name concatenation.
   Unqualified method calls resolve when unique; on fused entities (two
   instances of the same arrow on one record) qualification is the norm.
4. **Fusion has two flavors.** Colocation fusion (`<=>`) is is-also: one
   record, one arena, summed fields. Reference fusion (`<->`) closes open
   entities across instances: it unifies id types without merging records,
   so an arrow targeting the fused entity stores an ordinary id into the
   other store and runtime multiplicity is unconstrained (many subnodes may
   reference one inner node). Handles are wide pointers: root store
   reference plus an index path into nested arenas, gaining fields only via
   dereference. Colocation classes are therefore *hostings*: at most one
   member instance's root-marked entity plus element entities of one other
   instance (M2's exactly-one-placement check; spike 12). Composition
   juxtaposes whole patterns, nests them inside host records, and closes
   open entities by reference — it never shares one sort across instances.
   Sort-sharing over one flat arena is what inline instantiation within a
   `pattern!` block does; a flat schema cannot be reassembled from
   fragments by composition.
5. **Stores are immutable; construction goes through a builder.** A store
   is append-only under exclusive ownership while it is being built (M1's
   `&mut` constructors are the degenerate builder); once handles exist the
   borrow checker has frozen it, so handle identity by store pointer +
   index path is sound — a borrowed store cannot move or reallocate.
   Mutation never touches a live store (see M4): a write-side interface
   accumulates a change set and flushes it into a new immutable store. No
   generational arenas; removal is rebuild-without.
6. **Structural rejection, not semantic laws.** The toolkit rejects
   impossible patterns (fusion cycles without indirection, unclosed open
   entities, colocation classes with conflicting arena placement, true
   ambiguity) but does not check semantic constraints
   (bipartiteness, acyclicity). Constraints you can't state don't need
   checking — most are expressible by which arrows exist.
7. **Trait identity lives with the arrow pattern.** Arrows in user patterns
   *instantiate* single-arrow patterns from the standard library (or another
   library) rather than declaring fresh traits; a single-arrow `pattern!`
   with no instantiation source is the root that owns its trait. Generic
   algorithms are written against the stdlib arrow's trait and apply to
   every pattern that instantiates it. Without this, algorithms would not
   be portable across patterns, and redefining `Digraph` as a composition
   of single-arrow patterns (M3) would silently change trait identity and
   break them. The stdlib is the shared vocabulary, not a closed set: any
   crate may declare a root single-arrow pattern the way the stdlib does,
   and generic algorithms are portable exactly to the extent that patterns
   agree on roots. **Roots register themselves**: every root pattern emits a
   registration courier publishing its trait metadata (method name, GAT
   iterator name, associated type names, cardinality — plus the key's
   associated type name for a keyed root), and `implement!` resolves any
   root through that courier — there is no fixed table of stdlib arrows, and
   user-declared roots work exactly like stdlib ones.

## Terminology: the four senses of "root"

The word "root" does four distinct jobs across these plans. Context usually
disambiguates, but the milestone specs lean on all four:

- **Root pattern** (trait ownership): a single-arrow `pattern!` with no
  instantiation source. It owns and registers its trait (invariant 7). This
  is "root" in the instantiation DAG — the node with no parent.
- **Root-marked entity** (schema structure): one entity per pattern
  explicitly marked `root` in the entity list (`Graph` in Digraph,
  `Subgraph` in Subgraph). The marking is per-declaration and does not
  propagate through instantiation: an instantiation may bind a root
  pattern's root-marked entity to a non-root local entity. M2's placement
  rule and the store's root accessor read this marking.
- **Root store / root level** (storage layout): the single top-level store
  one `implement!` call generates. Arenas live at root level or embedded in
  host records; handles are a root store reference plus an index path.
- **Crate root** (module namespace): where `#[macro_export]` anchors
  couriers (M1's courier path resolution).

Adjacent to these, one more marking the specs lean on: an **element
entity** is an entity *not* marked `root` — the "host element" of M2's
placement rule and the "element members" of its fusion checks. Element
entities own records and arenas unless open (open entities own nothing
and resolve to their `<->` partner).

Two cautions follow. First, the stdlib declarations exercise the first two
senses in adjacent positions (`pattern! { Nodes { entities: root Graph, … } }`
is a root pattern *and* marks a root entity) — the `root` keyword does not
make a pattern a root pattern; the absence of an instantiation source does.
Second, a composed pattern has no root-marked entity of its own: each member
instance keeps its own marking, unfused ones get root accessors, and what
"the instance's root-marked entity" means for a composed member is undefined
until further composition is on the frontier.

## Milestones

| # | Theme | Spec |
|---|-------|------|
| 1 | `pattern!` + `implement!`, base digraph | [full](milestone-1-pattern-and-implement.md) |
| 2 | Composition, fusion, keyed arrows, subgraph pattern | [partial](milestone-2-composition.md) |
| 3 | User-facing backings, stdlib refactor | [partial](milestone-3-backings-and-stdlib.md) |
| 4 | Write-side builders, mutation by rebuild | [preliminary](milestone-4-builders.md) |
| 5+ | Selection, interop — see below | this file |

Cross-cutting implementability validation (twelve throwaway spikes against
milestones 1–4): [spikes.md](spikes.md) — status, results, and what each
spike folded back into the plans.

## Milestone 4 (summary): write-side builders, mutation by rebuild

Stores are immutable once built (invariant 5). M4 generates the write
side from the schema the same way as the read side: a transaction stages
changes against a borrowed store (old handles keep working), and flush
applies them to a rebuilt store through the construction hooks — removals
first, then constructive ops in submission order — running induction and
referential-integrity validation on the way. Removal is rebuild-without;
no generational arenas. Full sketch:
[milestone-4-builders.md](milestone-4-builders.md).

## Milestone 5+ (purposely vague)

Threads we can see from here but haven't scoped:

- **Automatic backing selection**: backings declare applicability
  constraints and cost hints; `implement!` picks, user overrides by name.
- **Interop backend**: implement the six base arrows over petgraph (or a
  remote graph) to prove the traits are backend-portable, not arena-specific.
- **Derived arrows beyond `sum`**: inverse of a total arrow, transitive
  closure, composition of arrows (paths as first-class derived relations).
- **Introspection**: schema reflection at compile time (entity/arrow lists
  as associated types) for codegen of serializers, visualizers, fuzzers.
- **Payloads**: node/edge weights as `one` arrows into value arenas —
  expected to fall out of the existing machinery, worth a milestone's test.

## Cross-cutting risks

- **Macro error messages** are the UX weak point of the whole design.
  Mitigation: parse and validate at the outermost proc-macro layer with
  real spans; courier macros carry a single token-forwarding rule so all
  grammar errors come from the proc macro. Compile-fail behavior
  (impossible bindings, impossible patterns) is tested with `trybuild`
  from M1 onward — validation that isn't tested doesn't exist.
- **Schema-token spans degrade through the courier — repeatedly.** User
  tokens keep their spans end to end; schema tokens embedded in the courier
  are re-spanned to the `implement!` call site, and the degradation
  compounds across hops: composition adds a bounce per member pattern, and
  root-metadata resolution adds one per arrow. Schema-side errors must
  quote the offending tokens in the message text rather than relying on
  spans — this is mandatory, not optional — and trybuild `.stderr` files
  will carry expansion-chain notes (pin them per toolchain).
- **Debuggability**: users will need `cargo expand` to read generated code.
  Keep generated code boring: no cleverness, consistent names, doc comments
  carrying the arrow that produced each item.
- **Compile times**: heavy monomorphization of iterators; revisit if
  composition depth grows.
