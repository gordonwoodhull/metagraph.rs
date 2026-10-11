# Milestone 1: `pattern!` + `implement!` for the base digraph

**Goal**: the hand-written `src/arena.rs` is replaced by macro-generated
code with materially identical behavior and API shape. The Königsberg test
suite passes with edits confined to imports and graph construction — every
assertion unchanged.

Explicitly out of scope: `compose_pattern!`, open entities, keyed partial
arrows, user-defined backings, backing auto-selection.

## Architecture: the macro pipeline

Proc macros cannot read other items' tokens, so schema information travels
between macros only through generated code. The pipeline:

1. **`pattern!`** (proc macro, parses + validates the schema DSL) emits its
   items at the invocation site:
   - for a root single-arrow pattern: the marker-generic trait it owns
     (`pub trait OutEdges<M> { ... }`), with a GAT iterator for `many`
     arrows, a plain associated type for `one`, and — for M2's keyed
     arrows — the method parameter and its associated key type
     (`fn subnode(&self, key: Self::Key) -> Option<Self::SubNode>`; the
     key sort is an associated type because the root cannot name the
     instantiation's key entity);
   - for every root: a **registration courier** publishing the root's
     metadata (trait method name, GAT iterator name, associated type
     names, cardinality; a keyed root additionally publishes its key's
     associated type name, so codegen types the parameter through the
     instantiation's `key(LocalEntity)` binding). Roots register
     themselves; there is no fixed table of stdlib arrows in the macros
     crate, and user-declared roots work exactly like stdlib ones;
   - for an instantiation pattern: one unit marker struct per arrow
     instantiation (per use, not per root — two instantiations of
     `stdlib::Nodes` get distinct markers), inside a `markers` module
     (see Naming conventions), plus the **schema courier** embedding the
     schema as tokens.
2. **`implement!`** (proc macro, thin dispatcher): parses the invocation
   (grammar below), validates the pattern path, chooses the runtime crate
   path itself (`proc-macro-crate`, with a `crate = ...` override), and
   emits an invocation of the pattern's courier with `__implement_gen!`
   as the forwarding target. It does no codegen itself.
3. **`__implement_gen!`** (doc-hidden proc macro), in two stages:
   - *Resolution*: parses schema tokens + user tokens, then resolves each
     arrow's root metadata by invoking the root's registration courier
     (the same courier mechanism, with the codegen stage as the
     forwarding target). This is how user-declared roots are implementable
     at all: their metadata arrives as tokens, always generated from the
     root's own declaration, so it can never drift from the trait.
   - *Codegen*: with metadata in hand, validates the backing binding and
     emits the concrete store (all codegen lives here, in real Rust, not
     tt-munching).

Every courier, root or schema, has exactly one rule and never interprets
anything:

```rust
macro_rules! __metagraph_pattern_Digraph {
    (($fwd:path ; $($rest:tt)*)) => {
        $fwd ! { { /* schema tokens */ } ; $($rest)* }
    };
}
```

The courier is **target-parameterized**: the caller names the forwarding
target, and the schema travels wrapped *inside* the target's delimiter
group. (A sibling `{ ... }` group next to the forwarded tokens is not a
valid item — the schema must ride inside the invocation.) This shape is
permanent: the courier never gains grammar rules, because all grammar
lives in the proc macros' parsers, where errors get real spans. M1's only
target is `__implement_gen!`; M2's composition chain and M1's own
root-metadata resolution reuse the same shape with other targets.

**Courier path resolution is a cross-crate requirement, not a detail.**
The stdlib's patterns are defined in the runtime crate but `implement!` is
invoked in *user* crates, so couriers must be addressable by path. The
mechanics:

- Plain `macro_rules!` + `pub use` is not re-exportable cross-crate
  (E0364). Every courier needs `#[macro_export]`, which anchors it at the
  *defining crate's root* macro namespace — so pattern names must be
  unique crate-wide. `pattern!` also emits `#[doc(hidden)]
  #[allow(unused_imports)] pub use __metagraph_pattern_Digraph;`
  (textually after the macro) at its invocation site, making the courier
  addressable as `metagraph_exper::stdlib::__metagraph_pattern_Digraph`.
- `implement!` takes the pattern as a path `P::Name` whose last segment
  must be a plain ident (reject generics with a targeted error) and
  derives everything from it: the courier is `P::__metagraph_pattern_Name`,
  the markers module is `P::<snake(Name)>::markers`. A single-segment path
  requires the courier to be in scope; detect that case and emit "import
  the pattern's module" rather than leaking the mangled courier name.
- All runtime-crate paths (`__implement_gen!`, handle types, the resolver
  stage) are chosen by `implement!` at invocation time via
  `proc-macro-crate` (plus the `crate = ...` override) and flow through
  the forwarding-target parameter. `pattern!` never bakes an absolute
  runtime path into a courier, so renamed dependencies work.
- The runtime crate root carries `extern crate self as metagraph_exper;`
  so generated absolute paths resolve inside the runtime crate itself
  (the stdlib is the first consumer; E0433 otherwise).
- The runtime crate re-exports `#[doc(hidden)] pub use
  metagraph_exper_macros::__implement_gen;` — proc macros are not
  reachable through transitive dependencies.

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
way the stdlib does. Instantiations name the root by path and never
restate its cardinality or associated types — those are inherited, and
`implement!` resolves them through the root's registration courier, so an
instantiation cannot contradict its root. The one exception is the
key-entity binding of a keyed root (M2): the root declares the keyed
cardinality and owns the method's key parameter, but it cannot name the
instantiation's key sort, so the instantiation supplies `key(LocalEntity)`
as a local clause — exactly like `by`, and never a restatement of
cardinality. The root's trait types the parameter as an associated type
(`type Key`; M2's keyed section sketches the declaration and the
probe-verified trait shape).

**Instantiation arrow names are local aliases.** The trait's method name,
GAT iterator name, and associated type names all come from the *root's*
declaration, never from the instantiation: `subnodes: stdlib::Nodes,
Subgraph -> SubNode` is still called `.nodes()` and yields
`Nodes::Nodes<'a>` — emitting `fn subnodes` is a trait mismatch (E0407,
spike-verified). The registration courier's metadata therefore carries
the full (method name, GAT name, associated type names, cardinality)
tuple — plus the key's associated type name for keyed roots (M2). Generated *operation* names (`add_<entity>`, `insert_<arrow>`,
`set_<arrow>`) are the opposite: they derive from the instantiation's
local names, since they are store API, not trait identity. A distinct
read-side spelling, if wanted, is an inherent alias under the rule in
M2's marker-qualification section.

Markers are scoped per pattern instance and re-minted by
`compose_pattern!` per composition instance: fusing two instances of one
pattern yields distinct markers, so diamond instantiation produces two
impls under two markers, never a conflicting impl. M1's codegen defines
the marker namespace that `compose_pattern!` inherits, so the rule lives
here: `pattern!` itself emits `pub mod <snake(pattern)> { pub mod markers
{ ... } }` at the invocation site, and `implement!` derives the module
path from the pattern path (above).

## `pattern!` syntax (M1 subset)

Entity names are written in full words — generated names derive
mechanically from them (see Naming conventions), and single letters would
produce unreadable code. One entity per pattern is explicitly marked
`root`:

```rust
// stdlib: the root arrow patterns (each owns and registers its trait)
pattern! { Nodes    { entities: root Graph, Node; arrows { nodes:     Graph -> Node, many; } } }
pattern! { OutEdges { entities: root Node,  Edge; arrows { out_edges: Node -> Edge, many; } } }
// ... InEdges, Source, Target, Edges

// user side: instantiate stdlib arrows, unifying entities by name
pattern! {
    Digraph {
        entities: root Graph, Node, Edge;
        arrows {
            nodes:     stdlib::Nodes,    Graph -> Node;
            out_edges: stdlib::OutEdges, Node -> Edge, by source;
            in_edges:  stdlib::InEdges,  Node -> Edge, by target;
            source:    stdlib::Source,   Edge -> Node;
            target:    stdlib::Target,   Edge -> Node;
            edges:     stdlib::Edges,    Node -> Edge := union(out_edges, in_edges);
        }
    }
}
```

- Cardinalities: `one`, `many` (declared on the root arrow; instantiations
  inherit them). Keyed partial arrives in M2.
- The `root` marking is explicit, never derived: M2's placement rule and
  the store's root accessor both read it. (Derivation — "the entity never
  targeted by any arrow" — works for Digraph but breaks the moment an
  arrow points at the root.)
