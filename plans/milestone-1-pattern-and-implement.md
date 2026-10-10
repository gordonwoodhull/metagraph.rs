# Milestone 1: `pattern!` + `implement!` for the base digraph

**Goal**: the hand-written `src/arena.rs` is replaced by macro-generated
code with materially identical behavior and API shape. The Königsberg test
suite passes with edits confined to imports and graph construction — every
assertion unchanged.

Explicitly out of scope: `compose_pattern!`, open entities, keyed partial
arrows, user-defined backings, backing auto-selection.

## Architecture: the three-layer macro pipeline

Proc macros cannot read other items' tokens, so schema information travels
between macros only through generated code. The pipeline:

1. **`pattern!`** (proc macro, parses + validates the schema DSL) emits:
   - for a root single-arrow pattern: the marker-generic trait it owns
     (`pub trait OutEdges<M> { ... }`), with a GAT iterator for `many`
     arrows, plain associated type for `one`;
   - one unit marker struct per arrow instantiation (per use, not per
     root — two instantiations of `stdlib::Nodes` get distinct markers);
   - a doc-hidden courier `macro_rules!` (`__metagraph_pattern_Digraph!`)
     embedding the schema as tokens in a rigid grammar. Invoked with a store
     name and backing assignments, it prepends the schema and forwards
     everything to `__implement_gen!`.
2. **`implement!`** (proc macro, thin dispatcher): parses just enough to
   find the pattern name, then emits an invocation of that pattern's courier
   with the user's store name and backing block. It does no codegen itself.
3. **`__implement_gen!`** (doc-hidden proc macro): receives the full schema
   tokens + user tokens, parses both, validates the backing binding, and
   emits the concrete store (all codegen lives here, in real Rust, not
   tt-munching).

The courier `macro_rules!` never interprets anything; it is a dumb token
courier with exactly one grammar rule (`($($t:tt)*)` forwarding), and it
stays that way: all grammar lives in `__implement_gen!`'s parser, where
errors get real proc-macro spans. M2's fusion-class tokens travel inside
the schema token channel; the courier never gains rules.

**Courier path resolution is a cross-crate requirement, not a detail.**
The stdlib's patterns are defined in the runtime crate but `implement!` is
invoked in *user* crates, so the courier must be addressable by path. The
mechanics:

- Plain `macro_rules!` + `pub use` is not re-exportable cross-crate
  (E0364). The courier needs `#[macro_export]`, which anchors it at the
  *defining crate's root* macro namespace — so pattern names must be
  unique crate-wide. `pattern!` also emits `#[doc(hidden)]
  #[allow(unused_imports)] pub use __metagraph_pattern_Digraph;`
  (textually after the macro) in the pattern's module, making the courier
  addressable as `metagraph_exper::stdlib::__metagraph_pattern_Digraph`.
- The runtime crate root carries `extern crate self as metagraph_exper;`
  so generated absolute paths resolve inside the runtime crate itself
  (the stdlib is the first consumer; E0433 otherwise). Proc-macro output
  cannot use `$crate`, so users who rename the dependency need a
  documented escape hatch: resolve via `proc-macro-crate`, plus a
  `crate = ...` override in `implement!`.
- The runtime crate re-exports `#[doc(hidden)] pub use
  metagraph_exper_macros::__implement_gen;` — proc macros are not
  reachable through transitive dependencies.
- `implement!` takes the pattern as a path whose last segment must be a
  plain ident (reject generics with a targeted error) and pastes the
  courier name alongside it. A single-segment path requires the courier
  to be in scope; detect that case and emit "import the pattern's
  module" rather than leaking the mangled courier name.

This must work in M1 — the stdlib is itself the first cross-module
consumer.

## Trait identity

Trait identity belongs to single-arrow patterns, not to the pattern that
uses the arrow. The stdlib declares six single-arrow patterns, each the
permanent owner of its trait (`Nodes`, `OutEdges`, `InEdges`, `Source`,
`Target`, `Edges`). Multi-arrow `pattern!` blocks *instantiate* stdlib
arrows with local entity bindings — they do not declare fresh traits. This
keeps generic algorithms portable across every pattern that instantiates a
stdlib arrow, and makes the M3 refactor (Digraph as composition of
single-arrow patterns) a non-event.

A single-arrow `pattern!` with no instantiation source is the root that
declares a fresh trait; users may define their own root arrows the same
way the stdlib does.

Markers are scoped per pattern instance and re-minted by
`compose_pattern!` per composition instance: fusing two instances of one
pattern yields distinct markers, so diamond instantiation produces two
impls under two markers, never a conflicting impl. M1's codegen defines
the marker namespace that `compose_pattern!` inherits, so the rule lives
here.

## `pattern!` syntax (M1 subset)

```rust
// stdlib: the root arrow patterns (each owns its trait)
pattern! { Nodes    { entities: G, N; arrows { nodes:     G -> N, many; } } }
pattern! { OutEdges { entities: N, E; arrows { out_edges: N -> E, many; } } }
// ... InEdges, Source, Target, Edges

// user side: instantiate stdlib arrows, unifying entities by name
pattern! {
    Digraph {
        entities: G, N, E;
        arrows {
            nodes:     stdlib::Nodes,    G -> N;
            out_edges: stdlib::OutEdges, N -> E;
            in_edges:  stdlib::InEdges,  N -> E;
            source:    stdlib::Source,   E -> N;
            target:    stdlib::Target,   E -> N;
            edges:     stdlib::Edges,    N -> E := union(out_edges, in_edges);
        }
    }
}
```

