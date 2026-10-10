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
   - one marker-generic trait per arrow (`pub trait OutEdges<M> { ... }`),
     with a GAT iterator for `many` arrows, plain associated type for `one`;
   - one unit marker struct per arrow;
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
courier with exactly one grammar rule.

## `pattern!` syntax (M1 subset)

```rust
pattern! {
    Digraph {
        entities: G, N, E;
        arrows {
            nodes:     G -> N, many;
            out_edges: N -> E, many;
            in_edges:  N -> E, many;
            source:    E -> N, one;
            target:    E -> N, one;
            edges:     N -> E, many := union(out_edges, in_edges);
        }
    }
}
```

- Cardinalities: `one`, `many`. (Keyed partial arrives in M2.)
- Derivation: `:= union(a, b)` over two `many` arrows with the same
  self-type and associated type. Derived arrows get a trait like any arrow
  but are never bound to a backing.

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
- Field names are namespaced per arrow (`first_out`, `next_out` for
  `out_edges`), so collisions are impossible by construction.
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

Handles implement `Eq`/`Hash` by store pointer + index (as today).
Markers are per-arrow unit structs in a `digraph::markers` module; with a
single instance of each arrow, method calls never need qualification.

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
- Runtime crate: `algo.rs` (updated signatures), `stdlib.rs` (the
  `pattern!` invocation for `Digraph` plus re-exports), `arena.rs` kept as
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
8. `cargo expand` comparison against `arena.rs`; delete `arena.rs`.
9. Doc pass: crate-level docs explaining the pattern/backing split.

## Acceptance criteria

- `cargo test` green; assertion lines in `koenigsberg.rs` byte-identical.
- Expanded code is boring: no type the user can't name, no trait bound not
  traceable to an arrow.
- `bfs`/`dfs` compile against the generated traits without marker
  annotations at call sites.

## Risks specific to M1

- **Courier grammar fragility**: any drift between what `pattern!` embeds
  and what `__implement_gen!` parses fails inside generated code. Mitigate
  with a single shared grammar module in the macros crate and round-trip
  tests.
- **Scope creep toward composition**: entity arenas-always and markers now
  are the two choices M2 depends on; everything else compositional must be
  deferred even when visible.
