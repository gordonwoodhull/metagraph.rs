# Milestone 3: user-facing backings, stdlib refactor

**Goal**: the backing layer becomes part of the language, and the standard
library becomes proof that the language suffices.

Specification level: deliberately vague. Both the backing interface and the
stdlib's shape will be informed by two milestones of scars.

## In scope (directionally)

### `backing!` DSL

Extract the built-in backings (`arena`, `field`, `intrusive_list`,
`hash_index`) into user-writable declarations:

```rust
backing! {
    intrusive_list: A -> B, many {
        on A { first: Option<BId> }
        on B { next: Option<BId> }
        iter Walk { /* named iterator struct over the link fields */ }
        on_insert(a, b) { /* push-front */ }
        iterate(a) { /* linked-list walk, returning Walk */ }
    }
}
```

Success criterion: the four built-ins are *re-implemented* in the DSL and
the M1/M2 test suites pass unchanged. If the DSL can't express them, the
DSL is wrong, not the tests.

Two mechanical obligations come with user-writable backings:

- **Token routing.** A backing declaration has the same cross-item
  problem a pattern has: `implement!` must know a backing's field
  contributions and hooks at expansion time, and proc macros cannot read
  other items' tokens. `backing!` therefore emits a *backing courier* —
  the same target-parameterized single-rule courier as patterns and
  roots — embedding its field declarations, hook bodies, and constraint
  annotations; `implement!` resolves a backing binding by invoking its
  courier. No fixed table of backings, same as no fixed table of roots.
- **Iterator types.** The generated read traits use GATs, whose
  associated type must name a concrete type. A `many` backing must
  therefore declare its iterator as a named struct (the `iter Walk`
  clause above); a bare `iterate(a) { ... }` body is insufficient on its
  own. Hook and iterator bodies contain arbitrary Rust — the DSL will not
  avoid that — so the hygiene of user idents forwarded through the macro
  layers must be prototyped before the `backing!` grammar is fixed.

### A second `many` backing

CSR-style range adjacency (build in two passes, iterate a slice) — chosen
because it has genuinely different tradeoffs (mutation during construction
vs. after) and forces the backing interface to say something about build
phases. Its two-phase construction maps onto the builder boundary (epic
invariant 5): accumulate adjacency during the build, compact to ranges at
flush; freeze is per-store, since the whole store flushes at once. This is
the first moment backing *choice* is real, and it motivates:

### Constraint and cost annotations

Backings declare what they require (local arenas on both ends, append-only
phase, `Eq + Hash` ids) and what they cost (bytes per record, iteration
order guarantees). `implement!` gains the ability to reject an
inapplicable backing with a good message. The intrusive-ownership hazard
from M2 (link fields are sound only when each object belongs to one
relation instance) is expressed here as an *exclusive-membership*
constraint annotation on `intrusive_list`; shared membership requires a
link-record design with its own arena (see M2's refinement discussion).
Full automatic selection is possible here but not required — the
annotations are the prerequisite, and they pay for themselves in error
quality alone. Cost hints (numbers vs. symbols such as `cache_friendly`,
`compact`) are decided when the annotations exist.

### Standard library refactor

- `Digraph` already instantiates the six single-arrow root patterns (per
  the trait-identity invariant), so this step is small: re-express it via
  `compose_pattern!` instead of inline instantiation, with zero
  user-visible change (the M1 tests are the regression suite). If
  degenerate composition turns out to produce byte-identical output to
  inline instantiation, say so and drop the ceremony.
- `union` is multiset-sum, not set union (self-loops appear twice — that
  is contract, not accident). If it is renamed (`chain`, `sum`), this
  milestone is the last cheap moment.
- `Subgraph`, `EdgeChain`, `Bipartite`, `Hierarchy` ship as library
  patterns users compose, per the original vision: the toolkit's language
  defines its own standard library. Candidate root arrows discovered in M2
  (`EdgesOf`, `MainNode`, …) get promoted into the stdlib here.

## Depends on

M2 complete, especially a hook interface that survived keyed indexes.
Do not start the `backing!` grammar before the four built-ins exist —
that ordering is the whole point of the milestone.