- Cardinalities: `one`, `many` (declared on the root arrow; instantiations
  inherit them). Keyed partial arrives in M2.
- Derivation: `:= union(a, b)` over two sibling `many` arrows with the
  same self and associated types. Derived arrows instantiate a root trait
  like any arrow but are never bound to a backing; they get generated
  per-instantiation impls with their own markers, uniform with every other
  arrow instantiation (a blanket impl cannot name the marker parameters;
  per-instantiation impls keep qualification spellings single-marker and
  let a store hand-optimize a derived arrow). The reference blanket
  `impl<T> Edges for T` in `src/traits.rs` is deleted when `traits.rs` is
  retired.
- Entity unification by name within a block is composition-lite: the same
  mechanism M2's `compose_pattern!` generalizes across instances.

## Backings (built-in, fixed table in the macros crate)

| backing | shape | field contributions | hooks | access |
|---|---|---|---|---|
| `arena` | `many` | none (entity arenas are implicit) | none | index-range iterator over the target arena |
| `field` | `one` | one typed field on the self record | constructor arg | direct read |
| `intrusive_list` | `many` | `Option<BId>` head on self record, `Option<BId>` next on target record | push-front on insert | linked-list walk |

Rules:

- **Every entity gets a `Vec` arena, an id newtype (`NodeId(pub u32)`), and
  a `Copy` handle (`store ref + id`) unconditionally.** Arrows never own
  arenas. (This resolves the old "which trait owns the edges arena"
  question: no arrow does.)
- Default bindings: `one` → `field`. `many` arrows must be bound explicitly
  in v1 (`arena` vs `intrusive_list` is a real choice with no safe default).
- Field contributions may target the self record, the target record, or the
  store struct itself. No v1 built-in needs store-level fields; the
  contract allows them so M2's index backings don't require an interface
  change.
- Field names are namespaced per arrow (`first_out`, `next_out` for
  `out_edges`), so collisions are impossible by construction.
- `intrusive_list` iterates in **reverse insertion order** (push-front).
  The test suite's expected traversal orders depend on this; it is part of
  the backing's contract, not an accident.
- Generated `add_<entity>` constructors take one argument per total `one`
  arrow originating at that entity (`add_edge(source, target)` falls out of
  the schema), then run each bound backing's insert hook.

## Generated API (contract with the tests)

```rust
let mut s = GraphStore::new();
let a = s.add_node();
let b = s.add_node();
s.add_edge(a, b);
let g = s.graph();               // root handle
g.nodes();                       // -> iterator of Node handles
n.out_edges();                   // -> iterator of Edge handles
e.target();                      // -> Node handle
n.edges();                       // derived: out ++ in, self-loops twice
```

Handles implement `Eq`/`Hash` by store pointer + index path (a single
index at M1 depth). Markers are per-arrow-instantiation unit structs
named after the arrow, one module per pattern instance:
`digraph::markers::OutEdges` here, `hierarchy::markers::outer::OutEdges`
for composed patterns in M2 — module paths, never name concatenation.
With a single instance of each arrow, method calls never need
qualification.

`algo::bfs`/`dfs` gain marker parameters, inferred at call sites:

```rust
pub fn bfs<N, E, MO, MT>(start: N) -> Vec<N>
where
    N: Copy + Eq + Hash + OutEdges<MO, Edge = E>,
    E: Target<MT, Node = N>;
```

## Repo changes

- Convert to a workspace: root package `metagraph-exper` (runtime) +
  `macros/` member `metagraph-exper-macros` (proc-macro).
- Runtime crate: `algo.rs` (updated signatures), `stdlib.rs` (six root
  single-arrow `pattern!` invocations owning their traits, plus the
  `Digraph` instantiation block and re-exports), `arena.rs` kept as
  reference until tests are green, then deleted (git retains it).
- `tests/koenigsberg.rs`: imports and construction updated; assertions
  untouched.

## Task sequence

1. Workspace split; macros crate skeleton with `pattern!` parsing only
   (emit nothing but traits + markers; `cargo expand` to verify).
2. Courier emission from `pattern!`; `implement!` dispatcher;
   `__implement_gen!` skeleton emitting arenas, ids, handles, store.
3. `field` backing (`source`, `target`).
4. `arena` backing (`nodes`).
5. `intrusive_list` backing (`out_edges`, `in_edges`) including the
   push-front insert hook in `add_edge`.
6. Derived `union` arrow (`edges`).
7. Port `tests/koenigsberg.rs`; green.
8. `trybuild` compile-fail tests for `implement!` validation: wrong-
   cardinality backing, unknown arrow, duplicate binding, missing binding.
9. `cargo expand` comparison against `arena.rs`; delete `arena.rs`.
10. Doc pass: crate-level docs explaining the pattern/backing split.

## Acceptance criteria

- `cargo test` green; assertion lines in `koenigsberg.rs` byte-identical.
- Expanded code is boring: no type the user can't name, no trait bound not
  traceable to an arrow.
- `bfs`/`dfs` compile against the generated traits without marker
  annotations at call sites. This holds while each handle carries at most
  one impl of each trait; in M2 fused handles carry several, and call
  sites there use partial turbofish
  (`bfs::<_, _, markers::outer::OutEdges, _>(h)`).

## Risks specific to M1

- **Courier grammar fragility**: any drift between what `pattern!` embeds
  and what `__implement_gen!` parses fails inside generated code. Mitigate
  with a single shared grammar module in the macros crate and round-trip
  tests.
- **Scope creep toward composition**: entity arenas-always and markers now
  are the two choices M2 depends on; everything else compositional must be
  deferred even when visible.
