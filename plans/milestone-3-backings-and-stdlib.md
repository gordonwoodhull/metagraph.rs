# Milestone 3: user-facing backings, stdlib refactor

**Goal**: the backing layer becomes part of the language, and the standard
library becomes proof that the language suffices.

Specification level: partial — decided and sketched to grammar. Every
question this file carried as "deliberately vague" now has a folded
decision (grammar coverage, the validate mechanism, the constraint
language, phase representation, the stdlib refactor, stdlib promotion),
each grounded in a spike or probe. Uncompiled: the macro side of
`backing!` — grammar and codegen — as with M2's keyed grammar.
Spike-verified islands: hook-body hygiene across the token channel
(spike 5), keyed mechanics in both embedded and store-level map
placements (spike 11; the edge-induced probe, 5/5 tests), CSR with its
phase annotations (spike 6), and the composed-`Digraph` rejection that
settles the stdlib refactor (spike 12).

## The premise: the built-ins stop being special

`arena`, `field`, `intrusive_list`, `hash_index` become ordinary
`backing!` declarations shipped in the runtime crate. The fixed table of
backings in the macros crate is deleted; `implement!` resolves every
backing — stdlib or user — through its **backing courier**, exactly as it
resolves root arrows through registration couriers. No fixed table of
backings, same as no fixed table of roots. The success criterion is the
proof: the four built-ins re-implemented in the DSL, M1/M2 test suites
unchanged. If the DSL can't express them, the DSL is wrong, not the
tests.

The consequence, worked through: every behavior M1/M2 stated as a codegen
fact about the four built-ins is now either **(a)** a consequence of what
a `backing!` declaration states, or **(b)** a codegen-reserved rule
uniform across all backings.

| behavior | class | grounding |
|---|---|---|
| intrusive head/link fields, push-front hook, reverse-insertion order, exclusive membership | (a) | M1 backings table — "part of the backing's contract, not an accident"; spike 5 |
| field's field-on-self-record, constructor arg, direct read | (a) — the `ctor_arg` clause | M1 backings table |
| arena: no fields, no hooks, existence membership, iterator over the target arena | (a), including the declared requirement `hosts_arena` | M1 backings table |
| hash_index map placement: root-level self record, host-embedded record, or store | (a) — contributions target the self record; a root-marked self's record *is* the store | M1 naming rule; spike 11 (host record); edge-induced probe (store-level) |
| per-backing validate checks (key existence, value-in-arena, link well-formedness, membership consistency, partition, CSR offsets) | (a) — the validate clause (below) | M2 canonical inventory; spike 4; probe; spike 6 |
| placement computation; entity kits (`Id(pub u32)`, compound path ids under embedding); hand-written handle `Eq`/`Hash` by (store pointer, id); the function-per-hook shape; stem-namespacing and collision prefixing of contributed names | (b) — uniform, backing-independent | M2 placement rule; M2 ("`NodeId`'s `pub u32` stays — the tests depend on it"); spike 11 finding 4 |
| arena bound to an arrow whose target's arena is hosted elsewhere | arena's declared requirement (`hosts_arena`) | M2 frontier; Hierarchy is the legal shape |
| intrusive sourced at an embedded class | (b) uniform restriction: contributed fields locate only on root-level classes or the store in M3 | M2 frontier |
| intrusive sourced at an unfused root | DSL-legal — store-level head, self-less insert op (the probe's shape); behind the codegen frontier | edge-induced probe |

Three placement-resolution rules make the table work, all uniform:

- **Contributions target the self record; a root-marked self's record is
  the store** (M1's naming rule, applied to backings). One rule covers
  hash_index's three observed placements — root-level self record,
  host-embedded record (spike 11: the map sits on `OuterNodeData` beside
  the embedded arena), and store-level (the probe: `sub.Subgraph` is
  root-marked and unfused, so `on A` resolves to the store, and no self
  id exists). `on store` (CSR) is the explicit form of the same slot.
- **Field values may be path ids** (spike 11 finding 1: path ids derive
  `Copy`/`Eq`/`Hash` unchanged and work as map values). The M2 frontier
  item "field backing into an embedded class rejected (its id needs a
  host path)" was a spike-1 codegen limit, not a semantic rule — it is
  codegen work, not a constraint, and M3 does not re-add it as one.
- **A root-marked self contributes no id**: the op and hook drop the self
  parameter, and bodies may not name it (grammar-checked). The probe
  pinned this — "a 3-arg form `insert_subnode(self_id, key, target)` is
  unemittable because the root-marked self owns no id".

## `backing!` grammar

**Bodies are ordinary Rust over roles; codegen binds the roles per
placement.** `a`, `b`, `k` are the hook's id parameters; `rec(x)` is a
record through its placement-correct path (flat:
`store.x_arena[i]`; embedded: `store.host_arena[h].x_arena[i]`);
`b_arena_of(a)` and `map_from(a)` name "the arena/map hosted by `a`'s
record", where a root-marked self's record is the store. Contributed
field names (`value`, `first`, `next`, `index`) are reserved stems bound
at emission to this arrow's namespaced fields — M1's naming tables
(`source: NodeId`, `first_out`, `next_out`) carry over unchanged. The
exact placeholder spelling is milestone-internal; the discipline is
spike 5's caveat 3 (a reserved set of placeholder idents; the
substitution is textual, so the grammar must forbid bodies from
shadowing them).

