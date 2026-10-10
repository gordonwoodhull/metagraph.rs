# Epic: metagraph — a toolkit for constructing higher-order graphs

## Vision

A Rust toolkit where graph *types* are assembled from atomic pieces:

- **`pattern!`** declares an abstract type graph: entities and single-arrow
  relations ("capabilities") with cardinalities (`one`, `many`, keyed
  partial). No storage. Traits are generated from arrows.
- **`compose_pattern!`** glues pattern instances together by declaring
  entity fusions (`<=>`, symmetric and transitive): the fused record carries
  the summed capabilities of every member. This is a pushout over type
  graphs. Optional containment uses ordinary pointers instead of fusion.
- **`implement!`** binds each arrow of a (possibly composed) pattern to a
  *backing* — a storage strategy contributing fields, mutation hooks, and an
  access protocol — and emits the concrete store, handles, and trait impls.

The base directed graph is not built in: it is six single-arrow patterns
composed. Bipartite graphs, hierarchical graphs (node is-also subgraph),
edge-refinement graphs (edge is-also chain), and edge-induced subgraphs are
all the same machinery applied to bigger piles of arrows.

## Design invariants (hold across all milestones)

1. **Composition only ever adds.** No renunciation; unwanted relations are
   never imported because patterns are assembled from single arrows.
2. **Abstract before concrete.** `pattern!` and `compose_pattern!` never
   mention storage. Backings bind at `implement!` time; the same abstract
   pattern can be implemented multiple ways, and generic algorithms work
   across implementations because trait identity comes from the arrow
   pattern, not the store.
3. **Markers disambiguate.** Generated traits are generic over a marker per
   arrow instance. Unqualified method calls resolve when unique; genuine
   ambiguity (two instances of the same arrow on one fused entity) requires
   qualification.
4. **Fusion is colocation.** Is-also means one record, one arena, summed
   fields. The handle is a hand-rolled wide pointer: store reference plus
   index, gaining fields only via dereference into nested stores.
5. **Structural rejection, not semantic laws.** The toolkit rejects
   impossible patterns (fusion cycles without indirection, unclosed open
   entities, true ambiguity) but does not check semantic constraints
   (bipartiteness, acyclicity). Constraints you can't state don't need
   checking — most are expressible by which arrows exist.

## Milestones

| # | Theme | Spec |
|---|-------|------|
| 1 | `pattern!` + `implement!`, base digraph | [full](milestone-1-pattern-and-implement.md) |
| 2 | Composition, fusion, keyed arrows, subgraph pattern | [partial](milestone-2-composition.md) |
| 3 | User-facing backings, stdlib refactor | [vague](milestone-3-backings-and-stdlib.md) |
| 4+ | Selection, mutation, interop — see below | this file |

## Milestone 4+ (purposely vague)

Threads we can see from here but haven't scoped:

- **Automatic backing selection**: backings declare applicability
  constraints and cost hints; `implement!` picks, user overrides by name.
- **Mutation**: stores are append-only through M3. Removal requires
  generation-counter ids, backing removal hooks, and a story for index
  maintenance. Generational arenas are the likely shape.
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
  real spans; keep courier-macro grammars rigid and dumb.
- **Debuggability**: users will need `cargo expand` to read generated code.
  Keep generated code boring: no cleverness, consistent names, doc comments
  carrying the arrow that produced each item.
- **Compile times**: heavy monomorphization of iterators; revisit if
  composition depth grows.