- No arrow may *target* the root-marked entity: after the no-arena rule
  it owns no id type to store (M2's pattern-time check, with a real
  error message; before that rule, generated code simply failed to
  compile — spike 10). Arrows *sourced* at the root are fine.
- Membership of a `many` arrow is **primitive, derived, or
  existence-derived**. Primitive membership is asserted directly
  (`tags: Post -> Tag, many`) and gives the builder an explicit
  `insert_<arrow>(a, b)` operation. Derived membership names the total
  `one` arrow that determines it (`out_edges: Node -> Edge, by source`):
  the determining field lives on the target record, the arrow's insert
  hooks fire from constructors (and, in M4, from `set_`), and no insert
  operation is generated — membership can never contradict the field.
  `by` is a property of the instantiation, not the root trait: the stdlib
  root declares a bare capability. The determining arrow must be total, so
  membership is always defined. Existence-derived membership is the entity
  arrows (`nodes: Graph -> Node`, arena-backed): a record is a member by
  existing, the write operation is `add_<entity>` itself, and no insert op
  is generated. (`union` is a different axis: it derives a *read* from
  sibling reads, not membership from a field.)
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

## `implement!` syntax

```rust
implement! { stdlib::Digraph as GraphStore {
    nodes:     arena,
    out_edges: intrusive_list,
    in_edges:  intrusive_list,
} }
```