The stdlib's five declarations:

```rust
// field — M1's source/target
backing! {
    field: A -> B, one {
        on A { value: BId }
        ctor_arg        // feeds the record literal at creation; the ctor arg
                        // is named after the arrow (add_edge(source, target))
        read(a) { rec(a).value }     // id-space; codegen wraps into the B handle
        validate { for ra in records(A) { /* ra.value in B's arena */ } }
    }
}

// arena — M1's nodes
backing! {
    arena: A -> B, many {
        membership: existence        // no insert op; the write path is add_<B>
        iter Walk { cur: u32, len: u32 }   // codegen adds the store ref
        iterate(a) { Walk { cur: 0, len: b_arena_of(a).len() } }
        requires: hosts_arena(A, B)
        // validate: nothing — existence cannot dangle
    }
}

// intrusive_list — M1's out_edges / in_edges
backing! {
    intrusive_list: A -> B, many {
        on A { first: Option<BId> }      // emits first_out / first_in
        on B { next: Option<BId> }       // emits next_out / next_in
        iter Walk { cur: Option<BId> }
        on_insert(a, b) {                // push-front
            rec(b).next = rec(a).first;
            rec(a).first = Some(b);
        }
        iterate(a) { Walk { cur: rec(a).first } }
        iteration_order: reverse_insertion
        requires: exclusive_membership, direct_records
        validate by {          // membership consistency against the determining
            /* field: each B appears exactly once in the list it names  */
        }
        validate primitive {   // the ownership partition walk: per-list and
            /* global seen sets; distinguishes shared membership from cycles */
        }
    }
}

// hash_index — M2's subnode / subedge
backing! {
    hash_index: A -> B, key(K) {
        on A { index: HashMap<KId, BId> }   // self record; the store when A
                                            // is root-marked (placement rules)
        on_insert(a, k, b) { map_from(a).insert(k, b) }
        lookup(a, k) { map_from(a).get(&k).copied() }
        validate {          // every self record's map, wherever placed:
            /* key existence (k names a real K record; else
               Err("…index key {k:?} does not exist (len {n})")
               and value-in-arena (v names a real B record) */
        }
    }
}

// csr_adjacency — the second `many` backing (spike 6)
backing! {
    csr_adjacency: A -> B, many {
        on store { buckets: Vec<Vec<BId>>, offsets: Vec<u32>, adj: Vec<BId> }
        on_insert(a, b) { store.buckets[a].push(b) }   // forward insertion
        compact { /* buckets -> offsets + adj at the builder boundary */ }
        iter Walk { /* slice range over adj */ }
        iterate(a) { Walk::new(store.offsets[a] .. store.offsets[a + 1]) }
        iteration_order: forward_insertion
        phase: append_only_build, requires_compaction_at_boundary,
               freeze_scope: per_store, read_precondition: post_compaction
        validate { /* offsets len == A_len + 1, starts 0, ends adj.len(),
                      monotonic — assuming compaction has run */ }
    }
}
```

