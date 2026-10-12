# Milestone 5: implement-time data — user payloads on records

**Goal**: user data types bind at `implement!`. Records carry them; they
ride the ordinary write-path ladder — constructor argument, builder
write, staged set through M4's flush. The same abstract pattern
implements with data, without data, or with different data: payloads are
an implementation concern, not schema vocabulary. This is what makes
stores usable for anything beyond structure — layout locations, labels,
cached computations, domain values.

Specification level: preliminary. Designed in the M5 review conversation
(2026-10-12) against M1–M4's landed and pinned machinery; no probe or
spike covers it yet, so every load-bearing unverified claim is named
inline and lands as a test or trybuild fixture in the checklist. The
mechanism it rides is grounded: M1–M2's record/constructor/read-view
machinery (landed), M3's declaration-driven codegen (steps 2–5 complete
on `worktree-m3-implement`, steps 6–9 in flight), M4's staging and flush
pipeline (reviewed on `worktree-m4-review`, unexecuted).

## Motivation

The driving scenario is ports and layout (the edge-crossing-levels
discussion this plan grew out of): a `Port` record exists *before*
layout computes its location; the location arrives later, from outside,
possibly after the store is frozen and being read; re-layout replaces
it. The location is not structure — nothing maintains it, nothing
validates it, no generic algorithm should be forced to see it.

