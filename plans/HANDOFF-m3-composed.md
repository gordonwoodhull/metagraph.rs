# HANDOFF — M3 implementation COMPLETE (steps 2–9; merge chore)

**Status when written:** every step of
`plans/milestone-3-backings-and-stdlib.md` is implemented and ticked,
including the overall gate, PLUS the completion pass that closed M3's
two carried items (below). Branch `worktree-m3-implement` (worktree
`.claude/worktrees/m3-implement`), cut from `9c3569c` = main; tip
`2c78f82` — 19 commits past main, NOT merged; the user merges. Full
workspace green: 38 suites (koenigsberg 8, csr 12, composed_csr 3,
validate_failures 11, unlink 4, compilefail's 40 fixtures, downstream
crates, doctests); both feature-gated negative builds
(`downstream-backing-indirect`, `downstream-indirect`, `--features
indirect`) reject with the resolver's clean message.

## What step 9 did

- **Stdlib promotion:** `EdgesOf`, `MainNode`, `MainEdge`, `SubNodeOf`,
  `SubEdgeOf` moved from the fixtures crate's `roots` module into
  `metagraph_exper::stdlib` (names as M2 resolved them; the naming rule
  is documented at the new home). The fixtures' shapes/neg patterns
  re-point by path (`metagraph_exper::stdlib::…` — the spelling the
  schemas already used for `Nodes` et al.), and the five test-file trait
  imports follow (hierarchy_subgraphs ×2, edge_induced,
  validate_failures, downstream/composed-renamed under its `mg` rename).
  stdlib's keyed shape-pin test now targets the shipped `SubNodeOf`
  instead of a local twin.
- **Doc pass:** the `implement!` docs (macros crate) and the crate-level
  backings table (runtime) now name all five backings + the type-state
  store; the stdlib sentence names the promoted roots. The
  generated-code doc audit came back CLEAN for both codegens — flat's
  store contributions, record fields, hooks, write ops, read view,
  iterators, and validate contributions all name the producing arrow
  (+ backing); composed adds the instance qualifier; entity items name
  the entity; trait impls carry none by rustdoc's rendering. No codegen
  changes were needed.

## Two consequences the promotion earned (recorded in the commit)

1. `impl_keyless_root.stderr` regenerated: the keyless rejection quotes
   the resolved root path, now
   `metagraph_exper :: stdlib :: SubNodeOf` (was `crate :: roots :: …`).
2. **downstream-indirect's negative case was GUTTED by the promotion**
   and re-armed: its rejection fired on `Subgraph`'s `crate::roots::…`
   instantiation paths — the only schema-internal paths that forced the
   m2-fixtures direct-dependency check. All of Subgraph's refs now
   resolve through the runtime, which every implement site depends on
   directly, so the fixture built green where it must fail. The fixtures
   crate gained a minimal local root (`local::Tags` + `DocTags`, open
   `Tag`) and the fixture now composes `DocTags` — a member that names
   its own declaring crate — and rejects cleanly again. LESSON for later
   milestones: the declaring-crate-directness rule is enforced only
   through schema-internal path resolution; a member whose every
   instantiation path targets another crate never trips it.

## The completion pass (two carried items, closed)

- **Docking arms landed** (`7a5f7f7`): `classes_legal` now rejects
  same-instance element-element fusion and the self-hosting regress —
  both were silently accepted under the at-most-one-instance reading.
  Fixtures `compose_coloc_same_instance` / `compose_coloc_self_hosting`
  pin the messages. The debt is recorded as discharged in M2's plan
  (check 5 + negative-tests list), M3's step-0 note, and the epic.
- **`on_unlink(a, b)` landed** (`2c78f82`): the M4 review's D3 fold,
  taken VERBATIM into this plan (grammar sketch, inventory row, spec
  bullet, step-4 box) so the M4 rebase conflicts only on checkbox
  states. Grammar + stdlib body (probe-verified spelling) + both
  codegens (`__unlink_<arrow>`, instance-qualified when composed,
  write-side/Build impl, self-less at an unfused root) +
  tests/unlink.rs (primitive, by-derived re-point trio, root-sourced,
  composed). `backing_unknown_clause.stderr` regenerated.

## Merge chore (for the user's merge session)

- Branch is strictly ahead of main (cut at `9c3569c`, main unmoved
  since); expect a clean fast-forward.
- The three cross-branch reconciliations main grafted at `9c3569c`
  (intrusive-at-unfused-root DSL-legal, field-into-embedded scope,
  docking-arms same-instance) are already PART of this branch's base —
  nothing to redo.
- After merge: M4's `HANDOFF-m4-impl-review.md` is CLOSED (its go was
  delivered); the M5 plan (`plans/milestone-5-data.md`, docked on
  `worktree-m5-plan` at `d0459dd` off main) is next per the epic —
  M3 landing was its stated prerequisite. M6 stays stacked behind M5.
- M4 rebase onto the new main: its M3-plan copy differs from main's new
  M3 plan ONLY by checkbox states (theirs pre-implementation `[ ]`,
  ours ticked) and the step-4 `on_unlink` box (ours ticked + a
  completion-pass parenthetical). Resolve by taking OURS on every
  checkbox conflict; the on_unlink substance is identical on both
  sides. M4's `milestone-4-builders.md` prose about `__unlink_<arrow>`
  now matches emitted code — its step 7 replays rather than amends, as
  the review intended.
- `tests/downstream/composed-renamed` still exercises the rename arm
  via `Subgraph` (m2-fixtures, renamed `fx`) AND the promoted roots via
  `mg::stdlib` — both declaring-crate resolutions in one test.

## Verification commands (all green at tip)

- `cargo test --workspace` — 38 suites.
- `cargo test -p metagraph-exper --test koenigsberg` — byte-sensitive
  traversal-order assertions.
- `cargo test -p metagraph-exper --test compilefail` — pinned stderr;
  regenerate with `TRYBUILD=overwrite` (done once this step).
- `cargo build -p downstream-backing-indirect --features indirect` and
  the `downstream-indirect` equivalent — each must FAIL with the
  resolver's clean "does not depend on directly" message (they do).
- `MG_DEBUG=1 cargo test -p metagraph-exper --test <t>` — dump
  `implement!` / `__implement_gen!` emissions when debugging.