What each clause is load-bearing for:

- **`ctor_arg` (field).** A required field must be in the record literal
  at creation; a post-push hook cannot express it. "One constructor arg
  per total `one` arrow" (M1) becomes "one arg per `one` arrow whose
  backing contributes `ctor_arg`" — field is the only such binding in
  M3, so the M1/M2 constructor surfaces are unchanged.
- **`membership: existence` (arena).** No insert op is generated; the
  write path is `add_<entity>` itself. A primitive `many` arrow bound to
  `arena` is rejected (Constraints, below) — the fixed-table era never
  had to say this, because only entity arrows ever bound arena.
- **Hooks are id-space functions over `&mut` store state** — spike 11's
  shape (`insert_subnode_hook(&mut Store, host, main, sub)`), the M1/M2
  commitment that M4's flush replay builds on. The read side is
  id-space too (`read`, `lookup`); codegen wraps ids into handles for
  the trait impls.
- **`iter Walk` names the iterator** feeding the trait's GAT — a `many`
  backing must declare its iterator as a named struct; a bare
  `iterate(a) { … }` body is insufficient on its own (spike-verified:
  the named clause generates the renamed iterator struct plus its
  `Iterator` impl).

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
  associated type must name a concrete type (the `iter Walk` clause,
  above).

**Hygiene is proven, with three contractual caveats.** The hygiene
prototype (intrusive_list re-implemented through a `backing!`-style
courier, hook bodies in arbitrary Rust, negative probes for capture)
answered its gating question: hook bodies *do* survive the token channel
hygienically — but three of the guarantees are discipline the grammar
must enforce, not properties the language gives:

1. `Span::mixed_site()` on scaffold idents is load-bearing. It gives
   macro_rules-like local hygiene across both macro layers; the negative
   probe (call-site spans instead) demonstrably flips a scaffold local
   into capturing the hook body's call (E0618). Without mixed_site the
   DSL is unsound.
2. Item paths are not hygienic in Rust: a helper named in a hook body
   resolves at the backing's *use* site, not its declaration site. The
   grammar must either require helpers to be in scope at the use site or
   route them through explicit bindings.
3. Name coupling is contractual and must be written into the grammar:
   what each hook parameter names, which scaffold locals the `iterate`
   body may rely on, and the reserved set of placeholder idents (the
   substitution is textual). The prototype's one real bug was exactly a
   contract misreading of what a parameter named.

## The validate contribution

Every backing may carry `validate` clauses — arbitrary Rust over the same
roles and the same token channel as hook bodies (spike 5 proved the
channel; spike 4 proved the checks as plain functions). Codegen emits one
doc-hidden function per bound backing and summons them from the store's
`validate()` in declaration order. M2's canonical inventory remains the
*contract* each backing must satisfy — M4's flush runs exactly these —
and the stdlib re-implementation is graded against it: each built-in's
clause must reproduce the checks the fixed-table era hard-coded.