The pattern is a path (last segment a plain ident); the store name follows
`as`; the block binds `many` arrows to backings. `one` arrows default to
`field` and their bindings may be omitted; derived arrows (`:= union`)
take no binding and are rejected if given one. Validation errors: unknown
arrow, duplicate binding, missing binding, wrong-cardinality backing —
in both directions once keyed arrows land (M2): `hash_index` on a
non-keyed arrow, and a keyed arrow bound to anything else — each
quoting the offending tokens in the message text, because spans have
degraded to the call site by the time the check runs.

## Naming conventions

Generated names derive mechanically from schema names; nothing is
hand-renamed:

| schema item | generated items |
|---|---|
| non-root entity `Node` | id `NodeId(pub u32)`, record `NodeData`, arena field `node_arena`, handle `Node<'g>` |
| arrow `out_edges` | marker `<pattern>::markers::OutEdges`, iterator `OutEdgesIter`, insert op `insert_out_edges` |
| root entity `Graph` | handle `Graph<'g>` and root accessor `graph()` on the store — nothing else |

Every non-root entity gets a `Vec` arena, an id newtype, a record struct,
and a `Copy` handle unconditionally: arrows never own arenas, and no code
path distinguishes one entity's storage from another's. The root-marked
entity is the single exception, and the exception is semantic, not a
special case: its record is the store itself. It is never constructed and
owns no arena, record, or id type — only its handle, which the root
accessor and the trait methods sourced at it need. (Spike-verified on the
M2 compose-chain workspace: dropping the root's vestigial kit from flat
codegen is a ~38-line diff confined to flat codegen — the entity-kit loop,
the store's arena-field list, and the per-entity count functions — with
the whole test suite green unmodified. The only API casualty is
`graph_count()`, which always returned 0 and which no test used; per-entity
counts are emitted for non-root entities only.)

## Backings (built-in, fixed table in the macros crate)

| backing | shape | field contributions | hooks | access |
|---|---|---|---|---|
| `arena` | `many` | none (entity arenas are implicit) | none | index-range iterator over the target arena |
| `field` | `one` | one typed field on the self record | constructor arg | direct read |
| `intrusive_list` | `many` | `Option<EdgeId>` head on self record, `Option<EdgeId>` next on target record | push-front on insert | linked-list walk |

Rules:

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
  the schema), then run each bound backing's insert hook — including the
  hooks of `many` arrows derived `by` those `one` arrows, which is how
  `add_edge` maintains `out_edges` and `in_edges`.
- **Hooks are named functions, and they are the single write path.** Every
  storage mutation — field writes, intrusive links — lives in a doc-hidden
  per-backing hook function over `&mut` store state; the `add_*`
  constructors are thin orchestration over them (pattern-level constructor
  logic sits in the orchestration layer, never inside backing hooks).
  M2's keyed backings and M4's flush replay build on exactly this
  boundary, so the function-per-hook shape is a commitment, not an
  implementation detail.
- **Every generated store has a `validate()` method.** M1 has nothing to
  check — it returns `Ok(())` — but the boundary exists from here on: M2
  hangs the key checks and the per-backing structural checks on it (M2's
  canonical inventory), and M4's flush calls it unconditionally.

## Generated API (contract with the tests)

