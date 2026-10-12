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
`implement!` is landed, tested code since M2 merged (the downstream
renamed/indirect crates in its suite), but the conjunction —
`implement!` resolving a *backing binding* through a backing courier and
splicing its bodies — has never been bounced. The bounce gate —
cross-crate arms included — runs before the fixed table is deleted, as
step 2's permanent in-repo test. Round 4 planned a throwaway probe for
it (/tmp/m3-probe-r4-bounce, the round's one authorized proc-macro
exception) and superseded the probe once M2 merged: with the resolution
half landed and the hygiene half spike-proven, the permanent gate is the
first-hand evidence and a stand-in model of tested mechanisms buys
calendar, not design — the gate's arm list gains the use-site
helper-resolution arm (spike 5's caveat 2) so it covers the whole
conjunction. The round's other recorded seam retired by probe: the
`advance(w)` clause, grounded until now only in the landed codegen's
hard-coded `next()` bodies, authored through the DSL and emitted at the
embedded placement (/tmp/m3-probe-r4-advance, 5/5). Still admitted but
unemitted: csr at a fused root-marked self (Frontier). Spike-verified
islands: keyed mechanics in both embedded and store-level map placements
(spike 11; the edge-induced probe, 5/5 tests), CSR with its phase
annotations (spike 6), one authored validate body emitted at two
placements through bound roles (the poly-placement probe, 7/7), a
mixed-binding phased store (CSR + intrusive on one type-state store,
rebuilt flush-style — the mixed-phase probe, 5/5), merged constructors
under the combined collision rule — both halves: instance-prefixed when
targets differ (the ctor-merge probe, 2/2 + E0063 negcheck) and
shared-argument when they agree (review 3, /tmp/m3-probe-r3-ctor-share,
4/4), and
the composed-`Digraph` rejection that settles the stdlib refactor
(spike 12). Review 3 added: hash_index emission-graded at all three
self placements including the never-before-emitted plain self
(/tmp/m3-probe-r3-hashindex, 11/11), the value-host-agreement check
made expressible through owner-yielding maps and a `host_of` role in
one placement-uniform body (/tmp/m3-probe-r3-hostagree, 5/5), and the
store-level intrusive head — the shape this plan calls DSL-legal —
emitted, read through the root handle, and partition-validated
(/tmp/m3-probe-r3-root-intrusive, 5/5).

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
| intrusive head/link fields, push-front hook, reverse-insertion order | (a) | M1 backings table — "part of the backing's contract, not an accident"; spike 5 |
| exclusive membership *within one arrow's lists* | (a) — a declared contract annotation, runtime-enforced by the two validate bodies; never a bindings-level rejection (two intrusive bindings on one target are legal — Digraph's `out_edges` + `in_edges` — per-arrow stem-namespacing keeps the link fields distinct) | M2 canonical inventory: "surfaced as a constraint annotation when M3's `backing!` DSL lands" |
| field's field-on-self-record, constructor arg, direct read | (a) — the `ctor_arg` clause | M1 backings table |
| arena: no fields, no hooks, existence membership, iterator over the target arena | (a), including the declared requirement `hosts_arena` | M1 backings table |
| hash_index map placement: root-level self record, host-embedded record, or store | (a) — contributions target the self record, per the three-case self taxonomy (placement rules below) | M1 naming rule; spike 11 (host record); edge-induced probe (store-level) |
| per-backing validate checks (key existence, value-in-arena, value-host agreement, link well-formedness, membership consistency, partition, CSR offsets) | (a) — the validate clause (below) | M2 canonical inventory; spike 4; edge-induced probe; spike 6 |
| placement computation; entity kits (`Id(pub u32)`, compound path ids under embedding); hand-written handle `Eq`/`Hash` by (store pointer, id); the named doc-hidden hook method per write path; collision prefixing of contributed names (the prefix names the colliding scope — arrow-stem within one instance, instance prefix across composition; grammar section) | (b) — uniform, backing-independent | M2 placement rule; M2 ("`NodeId`'s `pub u32` stays — the tests depend on it"); spike 11 finding 4 |
| the *stem formulas* that name contributed fields (arrow name for `field`; `stem_` + `link_stem(arrow)` for `intrusive_list`; `<arrow>_index` for `hash_index` — the instantiation's *local* arrow name, as in field's row; declared stem for csr) | (a) — declaration-specified, pinned per backing (the grammar section's table); there is no single mechanical rule | the four artifacts' pinned names: `source`, `first_out`/`next_out` (grammar.rs `link_stem`), `subnode_index` (spike 11, edge-induced probe, round-3 probe), `buckets` (spike 6) |
| arena bound to an arrow whose target's arena is hosted elsewhere | (a) — arena's declared requirement (`hosts_arena`) | M2 frontier; Hierarchy is the legal shape |
| intrusive sourced at an embedded class | (b) — uniform restriction: contributed fields locate only on root-level classes or the store in M3. Separately, intrusive_list declares the stronger (a) predicate `direct_records` on its *endpoints* (a different condition with a different extension — hash_index's host-embedded map legally sits on an embedded configuration) | M2 frontier (reclassified at the round-4 merge) |
| intrusive sourced at an unfused root | (a) — DSL-legal: the unfused root's record is the store, so the head is a store-level field, the hook is self-less, the op is a **store-level method** (never a root-handle method — the root handle wraps a shared `&Store`), and the read rides the root handle. Probe-verified end to end (/tmp/m3-probe-r3-root-intrusive, 5/5), including coexistence with per-record lists on one store and the partition validators | M2 write-ops rule; M3 placement rules; the root-intrusive probe |

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
- **Hook-maintained field values may be path ids** (spike 11 finding 1:
  path ids derive `Copy`/`Eq`/`Hash` unchanged and work as map values; the
  map is written by its insert hook at a known host, which is exactly
  M2's addressability exemption for hook-maintained contributions). M2's
  addressability check itself stands, unamended: a `ctor_arg`-carrying
  field whose value would reference an embedded class is still rejected
  at `implement!` time, because "the embedded class's ids are
  host-anchored paths the constructor argument cannot supply" (main's
  M2, implement-time checks). What the M2 frontier item *was* — a
  spike-1 codegen limit — is the emission of hook-maintained path-id
  values, which M3 grows; the rejection of constructor-fed path-id
  fields is a rule, and M3 keeps it.
- **A root-marked and *unfused* self contributes no id**: every emitted
  signature — op, hook, read, lookup — drops the self parameter (the
  root-intrusive and edge-induced probes pin write and read sides
  alike), the op is emitted as a store-level method (never a
  root-handle method: the root handle wraps a shared `&Store` — probe
  3's pinned clarification), and *bare* `a` outside role expressions
  is grammar-checked away. Role expressions mentioning `a` — `rec(a)`,
  `b_arena_of(a)`, `map_from(a)`, `idx(a)`, and store-field indexing
  through `a` — are exempt: they are placement-
  abstract placeholders, and at this placement codegen folds them to
  their store-level forms (`store.b_arena`, `store.<arrow>_index`), so `a`'s
  value is never needed. The edge-induced probe pinned the parameter
  side — "a 3-arg form `insert_subnode(self_id, key, target)` is
  unemittable because the root-marked self owns no id" — and the
  poly-placement probe pinned the role side (one authored body, two
  placements, only the bound roles differ).

## `backing!` grammar

**Bodies are ordinary Rust over roles; codegen binds the roles per
placement.** The role vocabulary is closed — every ident a body may name
is in this table or the reserved-placeholder list below it, and the
grammar rejects anything else (spike 5's
caveat 3: the substitution is textual, so bodies may not shadow the
reserved set):

| role | meaning |
|---|---|
| `a`, `b`, `k` | the hook's id parameters (self, target, key). Subject to the self rule above: at a root-marked-and-unfused self there is no `a`, and role expressions mentioning it fold to their store-level forms |
| `rec(x)` | the record of id `x` through its placement-correct path — flat: `store.x_arena[i]`; embedded: `store.host_arena[h].x_arena[i]`; reference-fused: resolved through the partner map to the partner class's path (M2) |
| `b_arena_of(a)`, `map_from(a)` | "the arena/map hosted by `a`'s record", per the three-case self taxonomy (a fused root-marked self's record is the class record; an unfused root-marked self's record is the store). `b_arena_of` generalizes per entity variable (`k_arena_of` for the key), and like `host_of` it has one definition per placement — at the vacuous-host form (flat ids) it yields the root-level arena of its entity, which is what makes composed containment placement-uniform: `idx(v) < b_arena_of(host_of(v)).len()` is one body at every placement (round 4 — the poly-placement probe verified this check through per-placement bound roles, `$v_in_arena`; the composed role form is its DSL spelling) |
| `records(E)` | placement-polymorphic enumeration of the **ids** of entity `E`'s records, wherever placed — path ids under embedding, flat ids at root level. Bodies reach the record through `rec(id)`; the id is what error messages name (round-3 repair: the old "(host path, index) pairs / flat index range" wording made `ra.value`-style bodies untypeable and left the error convention without an id) |
| `maps(A)` | every map the backing contributed for arrow self `A`, wherever placed, as **`(owner, map)` pairs** — one store-level pair whose owner is the vacuous host, one pair per self record at a plain self (owner the vacuous host — flat ids carry no host component), or one per host record with the host id as owner. The pair shape is what makes value-host agreement expressible (the host-agreement probe, 5/5; the poly-placement probe's bare-map `$maps` role could not — its honest-limits item 2) |
| `host_of(v)` | the host component of an id — a path id's host id, or the vacuous host for flat ids. One definition per placement (no overloading — E0428; codegen picks), so with owner-yielding `maps(A)` the value-host-agreement check is one placement-uniform comparison, vacuously true at the store-level placement |
| `store` | the store value binding available to every body |
| `AId`, `BId`, `KId` | the id types of the declaration's entity variables — placement-parametric per the self taxonomy: at a plain self `AId` is A's own id type; at a fused root-marked self it is the host's id type; at an unfused root-marked self it does not exist |
| `arena_len(E)` | the length of `E`'s arena — legal where `E`'s arena is unique (root level); under embedding each host record owns its own arena, the per-host spelling is `b_arena_of(a).len()`, and `arena_len(E)` is ill-defined there (which host's?) and grammar-rejected |
| `idx(id)` | id newtype → `usize`. Bodies do no newtype arithmetic themselves; codegen emits `idx()` on every id newtype and rewrites indexing (`store.buckets[idx(a)]`, `store.offsets[idx(a) + 1]`). On a path id, `idx` is the leaf index; the host component is read through `host_of` |

Reserved placeholders (the second closed set, named here so the
closed-vocabulary preamble holds): `determining(a)`, bound by codegen to
the by-arrow's emitted field (see intrusive_list's `validate by`); and
`handle(x)`, legal in `advance` bodies — it mints the target's handle at
this placement from the Walk's reserved store ref and, at embedded
placements, the host path the Walk carries, taking an id or a raw index
(arena's Walk counts indices). The landed codegen's mint —
`TargetHandle { store, id }`, path form `(host, index)` — is exactly
this operation (round 4: both advance sketches used `handle(...)` while
the table omitted it, breaking the closed-vocabulary preamble the
clause was written under).

Contributed field names are part of each backing's declared contract,
stated as a per-site formula the plan pins per backing — there is no
single mechanical rule (the round-3 emission grading found every
counting reading of the old single/several rule mispredicts at least one
pinned name, and the artifacts' orders differ: `first_out` is
stem-first, `subnode_index` arrow-first):

| backing | contributed field | emitted name |
|---|---|---|
| field | `value` on A | the arrow's local name (`source`) — M1's naming table |
| intrusive_list | `first` on A, `next` on B | `stem_` + the arrow's stem via M1's `link_stem` `_edges`-strip (`first_out`, `next_out`) |
| hash_index | `index` on A | the instantiation's local arrow name + `_index` (`subnode_index` — spike 11, the edge-induced probe, the round-3 probe; "local" pinned explicitly in round 4, matching field's row) |
| csr_adjacency | `buckets`/`offsets`/`adj` on the store | the declared stem, bare when unambiguous |

Collisions — two contributions deriving one name on one record — are
resolved by a prefix naming the colliding scope (round 4 split what the
old text called one "arrow-stem prefix" into its two mechanisms):
within one pattern instance, the arrow-stem prefix (`buckets_out`/
`buckets_in` for two csr bindings on one store); across composition
instances, the instance prefix (`outer_source`, matching
`add_outer_node`/`outer_node_arena` — `outer` is the instance there,
not a stem). The cross-instance arm is reachable, not exotic: several
unfused roots on one store are legal (M2), and two unfused-root
instances of one keyed pattern each contribute a store-level
`<arrow>_index` — identical stems, so the instance prefix is what
distinguishes them. The collision rule is the (b)
uniform half. M1's naming tables carry over unchanged for field and
intrusive_list.

Each `validate` body is emitted as one doc-hidden method per bound
backing on the store it validates (`fn __validate_<arrow>(&self) ->
Result<(), String>` — a method, not a free function: two `implement!`
sites in one module would collide on the free-function name; the
round-3 probe recorded the collision and suffixed its hand-emitted names
around it — `__validate_subnode`, `__validate_subnode_hier`,
`__validate_subnode_edge`), summoned from the store's `validate()`
in the `implement!` block's
binding order. Errors name offending ids in the message text (the
edge-induced probe's style).

The stdlib's five declarations:

```rust
// field — M1's source/target
backing! {
    field: A -> B, one {
        on A { value: BId }
        ctor_arg        // feeds the record literal at creation; the ctor arg
                        // is named after the arrow (add_edge(source, target)).
                        // This clause is M3's expression of M2's field-write
                        // hook: at construction it is the field's single
                        // write path (M2 grows the named hook in flat
                        // codegen; set_-time re-pointing is M4 vocabulary)
        read(a) { rec(a).value }     // id-space; codegen wraps into the B handle
        validate { for ra in records(A) {
            /* endpoint-in-range, placement-uniform:
               idx(rec(ra).value)
                 < b_arena_of(host_of(rec(ra).value)).len()
               (at flat ids the vacuous-host b_arena_of is the root-level
               B arena, so the body degenerates to arena_len(B) there),
               naming ra in the message */
        } }
    }
}

// arena — M1's nodes
backing! {
    arena: A -> B, many {
        membership: existence        // no insert op; the write path is add_<B>
        iter Walk { cur: u32, len: u32 }   // codegen adds the store ref, and
                                           // at embedded placements the host
                                           // path — the reserved iterator
                                           // fields (r4-advance: the index
                                           // mint needs the host; the link
                                           // walk takes it from the path id
                                           // and leaves the field unused)
        iterate(a) { Walk { cur: 0, len: b_arena_of(a).len() } }
        advance(w) { /* (w.cur < w.len).then(|| { let n = handle(w.cur);
                          w.cur += 1; n }) — handles minted from w.cur and,
                          at embedded placements, the host path w carries */ }
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
        advance(w) { /* let id = w.cur?; w.cur = rec(id).next; handle(id) */ }
        iteration_order: reverse_insertion
        contract: exclusive_membership   // a declared annotation, not a
                                         // bindings predicate — see Constraints
        requires: direct_records
        validate by {          // membership consistency: each B appears
            /* exactly once in the list that determining(a) names —
               determining(a) is codegen-bound to the by-arrow's field
               (a reserved placeholder, role table) */
        }
        validate primitive {   // the ownership partition walk: per-list and
            /* global seen sets; distinguishes shared membership from cycles */
        }
    }
}

// hash_index — M2's subnode / subedge
backing! {
    hash_index: A -> B, key(K) {
        on A { index: HashMap<KId, BId> }   // emits <arrow>_index; the self
                                            // record, or the store when A is
                                            // root-marked and unfused
        on_insert(a, k, b) {
            // live keys rejected at the call (M2) — check-then-write, so
            // M4's staging can call the predicate without the write:
            assert!(map_from(a).get(&k).is_none(),
                    "duplicate key {k:?}");
            map_from(a).insert(k, b);
        }
        lookup(a, k) { map_from(a).get(&k).copied() }
        validate {          // every self record's map, wherever placed:
            for (owner, m) in maps(A) {
                /* per entry (k, v), all three placement-uniform:
                   key existence (idx(k) < k_arena_of(host_of(k)).len()),
                   value-in-arena (idx(v) < b_arena_of(host_of(v)).len()),
                   value-host agreement (host_of(v) == owner — vacuous
                   wherever ids are flat: plain self and store level
                   alike, the vacuous host is each pair's owner there),
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
        advance(w) { /* walk adj[w.lo..w.hi] — a slice walk, forward */ }
        iteration_order: forward_insertion
        phase: append_only_build, requires_compaction_at_boundary,
               freeze_scope: per_store, read_precondition: post_compaction
        // `on store build`/`on store frozen` key on the closed label set
        // {build, frozen} (Constraints) — the phase: annotations describe
        // properties, not labels; two csr bindings on one store compact in
        // binding order at the single freeze()
        requires: direct_records      // buckets/offsets index by dense A ids;
                                      // a path-id self is rejected here, not
                                      // as a codegen type error
        validate { /* offsets len == arena_len(A) + 1, starts 0, ends
                      adj.len(), monotonic — assuming compaction has run */ }
    }
}
```

**The clause grammar, pinned (round 4).** The five declarations imply a
grammar the plan never stated — a parser author had to invent the
clause inventory, the mandatory/optional split, and the selection
rules. The inventory, per cardinality (`req` mandatory, `opt` optional,
`—` illegal):

| clause | `one` | `many` | `key(K)` | when absent |
|---|---|---|---|---|
| `on A { .. }` / `on B { .. }` | opt | opt | opt | no contribution on that record (arena has none) |
| `on store [build\|frozen] { .. }` | opt | opt | opt | no store-level contribution; labels are legal only on `store`, from the closed set `{build, frozen}` (Constraints); unlabeled `on store` exists in all phases — csr spells the labeled forms, hash_index never spells the slot (its `on A` folds to it) |
| `ctor_arg` | opt | — | — | the field is hook-maintained or default-initialized; requires an `add_<A>`-orchestrated self (below) |
| `read(a)` / `lookup(a, k)` | req | — | req | — (the access protocol). The keyword is backing-conventional — the trait method's name comes from the root's registration metadata (M1), never from the clause keyword |
| `iter Walk { .. }` | — | req | — | — (keyed traits carry no GAT — the hash-index probe's pinned shape) |
| `iterate(a)` | — | req | — | — |
| `advance(w)` | — | req | — | — (the body is the whole `next()` body, returning `Option<`target handle`>`) |
| `on_insert(a[, k], b)` | — | opt | opt | no insert-hook maintenance; `one`'s write path is `ctor_arg` (M4's `set_` grows the re-pointing) |
| `on_add(a)` | opt | opt | opt | no per-add growth of store-level per-self state; requires an `add_<A>`-orchestrated self |
| `compact { .. }` | — | opt | — | legal only on a phased backing; phased backings are `many` in M3 (csr is the only one) |
| `validate` | opt | opt | opt | no validate body summoned for this backing (arena writes none) |
| `validate by` / `validate primitive` | — | opt | — | selected per binding — below |
| `membership: existence` | — | opt | — | membership kind comes from the binding site (primitive or `by`-derived, M1) |
| `requires: ..` | opt | opt | opt | no applicability predicates |
| `contract: ..` | opt | opt | opt | no contract annotation |
| `iteration_order: ..` | — | opt | — | no order contract |
| `phase: ..` | — | opt | — | the backing is unphased |

Rules the table rides on:

- **Selection.** `validate by` is selected when the bound arrow is
  `by`-derived and `validate primitive` when its membership is
  primitive — the arrow's instantiation facts, read at `implement!`
  time where the binding is known (the declaration authors both; the
  binding picks). A declaration carrying only one variant binds only
  that membership kind; the other kind at the binding site is an
  `implement!`-time rejection quoting the arrow and the missing body.
  Carrying neither is arena's shape.
- **Cardinality is two checks.** Clause legality (this table) and the
  root's declared cardinality (Constraints) must both hold; either
  rejects independently.
- **Order and repetition.** Clauses appear in any order. A clause
  repeats only as its selector distinguishes it — `on` once per
  (target, label) pair, the validate family once per variant. Any
  other repeat, and any clause not in the table, is grammar-rejected.
- **Annotation families.** `requires:` (closed predicate vocabulary,
  Constraints), `contract:` (contract annotations; the closed set is
  `exclusive_membership` today), `iteration_order:` (closed values
  `reverse_insertion`, `forward_insertion`), `phase:` (a comma list of
  properties, each a bare ident or `ident: value` — spike 6's
  vocabulary distributed across three syntactic homes, each home's set
  closed), and `membership:` (`existence`). Unknown items in any
  family are grammar-rejected with the closed set quoted.

What each clause is load-bearing for:

- **`ctor_arg` (field).** A required field must be in the record literal
  at creation; a post-push hook cannot express it. "One constructor arg
  per total `one` arrow" (M1) becomes "one arg per *total* `one` arrow
  whose backing contributes `ctor_arg`" — `ctor_arg` requires totality
  (an optional `one` arrow's field is default-initialized and updated by
  M4's `set_`). Field is the only such binding in M3, so the M1/M2
  constructor surfaces are unchanged. Applicability: `ctor_arg` requires
  an `add_<A>`-orchestrated self (the rule the csr bullet below states
  for `on_add`); at an unfused root the root owns no constructor
  (`Store::new()` takes no arguments, M1's pinned API), so the binding —
  reachable, since `field` is the *default* binding for `one` arrows and
  M1 allows arrows sourced at the root — is an `implement!`-time
  rejection (round 4). Under composition the merged
  constructor takes the union of every member's `ctor_arg` arguments;
  colliding argument names are *shared* when the two members' resolved
  target classes agree (one argument feeds both fields — the `<->`
  partner map usually guarantees agreement) and *instance-prefixed*
  when they differ (`outer_source`, per M2's collision-first naming;
  probe-verified by the ctor-merge probe — and, since review 3, by
  /tmp/m3-probe-r3-ctor-share, 4/4, which emits both halves in one
  crate: the sharing half routes one argument into both instance-prefixed
  fields by explicit init, and the member *fields* stay instance-prefixed
  even when the *argument* is shared) — spike 1's "silently
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
  validate. Applicability follows, and generalizes (round 4) to every
  constructor-orchestrated clause — `on_add` here, `ctor_arg` on field:
  such a clause requires an `add_<A>`-orchestrated self — plain or
  fused root-marked. At an unfused root there is no `add_<A>` (the root
  owns no constructor) and no `a` for `store.buckets[idx(a)]`;
  `csr_adjacency` bound to an entity arrow or an unfused-root self is an
  `implement!`-time rejection. The fused root-marked form is admitted
  but unemitted: spike 6 built flat `Digraph` only, and the inference —
  `a` is the host id, `on_add` fires from the host class's constructor,
  buckets stay store-level — while coherent is carried as frontier
  until emission (Frontier, below).
- **Phase attribution on contributions (`on store build` / `on store
  frozen`).** The type-state store needs to know which fields exist in
  which phase; the spike split them by comment, which two legal
  emissions could disagree on. The attribution is the partition, and it
  is what makes the compile-fail guarantees (no insert hooks, no
  accumulators on the frozen type) derivable rather than asserted. The
  labels key on the closed set `{build, frozen}` — declared vocabulary,
  not derived from the `phase:` annotations (which describe properties);
  other labels are grammar-rejected. Two phased backings on one store
  compact in binding order at the single `freeze()`; unlabeled
  store-level contributions (hash_index at an unfused root) exist in
  all phases.
- **Hooks are id-space methods on the store** — doc-hidden
  `fn __insert_<arrow>(&mut self, ..)` per backing, the landed M1 shape
  (`macros/src/codegen.rs` emits methods and invokes them
  `self.#hook(..)`); spike 11's free-function spelling
  (`insert_subnode_hook(&mut Store, host, main, sub)`) was hand-emission
  convenience, and the round-3 probes reverted to methods. Methods for
  the same reason validate is a method (round 4, aligning text with
  code): two `implement!` sites in one module cannot collide on a
  method of distinct store types, where free functions would — the
  hash-index probe recorded exactly that collision and suffixed its
  hand-emitted names around it. The M1/M2 commitment that M4's flush
  replay builds on stands; the read side is
  id-space too (`read`, `lookup`); codegen wraps ids into handles for
  the trait impls.
- **`iter Walk` names the iterator** feeding the trait's GAT — a `many`
  backing must declare its iterator as a named struct with concrete
  fields; a bare `iterate(a) { … }` body is insufficient on its own
  (spike-verified: the named clause generates the renamed iterator
  struct plus its `Iterator` impl). Two clauses author it together:
  `iterate(a)` constructs the Walk; **`advance(w)` authors the
  `next()` body** over the same roles — without it the advance recipe
  is hidden per-backing codegen knowledge (review 3: the stdlib's three
  recipes — index mint, link walk, slice walk — are three distinct
  bodies with no DSL home, and a user Walk is unemittable). Reserved
  iterator fields: codegen adds the store ref to every Walk, and at
  embedded placements the host path beside it — `iterate(a)`'s `a` is
  the host id there (M2: embedded iteration "gains a host parameter").
  Which recipes need the host is probe-graded, not blanket-claimed
  (/tmp/m3-probe-r4-advance, 5/5 — the clause's own mechanism, body
  splicing and the `w` binding and `handle(x)` availability, authored
  and emitted at the embedded placement through both stdlib recipes):
  the index mint takes the host from the Walk's reserved field, while
  the intrusive link walk mints from the path id itself and leaves the
  reserved host field unused — the field is reserved for the recipes
  that need it. `handle(x)` stays in the reserved placeholders (role
  table) so the body's vocabulary stays closed.

Two mechanical obligations come with user-writable backings:

- **Token routing.** A backing declaration has the same cross-item
  problem a pattern has: `implement!` must know a backing's field
  contributions and hooks at expansion time, and proc macros cannot read
  other items' tokens. `backing!` therefore emits a *backing courier* —
  the same target-parameterized single-rule courier as patterns and
  roots — embedding its field declarations, hook bodies, and constraint
  annotations; `implement!` resolves a backing binding by invoking its
  courier. No fixed table of backings, same as no fixed table of roots.
  Resolution is the **declaring-crate resolver** M2 pins for roots, not
  a new mechanism: the backing courier bakes its declaring crate's
  `CARGO_PKG_NAME` at declaration, the implement hop maps it through
  the `proc-macro-crate` scan, and the implementing crate must depend
  directly on every backing's declaring crate (renames fine; re-export
  routing rejected — the resolver's one constraint, inherited by
  reference). Backing bindings therefore carry a **path**
  (`out_edges: stdlib::intrusive_list`); the bare-ident form stays
  legal and resolves against the runtime crate (where the stdlib's
  backings live) — the binding grammar grows the path form the way M2
  step 1 grows the qualified `instance.arrow` form. Cross-crate is the
  default path, not an exotic one: the stdlib's couriers live in the
  runtime crate and are resolved from every user crate, so task 1's
  bounce gate includes cross-crate arms. The courier inherits M1's
  landed constraints: `#[macro_export]`
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
   route them through explicit bindings. Known container paths are
   absolutized at emission (`HashMap` →
   `::std::collections::HashMap`) — a user crate without the import
   would otherwise see E0412 inside generated code (review 3).
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
method per bound backing, summoned from the store's `validate()` in
the `implement!` block's binding order. M2's canonical inventory remains
the *contract* each backing must satisfy — M4's flush runs exactly these —
and the stdlib re-implementation is graded against it: each built-in's
clause must reproduce the checks the fixed-table era hard-coded.

- field: endpoint-in-range over its contributed field.
- arena: nothing — existence cannot dangle.
- hash_index: key existence, value-in-arena, and value-host agreement
  (an embedded-target map holds host-anchored path ids; a value whose
  host component names a *different* host record is in-bounds in its
  own host's arena, so value-in-arena alone passes it — spike 11's
  `validate` implements all three, host agreement as `target.0 != host`
  naming "a different host's arena"; and M4's flush runs exactly this
  inventory). Expressibility is settled by the role table as of review
  3: owner-yielding `maps(A)` plus `host_of` writes the third check as
  one placement-uniform comparison (probe-verified,
  /tmp/m3-probe-r3-hostagree, 5/5 — the round-3 hash_index probe first
  demonstrated the gap live under the bare-map reading,
  /tmp/m3-probe-r3-hashindex, 11/11), naming offending ids in
  the message (the edge-induced probe's style). Round 4 extends the
  same settlement to the first two checks: with the vacuous-host
  `b_arena_of`/`k_arena_of` definitions (role table), key existence and
  value-in-arena are the same one-body shape — the sketches above now
  author all three checks outright, where round 3 left "placement-
  correct containment" as a comment. Key uniqueness is not an
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

- cardinality: `one` / `many` / `key(K)` — must match the root's.
  (Optional-vs-total is a separate `one`-arrow axis, not a fourth
  cardinality — see `ctor_arg`, which requires totality.)
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
- `exclusive_membership` (intrusive_list): a **declared contract
  annotation, never a bindings-level predicate** — two intrusive
  bindings sharing a target entity are legal (Digraph's `out_edges` +
  `in_edges`, M1's flagship binding; per-arrow stem-namespacing keeps
  the link fields distinct, so the lists cannot interfere). Review 3
  found the earlier bindings-level reading self-condemning: it rejected
  exactly that binding. What exclusivity actually means — each object
  belongs to at most one relation instance *per arrow* — is enforced at
  runtime by the two validate bodies (membership consistency against the
  determining field; the partition walk), per M2's inventory: "an
  exclusive-membership contract — built in while the backings are a
  fixed table (M1–2), surfaced as a constraint annotation when M3's
  `backing!` DSL lands". Shared membership across arrows is the
  link-record design with its own arena (M2's refinement discussion).
- ids are `Copy + Eq + Hash` universally — documented, not checked.

Contract and phase annotations ride alongside, from spike 6's
vocabulary: `iteration_order` (a per-backing contract — a closed value
set like the predicates: `reverse_insertion`, `forward_insertion`;
intrusive_list the former, CSR the latter; M4's flush
order contracts are stated against it), `append_only_build` (spike 6's
`append_only`, renamed in the declaration), `requires_compaction_at_boundary`,
`freeze_scope: per_store`, `read_precondition: post_compaction`. Spike
6's `field_contributions: store_level_only` is absorbed by the
phase-attributed `on store build/frozen` clauses rather than carried as
a separate annotation. `exclusive_membership` is one of these — a
contract annotation, not a predicate (above).

Each predicate's evaluation is pinned to an error template naming the
computed placement facts — the epic's cross-cutting mandate, applied to
the new vocabulary:

| predicate | inputs | rejects when | error names |
|---|---|---|---|
| cardinality | the root's declared cardinality, the backing's | mismatch | the arrow, both cardinalities |
| `existence` | bindings, computed placement | not the unique existence arrow of B (see above) | both arrows on a duplicate; the arrow and its membership kind otherwise |
| `hosts_arena(A, B)` | computed placement of B's arena, A's self case | B's arena is not hosted by A's record per the taxonomy | the arrow, B's computed arena location, A's self case |
| `direct_records` | placements of both endpoint classes | either endpoint needs path indirection | the arrow, both endpoints' placements |

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
ordering. One gap flagged for M4, not adjudicated here: its step 6 does
not state the compaction order when several phased backings share a
store — this plan pins binding order at the single `freeze()` (the
phase-attribution bullet), and M4 inherits or revisits that pin.

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
compose time; M2's docking check carries the same-instance
element-element and self-hosting arms as of the round-4 merge, with
their trybuild fixtures owed as a code follow-up in M2's
negative-tests list). Composition nests whole patterns
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
(spike 1's further-composition limit — unnumbered in its report; F8 is
the courier-payload finding — now grown by round 4's per-member
`("pkg", {schema})` carriage, M2's Mechanism section: nested composed
members must carry their declaring packages through the hops, and
composed library patterns inherit the resolver's direct-dependency
rule — M2's own "highest-risk unverified codegen" flag applies to the
nested case a fortiori). Shipping library patterns as composable makes that
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
  rename predates M1's implementation landing — the landed code spelled
  the derivation `union` until the mechanical code rename rode with
  M2's step-1 grammar pass (`macros/src/grammar.rs` and `src/stdlib.rs`
  now parse only `sum`). The semantics
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
above); `<=>` transitivity — vacuous beyond two members *by design*
once M2's docking consequence (every legal class is one element member
plus at most one root) is pinned by the extended docking check (the
arms are in M2 as of the round-4 merge); and further
composition of a composed pattern (the grammar note above). Retired in
this fold: intrusive lists sourced at unfused roots — DSL-legal
(store-level head, self-less store-level op, root-handle read) and
probe-verified end to end (/tmp/m3-probe-r3-root-intrusive, 5/5); spike 1's "constructor argument name collisions silently
deduped" (unemittable for required fields — E0063; colliding `ctor_arg`
contributions are shared when target classes agree and instance-prefixed
when they differ, per the grammar section — both halves now
probe-covered: prefixing /tmp/m3-probe-ctor-merge, sharing
/tmp/m3-probe-r3-ctor-share)
and "field backing into an embedded class rejected" as a blanket bar
(the ctor_arg-side rejection stands as M2's addressability rule; what is
retired is its extension to hook-maintained path-id values — see the
placement rules).

Standing limits of the round-3 probes, carried as frontier (round 4):
`maps(A)` must unify one owner type across a store's map placements —
mixed host-id shapes (two embedding levels, or path-id and flat-id maps
on one store) are untested (the host-agreement probe's limits); no
canonical element-handle accessor is pinned — the root-intrusive
probe's tests obtained element handles through the members read, and
codegen owes the accessor its own decision; the ctor-share probe
asserted `<->` resolution by emitted type only (no partner-map
machinery executed). One admission ahead of emission, alongside the
on_add applicability note: csr at a fused root-marked self is admitted
but unemitted — spike 6 built flat `Digraph` only, and the inference
(`a` is the host id, `on_add` fires from the host class's constructor,
buckets stay store-level) holds on paper until something emits it.

## Depends on

M2 complete — especially a hook interface that survived keyed indexes.
M2's review outcomes for the promotion names are in (see Stdlib
promotion). Do not start the
`backing!` grammar before the four built-ins exist; that ordering is the
whole point of the milestone.

**Merge chore against main's M2 — discharged at the round-4 merge**
(the rebase took main's M2 wholesale and re-grafted the
reconciliations as plan text; the chore is kept as the record of what
was grafted): intrusive arrows sourced at unfused roots reclassified
DSL-legal, probe-verified (/tmp/m3-probe-r3-root-intrusive, 5/5) —
the landed check still rejects the arm until M3's intrusive step
relaxes it, a named code follow-up; the field-into-embedded scope
(addressability stands, hook-maintained exempt) was already main's own
D2(a) fold; the docking arms (same-instance element-element,
self-hosting) are plan-pinned in check 5, with their code and trybuild
fixtures a named follow-up in M2's negative-tests list. **epic.md** —
this branch's epic taken at merge (hostings paragraph, spike-12
amendments, status markers). **spikes.md** — rows unioned, keeping the
branch's superset; main's queued combined-rule row retired by
/tmp/m3-probe-r3-ctor-share.

## Implementation plan

Each step is a handoff unit: an agent (or a pair) can take one step
with this plan as the whole contract — the cited sections are the spec,
the work checkboxes are the scope, and the acceptance checkboxes are
hard gates. Tick boxes in this file as work lands; a step is done when
its acceptance list is fully ticked and the suites named there are
green. Steps run in order; `Depends` is binding. The milestone's
overall gate, stated once: the four built-ins re-implemented as
`backing!` declarations, the fixed table deleted, and the M1/M2 suites
green unchanged.

**Step 0 — Preconditions (a gate, not work).**
- [x] M2 complete on main and the merge chore discharged at the
      round-4 merge (the three M2 reconciliations, the epic.md fold,
      and the spikes.md row union — Depends on, above; the docking
      arms' code + fixtures are the one named code follow-up)
- [x] the `union` → `sum` code rename landed (it rode with M2's
      step-1 grammar pass — verified in `macros/src/grammar.rs`)
- [x] /tmp/m3-probe-r4-advance read, its limits carried (the
      reserved-host-field refinement, grammar section); the planned
      /tmp/m3-probe-r4-bounce superseded at the round-4 merge — the
      bounce evidence is step 2's permanent in-repo gate, which adds
      the use-site helper-resolution arm (header)

**Step 1 — `backing!` parser + courier emission.** Contract: the
clause grammar (the pinned table and its rules) and the token-routing
paragraph.
- [x] parse the header (`name: A -> B, cardinality`) and the clause
      inventory per the table; grammar-reject unknown clauses, store
      labels, and annotation values, quoting the closed set in the
      message
- [x] enforce the repetition rule: `on` once per (target, label) pair,
      the validate family once per variant, everything else once
- [x] emit the backing courier: `#[macro_export]`, the M1
      target-parameterized single-rule shape, payload = the
      declaration's tokens plus the declaring crate's baked
      `CARGO_PKG_NAME`
- [x] document the E0255 constraint (a crate-root declaration collides
      with its own courier — declare backings in a named module)
- Acceptance:
- [x] round-trip tests: each stdlib declaration parses and re-emits;
      one rejection fixture per closed set (unknown clause, bad store
      label, unknown contract item, unknown phase property, bad
      iteration_order value, repeated clause)
- Depends: 0

*(Implementation note, step 1: one deliberate extension, flagged for
the record. The plan pins a per-site naming formula per contributed
field and states there is no single mechanical rule, but shows no
syntax for stating it. The parser therefore accepts an optional
per-field naming formula — `on A { value: BId = arrow }`, closed values
`arrow` / `stem` / `suffix`, default bare — and authored bodies
reference the logical field name while codegen renames to the emitted
one per binding. The five stdlib declarations carry exactly the
formulas the plan's table pins: field `= arrow`, intrusive `= stem`,
hash_index `= suffix`, csr bare.)*

**Step 2 — Binding resolution in `implement!`.** Contract: the
token-routing paragraph (declaring-crate resolver).
- [ ] grow the binding grammar with the path form; the bare-ident form
      stays legal and resolves against the runtime crate
- [ ] the resolution hop via `proc-macro-crate`: renames handled,
      re-export routing rejected, the direct-dependency rule enforced
      with a clean `compile_error!`
- [ ] the bounce gate, in-repo and permanent: two distinct backing
      couriers resolved per one `implement!`-shaped invocation, bodies
      spliced hygienically, with the cross-crate arms — a stdlib
      backing resolved from a user crate, a user backing declared in a
      second crate, a renamed dependency — and the use-site
      helper-resolution arm: a helper named in a hook body must resolve
      at the backing's use site (spike 5's caveat 2), so the gate
      bounces the whole conjunction, resolution and hygiene together
      (round 4 superseded the planned throwaway probe once M2 merged —
      header; this gate is the evidence)
- Depends: 1

**Step 3 — `field` + `arena` in the DSL; the fixed table deleted.**
Contract: the declarations, the placement rules, the premise table.
- [ ] `field`: `on`/`ctor_arg`/`read`/`validate` wired to the existing
      codegen shapes; the `ctor_arg` unfused-root rejection emitted
- [ ] `arena`: `membership: existence`, `iter`/`iterate`/`advance`
      with the reserved iterator fields, the `hosts_arena` predicate
      wired
- [ ] delete the fixed backing table from the macros crate
- Acceptance:
- [ ] M1 suite green, koenigsberg assertions byte-identical
- [ ] expanded code shows couriers resolved and hook *methods* emitted
- Depends: 1, 2

**Step 4 — `intrusive_list` in the DSL.** Contract: the declaration,
the hygiene caveats.
- [ ] `on A` + `on B` contributions, the stem formula, the push-front
      hook as a store method
- [ ] the hygiene caveats as grammar rules: `Span::mixed_site()`
      scaffold idents, absolutized container paths, reserved-set
      shadowing rejected
- [ ] `validate by` + `validate primitive` authored and selected per
      binding at `implement!` time
- Acceptance:
- [ ] M1 suite green including traversal orders (reverse insertion)
- Depends: 3

**Step 5 — `hash_index` in the DSL.** Gated on M2's keyed grammar
landing; the names are settled (Stdlib promotion).
- [ ] `key(K)` clauses: check-then-write `on_insert`, `lookup`, and
      the three-check validate over `maps(A)`/`host_of`/
      `k_arena_of`/`b_arena_of`
- [ ] all three self placements emitted: plain, host-embedded,
      store-level
- Acceptance:
- [ ] M2 keyed tests green in both M2 placements (host-embedded,
      store-level) plus the plain-self placement
- [ ] the `#[should_panic]` duplicate-key test green
- Depends: 3, M2's keyed step

**Step 6 — Validate machinery + corruption fixtures.** Contract: the
validate contribution.
- [ ] doc-hidden `__validate_<arrow>` store methods, summoned from
      `validate()` in binding order
- [ ] corruption fixtures per spike 4 for every canonical-inventory
      check: ghost keys, out-of-range and cross-host values, chain
      corruption in both partition shapes (shared membership vs cycle,
      distinguished in the report)
- [ ] the canonical inventory written out as the documented contract
      M4's flush runs
- Depends: 3–5

**Step 7 — Constraint predicates + trybuild error tests.** Contract:
Constraints and applicability, the evaluation table.
- [ ] predicate evaluation with the pinned error templates:
      cardinality, `existence`, `hosts_arena`, `direct_records`
- [ ] the `ctor_arg`/`on_add` orchestrator-rule rejection
- [ ] trybuild fixtures for each predicate and for the grammar
      rejections from step 1; messages quote tokens and state
      placement facts
- Depends: 3–5

**Step 8 — `csr_adjacency` + type-state phases.** Contract: CSR and
phase representation.
- [ ] `phase:` parsing; the `on store build`/`on store frozen`
      partition; the type-state store parameter emitted only when a
      phased backing is bound
- [ ] `compact` at the single `freeze()`, binding order for multiple
      phased backings
- [ ] compile-fail proofs: insert-after-freeze, frozen-as-builder, no
      accumulators on the frozen type
- [ ] applicability rejections (entity arrow, unfused-root self); the
      fused-root emission attempted against the frontier note — emit
      it, or record exactly what breaks
- Acceptance:
- [ ] spike-6's CSR tests and the mixed-phase probe's shape green
- Depends: 4, 6

**Step 9 — Stdlib promotion + doc pass.** Contract: Stdlib promotion.
- [ ] M2's discovered roots move into the runtime stdlib;
      pattern-local declarations re-point by path; names as M2
      resolved them
- [ ] doc pass to M1 step-11's standard; generated-code doc comments
      carry the arrow that produced each item
- Acceptance:
- [ ] M2 suites unchanged modulo import paths
- Depends: 1–8

**Overall gate (the milestone's success criterion, restated):**
- [ ] `field`, `arena`, `intrusive_list`, `hash_index`, and
      `csr_adjacency` are `backing!` declarations shipped in the
      runtime crate
- [ ] the fixed table of backings is gone from the macros crate, and
      no hidden fixed table survives (every behavior filed under (a)
      or (b) in the premise table)
- [ ] M1 and M2 suites green, unchanged