The pre-existing answer in the epic ("node/edge weights as `one` arrows
into value arenas") models the value as an *entity* and the attachment
as an *arrow*. That spelling works mechanically but puts a data concern
into the abstract schema: every implementation and every consumer of the
pattern carries a `Point` sort, and the pattern cannot be implemented
without the payload. This plan lands the other fork, decided in review:

> **Data is data, not entities.** Patterns stay type-abstract
> (invariant 2); the data type enters the system at `implement!` and
> nowhere else.

The arrow-shaped half (trait-visible weights) remains a future thread —
see Interactions.

## Premises (what M1–M4 established that this rides on)

- Patterns and compositions name sorts and arrows only; no Rust type
  enters `pattern!` or `compose_pattern!`. Records are generated per
  entity, their fields contributed entirely by backing declarations.
- M3 made both codegens declaration-driven: field contributions with
  naming formulas and collision prefixing (arrow-stem within one
  instance, instance prefix across composition), the combined
  constructor rule (share-or-prefix), and the uniform field-init rule —
  `ctor_arg`-fed ← the constructor argument; `Option<_>` → `None`;
  `Vec<_>`/`HashMap<_, _>` → `new()`.
- M4 pins the transaction: a change set is a plain struct, one op queue
  whose indices count every op; staging ops panic on programming errors
  (foreign/stale ref stamp, `set_`/remove of a `Pending` ref) and flush
  reports individually-legal-but-incoherent change sets as
  `FlushError`; flush is the seven-step pipeline whose step 4 copies the
  kept skeleton structurally (a record before anything that references
  it) and then replays staged constructive ops in submission order,
  root-level before nested. The builder carries an id-based read view
  (ids and copies, never handles).
- The `implement!` input round-trips the config hops as token groups —
  never re-tokenized in flight (M2's rule). Binding-grammar errors keep
  real spans because they are checked in stage 1, at the call site.

## The premise table

M3's discipline: every behavior is either **(a)** something the
declaration states, or **(b)** a uniform rule no declaration may vary.
Data follows it.

| behavior | class | grounding |
|---|---|---|
| which entities carry which fields at which types | (a) — the `data` clause | this plan |
| where data lives: on the declaring entity's record, wherever that record is placed | (b) — the record placement taxonomy (below) | M2's placement rule applied to records |
| field initialization: constructor argument or the closed default set | (a)+(b) — the clause supplies the type; supply mode follows M3's uniform init rule verbatim | M3 grammar section |
| access: `data()` on handles; `data()`/`data_mut()` on the builder | (b) — uniform emission conventions | this plan, access protocol |
| writes: builder assignment, staged per-field set post-freeze; never a mutator on a handle | (b) | invariant 5 |
| flush: data rides the kept-record copy verbatim; staged sets replay in submission order | (b) | M4 pipeline step 4 |
| referential integrity / validation: none — data is RI-opaque | design boundary | this plan, maintenance boundary |
| trait surface: none — data ops are inherent, store-specific | design boundary | invariant 7's converse, deliberately |

If a data behavior cannot be filed under (a) or (b), or is found
leaking into traits or generic visibility, the milestone's premise has
failed.

## The `data` clause

```rust
implement! {
    comp::Crossing as CrossingStore {
        g1.nodes:     arena,
        g1.out_edges: intrusive_list,
        // ... bindings unchanged ...
        data {
            g1.Node { loc: Option<Point> }
            g1.Port { loc: Option<Point>, name: String }
            g2.Node { loc: Option<Point> }   // inner frame, embedded record
        }
    }
}
```

- **One optional `data` block**, position free among the bindings,
  repetition rejected. The keyword is distinguishable from a binding
  (bindings begin `ident`/`instance.ident` and never open a brace
  group there).
- **Entity references** use the same grammar and resolution as
  bindings: bare entity in a flat pattern, `instance.Entity` under
  composition, qualified whenever ambiguous — the identical rule.
  Unknown entity → stage-1 rejection quoting the pattern's entity
  list. An entity may appear in the block once; twice → rejection
  quoting both occurrences.
- **Fields** are `ident : Type`, `Type` parsed as an arbitrary
  `syn::Type` at the call site. Types are opaque tokens to every
  mechanism: they never enter the role engine, never ride a backing
  courier, place no applicability constraints, and round-trip the
  config hops as token groups exactly as the schema does. No bounds are
  required of them — not `Copy`, not `Debug`, not `Default`. (Reads
  through the shared gateway are by reference, so `Copy` is an
  ergonomic convenience, never a requirement.)
- **Spans.** The clause is user tokens at the implement site; stage-1
  grammar errors keep real spans, exactly as binding errors do
  (M2's discipline). Codegen-time checks below quote tokens in the
  message text per the epic's mandate.

### Field supply: the init rule, inherited verbatim

A data field is initialized by exactly one of two modes, chosen by its
type's shape under M3's uniform init rule:

- `Option<_>` → `None`; `Vec<_>`/`HashMap<_, _>` → empty — the field is
  *default-initialized*, populated later by write.
- any other type → **constructor-supplied**: the field becomes an
  argument of `add_<entity>` (so `name: String` gives
  `add_port(owner, name)`).

No annotation syntax in v1; there is nothing to annotate because the
rule is total. `Default`-based init beyond the closed set is frontier.
Under composition, constructor-supplied data fields join the entity's
constructor signature through the existing merge machinery; the
combined collision rule's *share* arm has no meaning for data (there is
no target class to agree), so same-named ctor data fields from two
members always take the instance-prefix arm.

### Naming and collisions

- The generated struct is `<Class>UserData` — named after the class
  whose record it embeds in (`PortUserData`, `OuterNodeUserData`) —
  with **pub fields**, embedded in the record as one private field
  named `data`. The gateway (below) hands out `&<Class>UserData`.
- **`data` is a reserved arrow local name**, blanket, both codegens.
  The reserve covers two collision sites at once: the gateway method
  `data()` on handles, and the record's own `data` field (a
  field-backed arrow named `data` would contribute a record field
  `data` by the stem formula). Rejection quotes the arrow and the
  reserved word.
- **Store-wide staging-name rule.** Staged sets are per-field methods
  on the transaction (`tx.set_loc(r, pt)`), so data field names share
  one namespace: the store's arrow local names and every data field on
  the store. When a data field's name is unique across that namespace
  its staging method stays bare; when it collides — with another data
  field (`Node.loc` and `Port.loc`) or with an arrow local name (an
  arrow `loc`) — *the data fields* take entity-qualified spellings
  (`tx.set_node_loc`, `tx.set_port_loc`); arrow spellings never move.
  Reads never collide: field access is namespaced by `.data()`, and
  each class's gateway returns its own struct.
- **Derives.** Records carry `#[derive(Debug)]` today. A data-carrying
  record drops every derive that would impose a bound on user types
  (today: `Debug`); `<Class>UserData` itself carries no derives. The
  user-data struct is module-private API surface only through the
  gateway, and records are module-private, so nothing public breaks —
  step 2 audits internal tests.

## Placement

Data follows records, under the same three-case record taxonomy M3 pins
for backing self records:

- **Plain entity** — its own class record; the common case.
- **Embedded element entity** — the embedded record, per host (a
  `data { sub.SubNode { … } }` field lives in every host's embedded
  SubNode record, path-addressed). **Data is placement-uniform,
  including embedded records** — the one deliberate divergence from
  M3's restriction that contributed fields locate only on root-level
  classes or the store. The restriction exists because backing
  contributions are *structural*: hooks write them, validate walks
  them, iterators mint through them. Data has none of that — no hook,
  no validate, no iteration, no RI — so nothing structural can misread
  an embedded data field, and the placement machinery already carries
  it: builder writes reach it by path id (the builder is `&mut` over
  the whole tree), staged writes are path-addressed ops like any
  nested op (M4), handles mint into embedded arenas already, and flush
  copies embedded records wholesale.
- **Fused root-marked entity** — the host class record. Data declared
  on the guest merges onto the host record; the host's single gateway
  returns one merged `<Class>UserData`. Same-named fields from host
  and guest entities instance-prefix (the prefix names the declaring
  instance), exactly as colliding constructor arguments do.
- **Open entity** — owns nothing, resolves to its partner;
  implement!-time rejection naming the entity and its openness.
- **Unfused root-marked entity** — owns no record; implement!-time
  rejection naming the placement facts (same family as
  `impl_unfused_root_field`). Store-level singleton data is frontier.

## Access protocol

| surface | spelling | notes |
|---|---|---|
| handle, frozen store | `h.data() -> &<Class>UserData` | shared borrow for the handle's lifetime; emitted once per data-carrying class; **no mutator on handles, ever** (invariant 5 — a handle holds `&Store`) |
| builder, shared | `s.data(id) -> &<Class>UserData` | joins the id-based read view (the view's contract widens from "ids and copies" to "ids, copies, and data references" — handles remain impossible against the `&mut` builder) |
| builder, exclusive | `s.data_mut(id) -> &mut <Class>UserData` | direct assignment: `s.data_mut(p).loc = Some(pt)` — one gateway replaces per-field setters pre-freeze |
| transaction | `tx.set_<field>(ref, value)` (entity-qualified on collision) | a *field-set op*: a `Set` with no referential integrity, no induction, no hooks |

Why a gateway and not the two obvious alternatives — recorded because
it was decided in review, not by default:

- *Per-field getters* (`h.loc()`) were rejected: they multiply the
  generated surface per field, and they put data field names into the
  handle's method namespace, forcing the store-wide collision rule
  onto reads as well as writes.
- *`Deref` to the user-data struct* (field syntax `h.loc`) was
  rejected: deref coercion makes handles silently acceptable wherever
  `&UserData` is expected and presents a `(store, index)` pair as a
  pointer — costs that outweight the two saved characters. The gateway
  is an ordinary getter; field syntax appears inside it, on a genuine
  pub-field struct, which is where idiomatic Rust puts field syntax.

## Write paths and flush

Three moments, one ladder:

1. **Construction.** Constructor-supplied fields ride `add_<entity>`
   arguments. Default-initialized fields start `None`/empty. There is
   no third mode.
2. **During build.** `data_mut` assignment, any order — plain field
   writes through the exclusive builder. This is where a layout pass
   runs against the still-building store: read structure through the
   id-based view, write locations, then freeze and read through
   handles. No M4 involvement.
3. **Post-freeze.** Staged field-sets against the transaction;
   re-layout is the canonical client (`store.edit()`, `set_loc` per
   port, `flush()`, render from the new store while old handles keep
   their old values).

Flush semantics, pinned against M4's pipeline:

- **Data never flows through hooks** — there are no data hooks. The
  kept-skeleton copy in pipeline step 4 is structural (field-list
  driven per generated store), so a kept record's data struct is
  carried field-for-field *by construction*. This plan's obligation is
  to pin that carry with a test (the wipe hazard: a flush that
  default-initialized kept data instead of carrying it would pass
  every structural check), not to change M4.
- Staged `set_<field>` ops replay in submission order within the level
  grouping, after the kept-record copy — last write wins, observable
  and deterministic.
- **`Pending` refs are legal targets for data sets** — deliberately
  unlike arrow `set_`, which panics on `Pending` (M4: "a staged
  record's determining fields are fixed by its own add op"). A
  default-init data field on a newly added record is exactly the
  create-then-populate pattern — `let r = tx.add_node();
  tx.set_loc(r, pt);` — submission order guarantees the add precedes
  (the ref does not exist before it), and nothing structural depends
  on the value. This asymmetry is a pinned decision, not an
  oversight; the staging docs state it beside the arrow rule.
- A data set staged against a removed element is a `FlushError` (the
  existing set-of-removed family); against a nested path whose host
  was removed, the existing removed-path family. Foreign/stale ref
  stamps panic at staging as everywhere.
- **Removal drops data with the record** — nothing to cascade, data
  being RI-opaque. Removal's id-remap report does not touch data.
- `into_edit` allocation reuse is unchanged: data copies are part of
  the record copy, inside the derive-before-write / forward-write
  discipline. Phased stores: data is record-level and persists across
  freeze untouched (the build/frozen partition governs *store-level*
  backing contributions, not record fields).

## The maintenance boundary

> **Arrows are maintained by the toolkit; data is maintained by you.**

Data fields are RI-opaque. A `Point` cannot dangle, so `validate()`
gains nothing to check; an `Option<NodeId>` stored *as data* is not
remapped by flush and not validated — if you want a link maintained,
it must be an arrow. This single sentence is the per-field decider
between this milestone and the arrow-shaped half, and it goes into the
crate docs verbatim. `validate()` is unchanged; no backing gains a
data clause; key uniqueness, endpoint ranges, host agreement — none of
the canonical inventory sees data.

## Interactions with other milestones

- **M3**: data rides the declaration-driven emission path as a new
  contribution kind beside the existing field contributions (records,
  constructors, collision prefixing); it never touches the role
  engine, and no backing declaration changes.
- **M4**: no redesign, and the forward-compat question resolves
  structurally — M4's kept-skeleton copy is field-driven, so M5's
  record growth rides it. M4's write-trait table is untouched: data
  ops are inherent methods on the transaction, never traits, because
  data is store-specific by design (nothing generic may bound on it —
  that is what the arrow-shaped half is for).
- **Optional-`one` seam, named and deliberately not absorbed**: M4's
  interface table carries optional-`one` rows
  (`e.payload() -> Option<_>`, `set_<arrow>`), but no landed
  checklist grows the *pattern grammar* spelling for an optional `one`
  arrow — `pattern!` today declares `one` (total), `many`, `key(K)`.
  Data `Option<Point>` absorbs the payload use case; *structural*
  optional links (a portal node's optional outer port) still need the
  arrow word. Scheduling that grammar is M4-adjacent work this plan
  flags, not performs — the crossing-levels scenario depends on it.
- **Vocabulary payloads (the reshaped epic thread)**: `one` arrows
  into value sorts, trait-visible so generic algorithms can read
  weights across implementations. Future; the decider per field is the
  maintenance boundary above.
- **Stdlib**: unchanged. No stdlib pattern grows data; the goal test's
  fixtures stay in the fixtures crate.

## Error templates

| check | fires | error names |
|---|---|---|
| unknown entity in the `data` clause | stage 1 | the entity ref, the pattern's entity list |
| entity repeated in the data block | stage 1 | both occurrences |
| duplicate field within one entity | stage 1 | both fields |
| data on an open entity | codegen | the entity, its openness, its `<->` partner |
| data on an unfused root-marked entity | codegen | the entity, its computed placement |
| arrow locally named `data` | codegen, blanket | the arrow, the reserved word, both collision sites (gateway method, record field) |
| data field = arrow local name, store-wide | codegen | both; the data field's entity-qualified spelling |
| data field names colliding across classes | codegen | every colliding field, their qualified spellings |
| data set on a removed element / removed path | flush | op index (existing `FlushError` families) |

Codegen-time messages quote tokens (spans have degraded by then);
stage-1 messages may use spans. Both follow the epic's cross-cutting
mandate.

## Motivating scenario — the goal test

Fixture roots, added to the fixtures crate in a new `ports` module
(names discharge the M2 naming gate: the root takes its arrow's name
camel-cased, `-Of` where the bare name is taken):

| root pattern | arrow | shape at the goal test |
|---|---|---|
| `Ports` | `ports: Node -> Port, many` | primitive, bound `intrusive_list` — exclusive membership is single-owner ports, and per-node iteration is the intrusive walk |
| `SrcPort` | `src_port: Edge -> Port, one` | field-backed (constructor arg of `add_edge`) |
| `DstPort` | `dst_port: Edge -> Port, one` | field-backed |
| `Owner` | `owner: Port -> Node, one` | field-backed |

`tests/port_layout.rs`, over a flat `PortedDigraph` (the stdlib six
plus these four, à la carte — Bipartite's construction):

1. **During-build layout** — build nodes/edges/ports, run layout
   against the id-based read view, `data_mut` the locations, freeze,
   then read the crossing's port the way client code will:
   `e1.dst_port().data().loc` — `Option<Point>`, one hop, no join.
2. **Post-freeze re-layout** — `store.edit()`, staged `set_loc` per
   port, `flush()`; old handles still readable with the *old*
   locations, the new store carries the new ones (the pass
   discipline).
3. **Constructor-supplied data** — `name: String` on Port rides
   `add_port(owner, name)`.
4. **Create-then-populate** — `set_loc` on a `Pending` ref inside one
   change set (the asymmetry pin).
5. **Removal drops data** — remove a port; counts and lookups confirm.
6. **Composed variant** — a Hierarchy-shaped store with
   `data { sub.SubNode { loc: Option<Point> } }`: path-id `data_mut`
   during build, fused-handle `data()` read after freeze, flush carry.
7. **Carry determinism** — a flush with no staged data sets produces a
   store whose every data field equals the old store's (the wipe
   hazard's permanent guard; the corruption-test style of
   `validate_failures.rs`, applied to flush).

## Explicit non-goals

- No data traits, markers, or generic visibility — the arrow-shaped
  half's territory.
- No store-level data; no `Default`-init beyond the closed set; no
  user `validate` hooks for data; no serialization or introspection of
  data (M6+ threads).
- No change to arrows, backings, or M4's write traits.
- No `Deref` anywhere in the access surface.

## Implementation checklist

Each step is a handoff unit; the cited sections are the contract, the
boxes are the scope, acceptance boxes are hard gates. Steps 1–3 need
M3's declaration-driven codegen (steps 2–5 of M3); steps 4–5 need M4
landed. Steps 1–3 may run parallel to M4's implementation.

**Step 1 — grammar + round-trip.** Contract: the `data` clause section.
- [ ] parse the block in `grammar.rs` beside `parse_bindings` (keyword
      `data`, one brace block, entity refs through the existing
      instance/entity split, `syn::Type` fields); round-trip through
      the config as token groups (group-preserving, never
      re-tokenized)
- [ ] stage-1 rejections: unknown entity, entity twice, duplicate
      field, missing/misplaced block; real spans
- [ ] the `data` reserved-name and store-wide staging-name rules
      evaluated where binding names are known (stage 1 if the arrow
      list is available there, else codegen — say which in the commit)
- Acceptance:
- [ ] round-trip unit tests mirroring `bindings_parse_and_roundtrip`;
      one rejection fixture per stage-1 check
- [ ] M1/M2 suites green (no data clauses anywhere — unchanged by
      construction)
- Depends: M3 steps 1–2

**Step 2 — flat codegen.** Contract: naming, placement (plain case),
access protocol, the init rule.
- [ ] `<Class>UserData` emission (pub fields, no derives), record
      merge as one `data` field, derive-drop rule for data-carrying
      records (audit internal uses of record `Debug`)
- [ ] ctor-supplied fields as `add_<entity>` args per the init rule;
      default-init per the closed set
- [ ] `data()` on handles, `data()`/`data_mut()` on the builder, the
      read-view widening
- Acceptance:
- [ ] new flat tests: ctor supply, default-init reads, `data_mut`
      round-trip, non-`Copy` type through the shared gateway
- [ ] M1 suite green, koenigsberg assertions byte-identical
- Depends: 1, M3 steps 2–5

**Step 3 — composed codegen.** Contract: placement (embedded, fused),
naming collisions under composition.
- [ ] embedded-record placement end to end (path-id writes, fused/
      nested handle reads); fused-class merge with instance-prefixed
      colliding fields and the single merged gateway struct
- [ ] merged constructor args through the existing machinery (prefix
      arm); entity-qualified staging spellings
- [ ] trybuild fixtures: open entity, unfused root, `data` arrow name,
      both collision cases
- Acceptance:
- [ ] composed data tests (the Hierarchy variant) green; M2 suites
      green
- Depends: 2

**Step 4 — write side.** Contract: write paths and flush.
- [ ] staged field-set ops (queue variants per data field,
      path-addressed under nesting), `Pending` legality, replay in
      submission order
- [ ] carry pin: the no-staged-sets determinism test (goal test 7) and
      a staged-set last-write-wins test
- [ ] `FlushError` arm (set-on-removed, removed-path) riding the
      existing families; staging panics unchanged
- [ ] `into_edit` reuse verified with data-carrying records
- Acceptance:
- [ ] goal tests 2, 4, 5 as generated code; M4's suites green
- Depends: 2, 3, **M4 complete**

**Step 5 — goal test + docs.** Contract: the motivating scenario, the
maintenance boundary.
- [ ] `tests/port_layout.rs` complete (all seven scenarios); the
      fixtures-crate `ports` module
- [ ] crate docs: the boundary sentence verbatim, the access table,
      the write-path ladder, the `Pending` asymmetry beside the arrow
      rule; generated-code doc comments carry the entity and field
- [ ] the epic fold verified current (this plan's dock amended
      `epic.md`; check it survived M4's merge if ordering shifted)
- Acceptance:
- [ ] the full goal test green at two implement sites (flat and
      composed)
- Depends: 4

**Overall gate:**
- [ ] the `data` clause end to end on flat and composed patterns,
      embedded records included
- [ ] all three write paths, with the `Pending` rule pinned by test
- [ ] flush carry pinned by the determinism test
- [ ] M1–M4 suites green, unchanged
- [ ] the maintenance boundary in the crate docs, verbatim

## Risks

- **The derive drop.** Records derive `Debug` today; dropping it for
  data-carrying records must not regress internal tests. Records are
  module-private, so the blast radius is the implement module's own
  tests — audit in step 2, and if a test needs record `Debug`, it
  needs it through the gateway instead.
- **Two vocabularies.** `Option<T>` data and optional-`one` arrows
  overlap in shape; the boundary sentence is the decider and appears
  in the docs twice on purpose. The optional-`one` grammar seam
  (Interactions) is the real confusion source — schedule it
  consciously.
- **Staging-name collisions** are the one grammar wart; the
  entity-qualified spelling rule keeps them deterministic and
  trybuild-pinned, and reads stay collision-free by construction.
- **Full-rebuild cost.** Data joins M4's stated deep-copy cost; large
  payloads make flush proportionally dearer. Accepted under the same
  regime; persistent structures remain M4's named future.

## Depends on

M3 complete for steps 1–3 at minimum (the declaration-driven codegen
is the premise — data is a new contribution kind over that machinery).
M4 complete for steps 4–5 (the transaction surface). Sequencing note:
steps 1–3 are M4-independent and may proceed while M4 implements; the
write side is where the two meet, and the carry rule needs nothing
from M4 beyond its already-specified structural copy.

## Frontier carried

- **Vocabulary payloads** (arrow-shaped, trait-visible) — the epic's
  reshaped thread; decider per field is the maintenance boundary.
- **Store-level data** (a `data store { … }` singleton clause) —
  rejected-for-now with the unfused-root rejection, not designed.
- **`Default`-init beyond the closed set**, **user data-validate
  hooks**, **data in introspection/serialization** — M6+ threads.
- **The crossing-levels leftovers**, named during the M5 review
  conversation and awaiting scheduling, none as large as this
  milestone: the optional-`one` pattern grammar (the M4 seam above);
  embedded-record adjacency (the intrusive-at-embedded relaxation M3's
  frontier list already carries); keyed adjacency at an embedded plain
  self (unadjudicated there, and here left to that list); `set_`
  writing path-id values into embedded records (unadjudicated; data
  fields absorb the payload cases, structural links still want it).