- field: endpoint-in-range over its contributed field.
- arena: nothing — existence cannot dangle.
- hash_index: key existence and value-in-arena, naming offending ids in
  the message (the probe's style). Key uniqueness is structural under
  `HashMap` — a documented obligation of the inventory, not an emitted
  check.
- intrusive_list: two bodies, selected by the arrow's instantiation
  facts — `validate by` (membership consistency against the determining
  field; codegen substitutes the determining-field binding) and
  `validate primitive` (the ownership partition walk, distinguishing
  shared membership from cycles in its report).
- csr_adjacency: offsets length/start/end/monotonicity, assuming
  compaction has run.

Nothing is derived from field shapes: derivation would need a container
table (which shapes get element-checked) — a hidden fixed table — and
cannot express the partition walk anyway. Authored clauses only.

## Constraints and applicability

A `requires:` clause draws from a **closed predicate vocabulary** that
`implement!` evaluates against schema facts and computed placement.
Checking is codegen's job, so the vocabulary is closed the way
cardinalities are closed; extending it is a macros-crate change.

- cardinality: `one` / `many` / `key(K)` — must match the root's.
- membership: `existence` — the arrow must be the entity arrow of its
  target (arena).
- placement predicates: `hosts_arena(A, B)` — B's arena is hosted by
  A's record, where a root-marked A's record is the store; and
  `direct_records` — both endpoint classes addressable without path
  indirection (satisfied today by root-level placements and the store).
- `exclusive_membership` (intrusive_list): link fields are sound only
  when each object belongs to one relation instance; shared membership
  requires a link-record design with its own arena (M2's refinement
  discussion).
- ids are `Copy + Eq + Hash` universally — documented, not checked.

Contract and phase annotations ride alongside, from spike 6's
vocabulary: `iteration_order` (a per-backing contract —
intrusive_list reverse-insertion, CSR forward-insertion; M4's flush
order contracts are stated against it), `append_only_build`,
`requires_compaction_at_boundary`, `freeze_scope: per_store`,
`read_precondition: post_compaction`, and store-level-only field
contributions.

Rejections quote the backing's tokens and state the placement facts —
the epic's cross-cutting mandate, not optional:

```text
error: backing `arena` on `tags` requires existence-derived membership: its
       iterate reads the arena of B hosted by the self record. `tags` is a
       primitive `many` arrow — bind `intrusive_list` or declare an entity arrow.
```

Cost hints stay deferred to M5 selection; the annotations pay for
themselves in error quality alone.

## CSR and phase representation

`csr_adjacency` is the second `many` backing — chosen because it has
genuinely different tradeoffs (mutation during construction vs. after)
and forces the backing interface to say something about build phases.
Its two-phase construction maps onto the builder boundary (epic
invariant 5): accumulate during build, compact to ranges at the
boundary; freeze is per-store, since the whole store flushes at once.

The representation question spike 6 left open is decided: **type-state
on one store type, emitted only when a phased backing is bound.** The
build-phase and frozen stores are one type parameterized by a phase
marker (`GraphStore<Build>` / `GraphStore<Frozen>` in the spike);
`freeze()` consumes the build-phase store and compacts (releasing the
accumulators at the type level); `finish()` is freeze then validate.
Insert-after-freeze and frozen-as-builder are compile errors — the
spike's compile-fail proofs — so `read_precondition: post_compaction`
is enforced by the absence of read traits on the build phase, not by a
flag. The spike's `FlagStore` comparison documents the runtime-flag
alternative as weaker on every axis except generated-code simplicity,
and generated code does not write itself.

Non-phased stores keep the M1 shape — the store is its own builder — so
the M1/M2-unchanged criterion holds; only stores binding a phased
backing gain the parameter, and handles exist only for the frozen
phase. Stated cost: a phased store has no read traits during build —
no mid-construction reads, stricter than M1's reborrow windows,
consistent with the pass discipline, paid only by phased stores.

M4 interaction (flagged, not rewritten there): flush step 6 is the
freeze — the transaction rebuilds through the build phase and finalizes
before step 7's `validate()`; the spike's `finish()` demonstrates the
ordering.

## Standard library refactor

**`Digraph` stays an inline instantiation; the composed re-expression is
dropped, deliberately.** The previous text anticipated dropping the
ceremony if degenerate composition failed to produce byte-identical
output. Spike 12 found something stronger: the re-expression is
*illegal*, not merely different. Composing the single-arrow roots with
the fusions `Digraph`'s entity structure requires is rejected at compose
time — `colocation class fuses multiple roots se.E, te.E (double
closure)` — and no fusion set escapes: unifying one sort across member
instances always puts two roots in one class or one sort's elements in
two. Two independent blockers besides: the composed grammar (instances
+ fusions) has no slot to declare a derived `sum` arrow, and the
placement rule would embed each root-fused member's arenas per host
record even if the checks were relaxed.

The scope statement this settles, folded into the epic: **composition
expresses juxtaposition, hosting, and reference closure — never
sort-sharing.** Every colocation class is a docking (one guest root plus
one host instance's elements), so composition nests whole patterns
inside others and juxtaposes them; sharing a population is expressible
only target-side, by `<->` id unification. A flat multi-arrow schema
over shared sorts is reachable only inside one `pattern!` block —
entity unification by name, M1's "composition-lite", which is why
inline instantiation *is* the degenerate composition. The one legal
fragment fusion marks the boundary: `Nodes + OutEdges` composes, but
the result stores `OutEdges`' edge arena per node — a per-node
adjacency layout. A different store, not `Digraph`.

The refactor step is therefore what remains real work:

- `Subgraph`, `EdgeChain`, `Bipartite`, `Hierarchy` ship as library
  patterns users compose, per the original vision: the toolkit's
  language defines its own standard library.
- The epic's "re-expressed as a composition in M3" is amended (this
  fold) to say assembly happens by inline instantiation.
- "Zero user-visible change" is vacuous — nothing about `Digraph`
  changes, and the M1 tests remain the regression suite for the
  `backing!` re-expression alone.

Frontier note (carried, not silently expanded): further composition of a
composed pattern — a user composing a shipped library pattern — is
untested (M2's frontier); it needs the nested member-schema grammar
(spike 1, F8). Shipping library patterns as composable makes that
grammar load-bearing when the first user needs it; schedule it
consciously.

## Stdlib promotion of M2's roots

M2's discovered roots — `EdgesOf`; the `main_*` one-arrows; the keyed
pair — move into the runtime stdlib, and pattern-local declarations
re-point by path. Roots register themselves either way (M1), so
promotion is vocabulary placement, not a mechanism change; the criterion
is the M2 suites unchanged modulo import paths. The value is trait
sharing: a second subgraph-like pattern instantiating the promoted
`main_node` root gets the *same* trait, which is what keeps generic
algorithms portable.

Names wait on M2's pending review decisions — the root-pattern-vs-entity
collision (`MainNode` names both an open entity and a root pattern in
M2's sketch; the spike used `MainNodeOf`), trait-name derivation, and
the keyed root's trait shape (spike 11 and the edge-induced probe chose
differently: concrete key parameter vs. associated key type). Flagged
here, decided there.

## Lesser decisions, folded

- **`union` is renamed `sum`** (this fold; plan-text edits only — the
  repo is still prose plus the reference implementation). The semantics
  are unchanged and were always multiset-sum: self-loops appear twice,
  out-edges chained with in-edges. The word `union` suggested dedup;
  `sum` does not.
- **Cost hints stay deferred** to M5 selection; the annotations pay for
  themselves in error quality alone.
- **Byte-identical vs. observably-identical degenerate composition** is
  moot under the dropped ceremony. Had composition stayed, the criterion
  would be observably-identical — byte-identity of generated code is
  not a stable contract.

## Frontier carried into M3

From M2's list, re-expressed under the premise: embedding depth 1;
contributed fields locating on embedded classes (uniform restriction,
above); intrusive lists sourced at unfused roots (DSL-legal, store-level
head, self-less insert op); and further composition of a composed
pattern (the grammar note above).

## Depends on

M2 complete — especially a hook interface that survived keyed indexes —
and M2's review outcomes for the promotion names. Do not start the
`backing!` grammar before the four built-ins exist; that ordering is the
whole point of the milestone.

## Task sequence (sketch)

1. `backing!` courier emission and resolution in `implement!`; delete
   the fixed table. Re-implement `field` and `arena` in the DSL; M1
   suite green.
2. `intrusive_list` in the DSL — the hygiene caveats as grammar rules;
   M1 suite green, traversal orders included.
3. `hash_index` in the DSL (the keyed grammar is M2's); M2 keyed tests
   green in both placements (host-embedded, store-level).
4. `validate` clauses; corruption fixtures per spike 4; the canonical
   inventory as documented contract.
5. Constraint predicates and trybuild error tests — messages quote
   tokens and state placement facts.
6. `csr_adjacency` and type-state phases; CSR tests with compile-fail
   proofs as in the spike.
7. Stdlib promotion (names per M2's outcomes); library patterns
   shipped; doc pass.
