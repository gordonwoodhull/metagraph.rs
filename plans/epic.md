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
single-arrow patterns (instantiated inline in M1, re-expressed as a
composition in M3). Bipartite graphs, hierarchical graphs (node is-also
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
   dereference.
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
   registration courier publishing its trait metadata (method name,
   associated type names, cardinality), and `implement!` resolves any root
   through that courier — there is no fixed table of stdlib arrows, and
   user-declared roots work exactly like stdlib ones.

## Milestones

| # | Theme | Spec |
|---|-------|------|
| 1 | `pattern!` + `implement!`, base digraph | [full](milestone-1-pattern-and-implement.md) |
| 2 | Composition, fusion, keyed arrows, subgraph pattern | [partial](milestone-2-composition.md) |
| 3 | User-facing backings, stdlib refactor | [vague](milestone-3-backings-and-stdlib.md) |
| 4 | Write-side builders, mutation by rebuild | [preliminary](milestone-4-builders.md) |
| 5+ | Selection, interop — see below | this file |

Cross-cutting implementability validation (nine throwaway spikes against
milestones 2–4): [spikes.md](spikes.md) — status, results, and what each
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
- **Derived arrows beyond union**: inverse of a total arrow, transitive
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