```rust
let mut s = GraphStore::new();
let a: NodeId = s.add_node();       // constructors return ids
let b: NodeId = s.add_node();
let e: EdgeId = s.add_edge(a, b);   // one arg per total `one` arrow
s.node_count(); s.edge_count();     // per-entity counts
let g = s.graph();                  // root handle
g.nodes();                          // -> iterator of Node handles
n.id();                             // -> NodeId (also e.id() -> EdgeId)
n.out_edges();                      // -> iterator of Edge handles
e.source(); e.target();             // -> Node handles
n.edges();                          // derived: out ++ in, self-loops twice
```

Handles implement `Eq`/`Hash` by store pointer + index path (a single
index at M1 depth) — hand-written, never derived, and forwarding to
(store pointer, id), never the id alone: a `derive` would bound
`Store: Eq`/`Hash`, and id-only equality would equate handles from
different stores (spike-verified; the reference `arena.rs` —
`std::ptr::eq` plus id — is the shape, and spike 8 makes this identity
load-bearing for generic transforms). Markers are per-arrow-instantiation unit structs named
after the arrow, one module per pattern instance:
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
  `macros/` member `metagraph-exper-macros` (proc-macro). The workspace
  split and the proc-macro/trybuild dependency stack are the milestone
  that reshapes `Cargo.lock`; start tracking it in git at that point
  (reproducible checkouts, and it removes one noise source from the
  trybuild `.stderr` pinning).
- The macros crate has **one shared grammar module** used by `pattern!`'s
  emission and `__implement_gen!`'s parsing, with round-trip tests. This
  is a commitment, not a mitigation: any drift between what `pattern!`
  embeds and what `__implement_gen!` parses fails inside generated code,
  where it is undebuggable from user-facing error messages.
- Runtime crate: `algo.rs` (updated signatures), `stdlib.rs` (six root
  single-arrow `pattern!` invocations owning their traits, plus the
  `Digraph` instantiation block and re-exports), `arena.rs` kept as
  reference until tests are green, then deleted (git retains it).
- `tests/koenigsberg.rs`: imports and construction updated; assertions
  untouched.

## Task sequence

1. Workspace split; macros crate skeleton with `pattern!` parsing only
   (emit nothing but traits + markers; `cargo expand` to verify).
2. Courier emission from `pattern!` (schema courier and root registration
   courier, same rule shape); `implement!` dispatcher; `__implement_gen!`
   skeleton with the two-stage (resolution → codegen) structure, emitting
   arenas, ids, handles, store.
3. `field` backing (`source`, `target`).
4. `arena` backing (`nodes`).
5. `intrusive_list` backing (`out_edges`, `in_edges`) including the
   push-front insert hook in `add_edge`.
6. Derived `union` arrow (`edges`).
7. Port `tests/koenigsberg.rs`; green.
8. `trybuild` compile-fail tests for `implement!` validation: wrong-
   cardinality backing, unknown arrow, duplicate binding, missing binding.
   Error messages must quote the offending tokens (spans have degraded).
9. Downstream-crate tests: a user crate implements the stdlib `Digraph`;
   a user crate declares *its own* root single-arrow pattern and
   implements a pattern instantiating it (roots register themselves — the
   stdlib enjoys no privilege); a renamed dependency (`mg` for
   `metagraph-exper`) works via `proc-macro-crate` and `crate = ...`.
10. `cargo expand` comparison against `arena.rs`; delete `arena.rs`.
11. Doc pass: crate-level docs explaining the pattern/backing split.

## Acceptance criteria

- `cargo test` green; assertion lines in `koenigsberg.rs` byte-identical.
- Expanded code is boring: no type the user can't name, no trait bound not
  traceable to an arrow.
- `bfs`/`dfs` compile against the generated traits without marker
  annotations at call sites. This holds while each handle carries at most
  one impl of each trait; in M2 fused handles carry several, and call
  sites there use partial turbofish
  (`bfs::<_, _, markers::outer::OutEdges, _>(h)`).
- The step-9 downstream-crate tests pass: user-declared root, renamed
  dependency.

## Risks specific to M1

- **Courier grammar fragility**: covered by the shared-grammar commitment
  above.
- **Span degradation compounds**: schema tokens are re-spanned at every
  courier hop (M1 has one; M2 composition and root resolution add more).
  Quoting offending tokens in error messages is mandatory from the start.
- **Scope creep toward composition**: entity arenas-always and markers now
  are the two choices M2 depends on; everything else compositional must be
  deferred even when visible. The courier's target parameter is the one
  exception: it exists in M1 because root-metadata resolution already
  needs it, and it is exactly the mechanism M2's composition chain
  reuses.
