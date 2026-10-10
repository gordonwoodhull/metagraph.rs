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
        on_insert(a, b) { /* push-front */ }
        iterate(a) { /* linked-list walk */ }
    }
}
```

Success criterion: the four built-ins are *re-implemented* in the DSL and
the M1/M2 test suites pass unchanged. If the DSL can't express them, the
DSL is wrong, not the tests.

### A second `many` backing

CSR-style range adjacency (build in two passes, iterate a slice) — chosen
because it has genuinely different tradeoffs (mutation during construction
vs. after) and forces the backing interface to say something about build
phases. This is the first moment backing *choice* is real, and it motivates:

### Constraint and cost annotations

Backings declare what they require (local arenas on both ends, append-only
phase, `Eq + Hash` ids) and what they cost (bytes per record, iteration
order guarantees). `implement!` gains the ability to reject an
inapplicable backing with a good message. Full automatic selection is
possible here but not required — the annotations are the prerequisite, and
they pay for themselves in error quality alone.

### Standard library refactor

- `Digraph` is redefined as a composition of six single-arrow patterns,
  with zero user-visible change (the M1 tests are the regression suite).
- `Subgraph`, `EdgeChain`, `Bipartite`, `Hierarchy` ship as library
  patterns users compose, per the original vision: the toolkit's language
  defines its own standard library.

## Open questions

- How much of a backing's insert hook is code vs. protocol description
  (can the DSL avoid arbitrary Rust in hooks? probably not — hygiene and
  spans need care).
- Whether cost hints are numbers, symbols (`cache_friendly`, `compact`),
  or deferred entirely.
- Does CSR's two-phase construction require stores to have a `freeze`
  notion, and is that per-store or per-backing?

## Depends on

M2 complete, especially a hook interface that survived keyed indexes.
Do not start the `backing!` grammar before the four built-ins exist —
that ordering is the whole point of the milestone.
