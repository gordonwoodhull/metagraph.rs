# Milestone 3: user-facing backings, stdlib refactor

**Goal**: the backing layer becomes part of the language, and the standard
library becomes proof that the language suffices.

Specification level: partial — decided and sketched to grammar. Every
question this file carried as "deliberately vague" now has a folded
decision (grammar coverage, the validate mechanism, the constraint
language, phase representation, the stdlib refactor, stdlib promotion),
each grounded in a spike or probe. Uncompiled: the macro side of
`backing!` — grammar and codegen — as with M2's keyed grammar. One seam
is recorded untested rather than claimed: hook-body hygiene across the
token channel is spike-verified (spike 5), and courier resolution inside
`implement!` is spike-verified for *root metadata* (spike 1), but the
conjunction — `implement!` resolving a *backing binding* through a
backing courier and splicing its bodies — has never been bounced. Task 1
gates on that bounce before the fixed table is deleted. Spike-verified
islands: keyed mechanics in both embedded and store-level map placements
(spike 11; the edge-induced probe, 5/5 tests), CSR with its phase
annotations (spike 6), one authored validate body emitted at two
placements through bound roles (the poly-placement probe, 7/7), a
mixed-binding phased store (CSR + intrusive on one type-state store,
rebuilt flush-style — the mixed-phase probe, 5/5), instance-prefixed
merged constructors (the ctor-merge probe, 2/2 + E0063 negcheck), and
the composed-`Digraph` rejection that settles the stdlib refactor
(spike 12).

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
| hash_index map placement: root-level self record, host-embedded record, or store | (a) — contributions target the self record, per the three-case self taxonomy (placement rules below) | M1 naming rule; spike 11 (host record); edge-induced probe (store-level) |
| per-backing validate checks (key existence, value-in-arena, link well-formedness, membership consistency, partition, CSR offsets) | (a) — the validate clause (below) | M2 canonical inventory; spike 4; edge-induced probe; spike 6 |
| placement computation; entity kits (`Id(pub u32)`, compound path ids under embedding); hand-written handle `Eq`/`Hash` by (store pointer, id); the function-per-hook shape; stem-namespacing and collision prefixing of contributed names | (b) — uniform, backing-independent | M2 placement rule; M2 ("`NodeId`'s `pub u32` stays — the tests depend on it"); spike 11 finding 4 |
| arena bound to an arrow whose target's arena is hosted elsewhere | (a) — arena's declared requirement (`hosts_arena`) | M2 frontier; Hierarchy is the legal shape |
| intrusive sourced at an embedded class | (b) — uniform restriction: contributed fields locate only on root-level classes or the store in M3. Separately, intrusive_list declares the stronger (a) predicate `direct_records` on its *endpoints* (a different condition with a different extension — hash_index's host-embedded map legally sits on an embedded configuration) | M2 frontier |
| intrusive sourced at an unfused root | (a) — DSL-legal: the unfused root's record is the store, so the head is a store-level field and the insert op drops the self argument; (b) — behind the codegen frontier, and unexercised (the edge-induced probe covered a store-level *map*, never a store-level head) — owed a probe before emission | M2 write-ops rule; M3 placement rules |

The boundary the table draws: the (a) half is what a `backing!`
declaration *states*; the (b) half is the uniform resolution of `on`
targets and roles to locations, plus emission conventions no declaration
may vary. If a behavior cannot be filed under one of the two, the DSL
has a hidden fixed table and the premise fails.

Three placement-resolution rules make the table work, all uniform:

- **Contributions target the self record.** "The self record" is a
  three-case taxonomy (M1's naming rule, applied to backings): a *plain*
  self has its own class record; a *root-marked and fused* self's record
  is the fusion-class record it is hosted by (spike 11: the map sits on
  `OuterNodeData` beside the embedded arena — the root-marked `sub`
  self is fused into `outer`'s element, so its record is the host
  element's record); a *root-marked and unfused* self's record is the
  store (the edge-induced probe: `sub.Subgraph` is root-marked and
  unfused, so `on A` resolves to the store, and no self id exists).
  `on store` (CSR) is the explicit form of the same slot. One rule
  covers hash_index's three observed placements.
- **Field values may be path ids** (spike 11 finding 1: path ids derive
  `Copy`/`Eq`/`Hash` unchanged and work as map values). The M2 frontier
  item "field backing into an embedded class rejected (its id needs a
  host path)" was a spike-1 codegen limit, not a semantic rule — it is
  codegen work, not a constraint, and M3 does not re-add it as one.
- **A root-marked and *unfused* self contributes no id**: the op and
  hook drop the self parameter, and *bare* `a` outside role expressions
  is grammar-checked away. Role expressions mentioning `a` — `rec(a)`,
  `b_arena_of(a)`, `map_from(a)` — are exempt: they are placement-
  abstract placeholders, and at this placement codegen folds them to
  their store-level forms (`store.b_arena`, `store.index`), so `a`'s
  value is never needed. The edge-induced probe pinned the parameter
  side — "a 3-arg form `insert_subnode(self_id, key, target)` is
  unemittable because the root-marked self owns no id" — and the
  poly-placement probe pinned the role side (one authored body, two
  placements, only the bound roles differ).

## `backing!` grammar

**Bodies are ordinary Rust over roles; codegen binds the roles per
placement.** The role vocabulary is closed — every ident a body may name
is in this table, and the grammar rejects anything else (spike 5's
caveat 3: the substitution is textual, so bodies may not shadow the
reserved set):

| role | meaning |
|---|---|
| `a`, `b`, `k` | the hook's id parameters (self, target, key). Subject to the self rule above: at a root-marked-and-unfused self there is no `a`, and role expressions mentioning it fold to their store-level forms |
| `rec(x)` | the record of id `x` through its placement-correct path — flat: `store.x_arena[i]`; embedded: `store.host_arena[h].x_arena[i]`; reference-fused: resolved through the partner map to the partner class's path (M2) |
| `b_arena_of(a)`, `map_from(a)` | "the arena/map hosted by `a`'s record", per the three-case self taxonomy (a fused root-marked self's record is the class record; an unfused root-marked self's record is the store) |
| `records(E)` | placement-polymorphic enumeration of every record of entity `E`, wherever placed — (host path, index) pairs under embedding, degenerating to a flat index range at root level. What "every self record's …, wherever placed" iterates |
| `maps(A)` | every map the backing contributed for arrow self `A`, wherever placed — one store-level map, or one per host record (the poly-placement probe's `$maps` role) |
| `store` | the store value binding available to every body |
| `AId`, `BId`, `KId` | the id types of the declaration's entity variables |
| `arena_len(E)` | the length of `E`'s arena at its placement |
| `idx(id)` | id newtype → `usize`. Bodies do no newtype arithmetic themselves; codegen emits `idx()` on every id newtype and rewrites indexing (`store.buckets[idx(a)]`, `store.offsets[idx(a) + 1]`) |

Contributed field names (`value`, `first`, `next`, `index`) are
reserved stems bound at emission to this arrow's namespaced fields, by
one mechanical rule: a backing's *single* contributed field on a record
takes the arrow's name (`source: NodeId`); a backing with *several*
contributions on one record emits `stem_arrow` (`first_out`, `next_out`);
under composition the instance prefix applies on collision
(`outer_source`, matching `add_outer_node`/`outer_node_arena`). M1's
naming tables carry over unchanged.

Each `validate` body is emitted as one doc-hidden function per bound
backing, `fn __validate_<arrow>(store: &Store) -> Result<(), String>`,
summoned from the store's `validate()` in the `implement!` block's
binding order. Errors name offending ids in the message text (the
edge-induced probe's style).

The stdlib's five declarations:

```rust
// field — M1's source/target
backing! {
    field: A -> B, one {
        on A { value: BId }
        ctor_arg        // feeds the record literal at creation; the ctor arg
                        // is named after the arrow (add_edge(source, target))
        read(a) { rec(a).value }     // id-space; codegen wraps into the B handle
        validate { for ra in records(A) {
            /* idx(ra.value) < arena_len(B), else name the offending id */
        } }
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
        validate by {          // membership consistency: each B appears
            /* exactly once in the list that determining(a) names —
               determining(a) is codegen-bound to the by-arrow's field */
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
                                            // is root-marked and unfused
        on_insert(a, k, b) {
            // live keys rejected at the call (M2): the hook checks first
            assert!(map_from(a).insert(k, b).is_none(),
                    "duplicate key {k:?}");
        }
        lookup(a, k) { map_from(a).get(&k).copied() }
        validate {          // every self record's map, wherever placed:
            for m in maps(A) {
                /* per entry (k, v): key existence (idx(k) < arena_len(K))
                   and value-in-arena (placement-correct containment of v),
                   naming offending ids in the message */
            }
        }
    }
}

// csr_adjacency — the second `many` backing (spike 6)
backing! {
    csr_adjacency: A -> B, many {
        on store build  { buckets: Vec<Vec<BId>> }          // build-phase only
        on store frozen { offsets: Vec<u32>, adj: Vec<BId> } // frozen-phase only
        on_add(a) { store.buckets.push(Vec::new()) }   // one slot per A record
        on_insert(a, b) { store.buckets[idx(a)].push(b) }  // forward insertion
        compact { /* buckets -> offsets + adj at the builder boundary;
                     consumes the accumulators (a frozen store carries no
                     build state — the mixed-phase probe's stale-buckets
                     hazard is designed out, not asserted against) */ }
        iter Walk { lo: u32, hi: u32 }   // codegen adds the store ref
        iterate(a) { Walk { lo: store.offsets[idx(a)],
                            hi: store.offsets[idx(a) + 1] } }
        iteration_order: forward_insertion
        phase: append_only_build, requires_compaction_at_boundary,
               freeze_scope: per_store, read_precondition: post_compaction
        requires: direct_records      // buckets/offsets index by dense A ids;
                                      // a path-id self is rejected here, not
                                      // as a codegen type error
        validate { /* offsets len == arena_len(A) + 1, starts 0, ends
                      adj.len(), monotonic — assuming compaction has run */ }
    }
}
```

What each clause is load-bearing for:

- **`ctor_arg` (field).** A required field must be in the record literal
  at creation; a post-push hook cannot express it. "One constructor arg
  per total `one` arrow" (M1) becomes "one arg per *total* `one` arrow
  whose backing contributes `ctor_arg`" — `ctor_arg` requires totality
  (an optional `one` arrow's field is default-initialized and updated by
  M4's `set_`). Field is the only such binding in M3, so the M1/M2
  constructor surfaces are unchanged. Under composition the merged
  constructor takes the union of every member's `ctor_arg` arguments;
  colliding argument names are *shared* when the two members' resolved
  target classes agree (one argument feeds both fields — the `<->`
  partner map usually guarantees agreement) and *instance-prefixed*
  when they differ (`outer_source`, per M2's collision-first naming;
  probe-verified by the ctor-merge probe) — spike 1's "silently
  deduped (keeps first)" limit is unemittable for required fields
  (E0063, confirmed by the probe's negcheck) and is retired.
- **`membership: existence` (arena).** No insert op is generated; the
  write path is `add_<entity>` itself. A primitive `many` arrow bound to
  `arena` is rejected (Constraints, below) — the fixed-table era never
  had to say this, because only entity arrows ever bound arena.
- **`on_add(a)` (csr).** Store-level per-self-indexed contributions
  (`buckets`) must grow with the self arena; the hook fires from
  `add_<A>` orchestration exactly as insert hooks fire from record
  constructors (M1's commitment extended to entity constructors). The
  spike maintained this invariant hand-inlined in `add_node`; without
  the clause the declaration panics on first insert or fails its own
  validate.
- **Phase attribution on contributions (`on store build` / `on store
  frozen`).** The type-state store needs to know which fields exist in
  which phase; the spike split them by comment, which two legal
  emissions could disagree on. The attribution is the partition, and it
  is what makes the compile-fail guarantees (no insert hooks, no
  accumulators on the frozen type) derivable rather than asserted.
- **Hooks are id-space functions over `&mut` store state** — spike 11's
  shape (`insert_subnode_hook(&mut Store, host, main, sub)`), the M1/M2
  commitment that M4's flush replay builds on. The read side is
  id-space too (`read`, `lookup`); codegen wraps ids into handles for
  the trait impls.
- **`iter Walk` names the iterator** feeding the trait's GAT — a `many`
  backing must declare its iterator as a named struct with concrete
  fields; a bare `iterate(a) { … }` body is insufficient on its own
  (spike-verified: the named clause generates the renamed iterator
  struct plus its `Iterator` impl).

Two mechanical obligations come with user-writable backings:

- **Token routing.** A backing declaration has the same cross-item
  problem a pattern has: `implement!` must know a backing's field
  contributions and hooks at expansion time, and proc macros cannot read
  other items' tokens. `backing!` therefore emits a *backing courier* —
  the same target-parameterized single-rule courier as patterns and
  roots — embedding its field declarations, hook bodies, and constraint
  annotations; `implement!` resolves a backing binding by invoking its
  courier. No fixed table of backings, same as no fixed table of roots.
  The courier inherits M1's landed constraints: `#[macro_export]`
  anchors it at the defining crate's root, so a `backing!` declaration
  at a crate root E0255-collides with its own courier — declare backings
  in a named module (M1's documented constraint, same mechanism).
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
channel; spike 4 proved the checks as plain functions; the poly-placement
probe proved one authored body emits at two placements through bound
roles). Emission follows the convention paragraph: one doc-hidden
function per bound backing, summoned from the store's `validate()` in
the `implement!` block's binding order. M2's canonical inventory remains
the *contract* each backing must satisfy — M4's flush runs exactly these —
and the stdlib re-implementation is graded against it: each built-in's
clause must reproduce the checks the fixed-table era hard-coded.

- field: endpoint-in-range over its contributed field.
- arena: nothing — existence cannot dangle.
- hash_index: key existence and value-in-arena, naming offending ids in
  the message (the edge-induced probe's style). Key uniqueness is not an
  emitted check — a per-self `HashMap` cannot hold two values for one
  key, and a plain insert silently overwrites with `validate()` still
  passing (the edge-induced probe demonstrates both). Uniqueness is
  enforced where the op is submitted: the insert hook checks the map
  first, and the builder method / M4's staging call reject a live key
  (M2's keyed section) — uniqueness holds by construction thereafter.
- intrusive_list: two bodies, selected by the arrow's instantiation
  facts — `validate by` (membership consistency against the determining
  field; codegen binds the reserved `determining(a)` placeholder to the
  by-arrow's emitted field, resolving the stem through the *other*
  backing's contribution — hygiene caveat 3 requires exactly this
  contract to be named) and `validate primitive` (the ownership
  partition walk, distinguishing shared membership from cycles in its
  report).
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

- cardinality: `one` / `many` / `optional, key(K)` — must match the
  root's.
- membership: `existence` — the arrow must be the *entity arrow* of its
  target. Defined mechanically: B's entity arrow is the unique arrow
  targeting B that is bound to a `membership: existence` backing and for
  which `hosts_arena` holds. Placement is computed from fusion classes
  at compose time, before any predicate is evaluated, so this is not
  circular with `hosts_arena`. Uniqueness is part of the rule: a second
  existence binding on the same target is an `implement!` error quoting
  both arrows.
- placement predicates: `hosts_arena(A, B)` — B's arena is hosted by
  A's record, per the three-case self taxonomy (plain self: B's arena
  embeds in A's class record; fused root-marked self: A's record is the
  class record it is hosted by; unfused root-marked self: A's record is
  the store, and the predicate is satisfied by any root-level B arena);
  and `direct_records` — both endpoint classes addressable without path
  indirection (satisfied today by root-level placements and the store).
- `exclusive_membership` (intrusive_list): evaluated as a bindings fact —
  rejected when two intrusive link structures would thread through the
  same target record sort (two intrusive bindings sharing a target
  entity). Link fields are sound only when each object belongs to one
  relation instance; shared membership requires a link-record design
  with its own arena (M2's refinement discussion).
- ids are `Copy + Eq + Hash` universally — documented, not checked.

Contract and phase annotations ride alongside, from spike 6's
vocabulary: `iteration_order` (a per-backing contract —
intrusive_list reverse-insertion, CSR forward-insertion; M4's flush
order contracts are stated against it), `append_only_build` (spike 6's
`append_only`, renamed in the declaration), `requires_compaction_at_boundary`,
`freeze_scope: per_store`, `read_precondition: post_compaction`. Spike
6's `field_contributions: store_level_only` is absorbed by the
phase-attributed `on store build/frozen` clauses rather than carried as
a separate annotation.

Each predicate's evaluation is pinned to an error template naming the
computed placement facts — the epic's cross-cutting mandate, applied to
the new vocabulary:

| predicate | inputs | rejects when | error names |
|---|---|---|---|
| cardinality | the root's declared cardinality, the backing's | mismatch | the arrow, both cardinalities |
| `existence` | bindings, computed placement | not the unique existence arrow of B (see above) | both arrows on a duplicate; the arrow and its membership kind otherwise |
| `hosts_arena(A, B)` | computed placement of B's arena, A's self case | B's arena is not hosted by A's record per the taxonomy | the arrow, B's computed arena location, A's self case |
| `direct_records` | placements of both endpoint classes | either endpoint needs path indirection | the arrow, both endpoints' placements |
| `exclusive_membership` | the set of intrusive bindings | two intrusive bindings share a target record sort | both arrows, the shared target |

Rejections quote the backing's tokens as well — spans have degraded by
the time the check runs:

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

The combination with non-phased backings on the same store is
probe-verified (the mixed-phase probe, 5/5): CSR and intrusive_list
coexist on one `GraphStore<P>` — one `add_edge` fires both backings'
hooks, per-record link fields cross the boundary untouched inside the
shared arenas, `freeze` finalizes only the phased backing's store-level
state, and each backing's validate function catches its own damage.
The probe also pinned the hazard the phase-attributed contributions
design out: with one undifferentiated field list, a forgetful `freeze`
silently carries stale accumulators into the frozen store; with
`on store build` / `on store frozen` partitions there is nothing stale
to carry.

The id-based read view (M2's builder shape) on a Build-phase store
covers non-phased backings only — field reads and other build-time-final
contributions. A phased backing contributes no build-phase getters:
its store-level state is pre-compaction buckets, and reading it would
violate `read_precondition: post_compaction` by construction. M4's
flush replay reads through this view against the Build-phase store it
is rebuilding; that is the whole of the interaction.

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
sort-sharing.** Every colocation class is a docking: exactly one
guest root-marked entity plus element entities of exactly one *other*
instance (the root's own instance may not contribute elements — a
same-instance fusion, `outer.Node <=> outer.Edge`, is rejected at
compose time; see M2's checks). Composition nests whole patterns
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

The naming gate is discharged — M2's review decided all three questions
(6290959 on main): root patterns take the arrow's name with no suffix
where free (`MainNode`/`MainEdge`; spike 1's `MainNodeOf` was a
deviation), and `-Of` stays where the bare name is taken (`SubNodeOf`,
`SubEdgeOf`, `EdgesOf`); the root's trait takes its *arrow's* name
camel-cased, not the pattern's (M2's keyed sketch trait is `Subnode<M>`);
and the keyed root's trait shape settled on the associated `Key` type
(the edge-induced probe's shape; spike 11's concrete parameter was the
other candidate). Promotion uses these names as-is.

## Lesser decisions, folded

- **`union` is renamed `sum`** (this fold; plan-text edits only). The
  rename predates M1's implementation landing — the landed code still
  spells the derivation `union` (`macros/src/grammar.rs`,
  `src/stdlib.rs`); a mechanical code rename rides with whichever
  milestone next touches the grammar. The semantics
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
above); intrusive lists sourced at unfused roots (DSL-legal — store-level
head, self-less insert op — but unexercised; owed a probe before
emission); `<=>` transitivity exercised only on
two-member classes (M2's frontier, still uncarried); and further
composition of a composed pattern (the grammar note above). Retired in
this fold: spike 1's "constructor argument name collisions silently
deduped" (unemittable for required fields — E0063; colliding `ctor_arg`
contributions are shared when target classes agree and instance-prefixed
when they differ, per the grammar section)
and "field backing into an embedded class rejected" (a spike-1 codegen
limit, not a rule — see the placement rules).

## Depends on

M2 complete — especially a hook interface that survived keyed indexes.
M2's review outcomes for the promotion names are in (see Stdlib
promotion). Do not start the
`backing!` grammar before the four built-ins exist; that ordering is the
whole point of the milestone.

## Task sequence (sketch)

1. `backing!` courier emission and resolution in `implement!`; delete
   the fixed table. **Acceptance gate, before the table is deleted:** a
   ~100-line bounce probe — two distinct backing couriers resolved per
   one `implement!`-shaped invocation, bodies spliced hygienically.
   The hygiene half is spike-5-proven and the resolution half is
   spike-1-proven, but their conjunction has never been bounced (the
   header's recorded seam). Then re-implement `field` and `arena` in
   the DSL; M1 suite green.
2. `intrusive_list` in the DSL — the hygiene caveats as grammar rules;
   M1 suite green, traversal orders included.
3. `hash_index` in the DSL (**gated on M2's keyed grammar** landing; the
   names are settled); M2 keyed tests green in both placements (host-embedded,
   store-level).
4. `validate` clauses; corruption fixtures per spike 4; the canonical
   inventory as documented contract.
5. Constraint predicates and trybuild error tests — messages quote
   tokens and state placement facts per the evaluation table above.
6. `csr_adjacency` and type-state phases; CSR tests with compile-fail
   proofs as in the spike.
7. Stdlib promotion (names per M2's resolved outcomes, above); library
   patterns shipped; doc pass.
