# Milestone 6: the crossing — ports, portals, and the inner graph as a graph

**Goal**: the edge-crossing scenario end to end. An edge leaves a node of
the outer graph, attaches at a port on the host node (the node that
is-also the inner graph), crosses a portal into the embedded graph, and
continues as an ordinary edge among the inner graph's own nodes — and
the inner graph traverses as a graph: every hop of the crossing walk is
a stock root-trait call, no bespoke inner API. Three growths get it
there: the optional-`one` axis, the intrusive head at an embedded
record, and `set_`/flush remap extended into the nested world.

Specification level: preliminary. Designed in the same review
conversation as M5 (2026-10-12, the edge-crossing-levels discussion);
no probe has run — the one genuinely uncertain conjunction (embedded
chains × flush rebuild) is queued as this plan's first-gated probe.
The machinery it rides is grounded: M3 step 6 emits host-anchored
intrusive lists over embedded records (in flight on
`worktree-m3-implement`); the r4-advance probe proved embedded-placement
walks; spike 11 proved hook-written path-id values; M4's nested
machinery (path-addressed ops, composed host∘sub remap) is
probe-proven for keyed carry; M5 lands the `data` clause this plan's
goal test reads through.

## The scenario

One conceptual edge from `n1` (a node of `g1`) to `n4` (a node of `g2`,
embedded at `n0`), encoded — because an edge cannot span levels — as:

- `n0`: the host — `g1.Node <=> g2.Graph`; `g2` is-also a node of `g1`.
- `n2`: a **port** on `n0` — a record in `g1`'s port arena owning a
  boundary location (data, once layout computes it). Invisible when
  drawn; the clipping point of `e1` at `n0`'s visual representation.
- `n3`: a **portal node** of `g2` — an ordinary, invisible node of the
  inner graph where the crossing lands. The port's location, translated
  to inner coordinates, is readable through it (data).
- `e1`: a `g1` edge, `n1 → n0`, attached at `n2` (`dst_port`).
- `e2`: a `g2` edge, `n3 → n4` — the continuation.
- The **portal pairing** `n2 ↔ n3`: `n2 ↦ n3` as a host-owned keyed
  map (`entry`), plus the outward optional link `n3 ↦ n2`
  (`portal_at`).

Layout computation itself is out of scope (it is the client); the
store carries its results — locations as M5 data on ports and inner
nodes, written during build and re-written through flush.

## Premises (what M1–M5 established that this rides on)

- Hosting embeds the guest's element arenas inside each host record;
  embedded records carry path ids `(host, index)`; handles are one
  root-store reference plus an index path (M2).
- The landed embedded-graph vocabulary is the Subgraph shape: member
  bags (existence arenas), host-owned keyed maps with path-id values
  (`subnode`), flat-valued fields on embedded records (`main_node`),
  and — landing with M3 step 6 — host-anchored intrusive lists over
  embedded records (head on the root-level host record, `next` links
  on embedded records).
- M3's uniform init rule defaults `Option<_>` to `None`; its range
  pre-pass already walks contributed `Option<id>` fields, and step 6's
  path-aware pre-pass checks host liveness before leaf bounds.
- M4: the change set is path-addressed (`(index path, op)` pairs); the
  keyed-carry probe proved composed remap (host map ∘ per-host sub
  maps) for path-id **map values**; removal cascades a nested store
  with its host record; `set_` is pinned root-level-only "under M3's
  contributed-field-location restriction" — the sentence this plan
  amends.
- M5: the `data` clause puts `Option<Point>` on any record,
  embedded included, through the `.data()` gateway.

## The scenario as schema

```rust
// fixture roots (fixtures crate, `ports` module; names per the M2 gate)
pattern! { Ports    { entities: root Node, Port;
                      arrows { ports: Node -> Port, many; } } }
pattern! { SrcPort  { entities: root Edge, Port;
                      arrows { src_port: Edge -> Port, one; } } }
pattern! { DstPort  { entities: root Edge, Port;
                      arrows { dst_port: Edge -> Port, one; } } }
pattern! { Owner    { entities: root Port, Node;
                      arrows { owner: Port -> Node, one; } } }
// optional twins (see the optional-`one` section for why they are roots)
pattern! { OptSource{ entities: root Edge, Node;
                      arrows { source: Edge -> Node, one?; } } }
pattern! { OptTarget{ entities: root Edge, Node;
                      arrows { target: Edge -> Node, one?; } } }
// the portal pairing, guest-side
pattern! { EntryOf  { entities: root G, N, OuterPort(open);
                      arrows { entry: G -> N, key(OuterPort); } } }
pattern! { PortalAt { entities: root N, OuterPort;
                      arrows { portal_at: N -> OuterPort, one?; } } }

pattern! {
    PortedDigraph {                       // g1: flat, all landed vocabulary
        entities: root Graph, Node, Edge, Port;
        arrows {
            /* the stdlib six, plus */
            ports:    Ports,   Node -> Port;
            src_port: SrcPort, Edge -> Port;
            dst_port: DstPort, Edge -> Port;
            owner:    Owner,   Port -> Node;
        }
    }
}
pattern! {
    EmbeddedPorted {                       // g2: guest-side, the new machinery
        entities: root G, Node, Edge, Port, OuterPort(open);
        arrows {
            nodes:     stdlib::Nodes,     G -> Node;
            out_edges: stdlib::OutEdges,  Node -> Edge;   // PRIMITIVE — no `by`
            in_edges:  stdlib::InEdges,   Node -> Edge;   // (see below)
            source:    OptSource,         Edge -> Node;   // optional endpoints
            target:    OptTarget,         Edge -> Node;
            ports:     Ports,             Node -> Port;
            entry:     EntryOf,           G -> Node, key(OuterPort);
            portal_at: PortalAt,          Node -> OuterPort;
        }
    }
}
compose_pattern! {
    Crossing {
        instances:
            g1: PortedDigraph,
            g2: EmbeddedPorted;
        g1.Node      <=> g2.G;        // n0: g2 is-also a node of g1
        g2.OuterPort <-> g1.Port;     // the portal key = an outer port
    }
}
```

Two derivations in that schema are load-bearing, so their reasons are
pinned here:

- **`out_edges` is primitive (no `by source`)** for two independent
  causes, either of which would force it: a determinant must be total
  (the by-totality check), and a total `source` at this placement is
  unemittable (below). Membership and endpoints are therefore separate
  facts, joined by a hand-written constructor —
  `add_g2_edge(host, from, to)` over generated primitives
  (`add_g2_edge_record`, `insert_g2_out_edges`, `insert_g2_in_edges`,
  `set_g2_source`, `set_g2_target`), exactly the `add_subedge`
  orchestration precedent. Constructor logic is pattern semantics
  living as ordinary Rust (M4's induction section); nothing here is
  declared.
- **`source`/`target` are optional** because the addressability rule
  stands: a constructor argument cannot supply a host-anchored path id
  (the `Pinned` fixture's rule, kept deliberately). Born `None`, set
  after creation — which makes the inner endpoints optional *as a
  consequence of nesting*, not by choice. The read surface follows
  (optional roots, below).

The storage the composition generates (annotated where M6 changes it):

```rust
struct G1NodeData {                       // n0 — root level, landed
    first_out:   Option<G1EdgeId>,        // g1 adjacency — landed
    first_port:  Option<G1PortId>,        // ports list head — landed
    g2_node_arena: Vec<G2NodeData>,       // n3, n4 — path ids
    g2_edge_arena: Vec<G2EdgeData>,       // e2
    g2_entry_index: HashMap<G1PortId, G2NodeId>,  // n2 ↦ n3 — landed
}
struct G2NodeData {                       // embedded
    first_out: Option<G2EdgeId>,          // ← growth 2: the embedded head
    portal_at: Option<G1PortId>,          // ← growth 1: optional link, flat value
    data: G2NodeUserData,                 // M5 — loc, inner frame
}
struct G2EdgeData {
    next_out:  Option<G2EdgeId>,          // ← growth 2 (link fields on embedded
    next_in:   Option<G2EdgeId>,          //    records — M3 step 6 emits these)
    source:    Option<G2NodeId>,          // ← growth 1 + 4: path-id values,
    target:    Option<G2NodeId>,          //    written by set_
}
```

## Growth 1: the optional-`one` axis

**The word.** `pattern!` grows one production: `one?` (spelling
decision recorded; flip to `one optional` at review if words are
preferred). A `one?` arrow is *at most one, possibly absent,
single-valued, assigned late*: field `Option<BId>`, default `None` per
M3's init rule, read `Option<Handle<B>>`, written by `set_<arrow>`.
No ctor argument in v1 (born `None`; ctor supply and a `clear` verb
are frontier — monotone `Some` only).

**Optionality is owned by the root, exactly like cardinality.** A root
that declares `one` is total forever; a root that declares `one?` owns
the `Option`-returning trait. `EmbeddedPorted` therefore instantiates
*optional twins* (`OptSource`/`OptTarget`), not the stdlib's `Source`/
`Target` — trait identity lives with the root's declaration (invariant
7), and one trait cannot return both shapes. This is the axis M3's
clause table already anticipates ("optional-vs-total is a separate
`one`-arrow axis"), and M4's interface table already carries the write
half (`optional one | e.payload() -> Option<_> | Set<Arrow>`); M6
lands the declaration and closes the seam M5's plan flagged. The
optional root's write trait rides M4's courier write slot like any
arrow-named verb.

**Consequences pinned:**

- `by` requires a total determinant — an optional arrow can never be
  one. Existing by-totality check, new error template.
- The stdlib `field` backing generalizes by selection-on-instantiation-
  facts, the way `validate by`/`validate primitive` select: an optional
  binding contributes `value: Option<BId>`, omits `ctor_arg`
  (`ctor_arg` requires totality — standing rule), and reads `Option`.
  One declaration, two emission shapes; no second backing.
- Validate: `Some` endpoints join the range pre-pass (already emitted
  for `Option<id>` fields); `None` is legal everywhere.
- Flush: a `set_` endpoint naming a removed record is the existing
  set-of-removed `FlushError`; a surviving endpoint is remapped at
  compaction — flat ids through the arena map, path ids through the
  composed remap (growth 3).

## Growth 2: the intrusive head at an embedded record

**The relaxation.** M3's uniform restriction — contributed fields
locate only on root-level classes or the store — is lifted for the one
arm this scenario needs: **the list head on an embedded self record**.
The other arm (link fields on embedded targets, host-anchored heads)
is M3 step 6's, landing now; after both, an embedded graph's adjacency
is fully expressible. The `impl_intrusive_at_embedded` trybuild
fixture is retired and replaced by this plan's positive driver.

**The predicate split.** The closed `requires:` vocabulary grows one
predicate: `addressable_records` — endpoints addressable through their
`rec()` paths, flat or path-id. `intrusive_list` declares it;
`csr_adjacency` keeps `direct_records` (dense arrays genuinely need
flat indices). Extending the vocabulary is a macros-crate change by
design (M3's constraints section says so); error templates quote the
closed set.

**Host-local chains are the only case generated here** — and the
friendly one: every id in a well-formed inner list shares the head's
host (`store.g1_node_arena[h].g2_edge_arena[i]` with `h` pinned for
the walk; the r4-advance probe already pinned that the link walk takes
its host from the path id and never reads the reserved host field).
But nothing *structurally* prevents `insert_g2_out_edges(n3@n0, e@n1)`
— linking another host's edge into `n0`'s list, mixing two
sub-stores. Maps catch exactly this with value-host agreement;
intrusive links have no such check. M6 grows it:

> **Host agreement for embedded links**: a validate body in the
> stdlib `intrusive_list` declaration, authored in the existing role
> vocabulary (`host_of(link) == host_of(head)`, naming offending ids
> in the message) — the field-side twin of value-host agreement, and
> the second half of one uniform discipline (growth 3).

**The M4 obligation — per-host chain rebuild.** Flush rebuilds kept
intrusive lists by walking old chains; at this placement the walk runs
per host and every id translates through the composed remap (host map
∘ per-host sub map). The composed remap is probe-proven for keyed
*map* carry; extending it to link chains is the genuinely uncertain
conjunction in this plan — which is why the probe (below) gates the
implementation steps. Removal of the host record itself needs nothing
new (the inner graph dies with it, M4's nested-store cascade
exception); removal of an inner node exercises the per-host rebuild
and endpoint remap together.

## Growth 3: `set_` and remap in the nested world

M4 pinned `set_` flat-only ("set_-able arrows are root-level-only …
always flat-addressed"). M6 amends it in three moves:

- **(a) Path-addressed records.** `set_` ops ride the existing
  path-addressed queue; the `__set_<arrow>` hook is already
  placement-parametric through `rec()`. Emission work, no new
  mechanism. (`portal_at`: an embedded record's field with a flat
  value — `main_node`'s record/value shape, written late.)
- **(b) Path-id values.** Endpoint checking joins step 6's
  path-aware pre-pass (host liveness before leaf bounds); flush remap
  extends the composed translation from map values to field values —
  `source`/`target` of `e2`, and any optional link holding a path id.
- **(c) The discipline, decided.** The rule that has governed maps
  since value-host agreement is stated once and made uniform:

  > **A pointer into an embedded arena is owned by its host.** A
  > field holding a path id must live on that host's record or inside
  > that host's own embedded arena; `host_of(self) == host_of(value)`,
  > vacuous when the value is flat. This is an `implement!`-time
  > applicability check over placement facts (the field form of
  > value-host agreement, which stays the runtime check for maps).

  Consequences: `source`/`target` (both sides in `n0`'s arena) and
  `portal_at` (flat value) pass; a reified root-level `Portal` record
  holding an inner `G2NodeId` is rejected — that use case stays
  frontier, deliberately. A staged `set_` whose embedded value is
  involved requires `Existing` refs on both sides (a `Pending` host
  can never host-agree — the same reasoning as M4's keyed
  `Existing`-only staging pin, and the same panic family).

## What is deliberately not done

- **Keyed adjacency at an embedded plain self** (a `HashMap` on `n3`'s
  record). Rejected for this milestone with the two reasons the
  scenario supplied: a keyed arrow cannot enumerate (no GAT on keyed
  traits — no walk inside `g2`), and every keyed question the crossing
  actually asks anchors at the **host**, where it is already landed —
  `entry` (port ↦ node) is `subnode`'s shape, and the continuation ask
  (`continues: G -> Edge, key(OuterEdge)` with `OuterEdge <->
  g1.Edge`) is the same landed shape; the goal test asserts it as a
  bonus. Kept on the books as the fallback if the growth-2 relaxation
  proves expensive.
- **Reified portal entities** — blocked by the discipline (c), named
  as frontier with exactly what would unlock them.
- Depth-2 embedding; further composition of composed patterns; a
  one-arrow insert-hook clause; total endpoints at embedded records
  (the addressability rule stands); clearing or ctor-supplying
  optional arrows.

## Vocabulary and promotion

The ports family (`Ports`, `SrcPort`, `DstPort`, `Owner`), the
optional twins (`OptSource`, `OptTarget`), and the portal pair
(`EntryOf`, `PortalAt`) are declared as fixture roots first (fixtures
crate, `ports` module) and drive the goal test; the promotion step
mirrors M3 step 9 — into the runtime stdlib, criterion: the suites
unchanged modulo import paths. The value is trait sharing: a second
ported pattern instantiating `DstPort` gets the same trait, which is
what keeps port-aware generic algorithms portable.

## The goal test

`tests/edge_crossing.rs` (named for the sketch it implements):

1. **The crossing walk** — build the store (outer nodes/edges/ports;
   inner nodes/edges through `add_g2_edge`; portal wiring through
   `insert_g2_entry` + `set_g2_portal_at`), then walk `n1 → n4` with
   every hop a stock trait call:
   `e1.dst_port()` (`DstPort`), `e1.target()` (`Target`, the host),
   `EntryOf::entry(&n0, n2.id())`, `OutEdges::out_edges(&n3)`
   (`markers::g2::OutEdges`, primitive membership),
   `e2.target()` (`OptTarget`, `Option`). Assert the walk reaches
   `n4` and that no step used anything but root traits and handles.
2. **The inner graph is a graph** — a generic traversal (an
   option-endpoint variant of `algo::bfs`) compiles against
   `markers::g2::OutEdges` and produces the inner order; the same
   generic code runs against a flat store at another marker
   instantiation.
3. **Layout, twice** — during build through the read view +
   `data_mut`; then post-freeze re-layout through `edit()` +
   staged `set_data`-family ops + `flush()`, old handles reading old
   locations, the new store the new ones (M5's ladder, exercised at
   embedded placements).
4. **Flush surgery** — remove an inner node: per-host chain rebuild,
   endpoint remap, kept relative order preserved, data carried;
   remove the host `n0`: the inner graph cascades with it.
5. **Corruption fixtures** — a cross-host link planted directly
   (same-module damage, the `validate_failures` style) fails host
   agreement naming both ids; a stale-path endpoint fails the
   path-aware pre-pass; the messages pin.
6. **The host-anchored keyed asks** — `entry` and `continues`
   round-trip (landed shapes, asserted so the not-done section's
   claim stays testable).

## Error templates

| check | fires | error names |
|---|---|---|
| `by` naming an optional determinant | pattern/implement | the arrow, the determinant, the totality rule |
| optional binding on a total root / vice versa | implement | both cardinality-axes (cannot happen through fixture roots — reachable only by mis-declaring; the grammar rejects `one?` on instantiation lines, which never restate cardinality) |
| embedded-valued field on a non-host record (discipline c) | implement | the arrow, self's placement, value's computed arena |
| host-agreement violation (links) | `validate()` | the offending link id, the head's host, the link's host |
| staged `set_` with `Pending` embedded value | staging panic | the op, the `Existing`-only rule |
| `set_` naming a removed endpoint | flush | op index (existing family) |
| unknown `requires:` predicate or bad `one?` spelling | pattern grammar | the closed set / the production, real spans |

## Implementation checklist

Each step is a handoff unit; steps 1 and 3+ are M3-side and M4-side
respectively; the probe gates the uncertain conjunction.

**Step 1 — optional-`one` grammar, flat.** Contract: growth 1.
- [ ] the `one?` production in `grammar.rs` (round-trip, rejections);
      root-declared optionality travels with instantiation like
      cardinality does
- [ ] `field` selection-on-facts: `Option<BId>` contribution, no
      `ctor_arg`, `Option` read; `set_<arrow>` on builder and Tx
      (the M4 row); determinant exclusion template
- Acceptance:
- [ ] flat optional tests (default-`None` read, set-then-read,
      set_-of-removed FlushError); trybuild: `by` on optional
- [ ] M1–M5 suites green
- Depends: M3 steps 1–5, M4 step 7

**Step 2 — PROBE: embedded crossing core × flush.** Hand-emitted
miniature (`/tmp/m6-probe-embedded-crossing`, REPORT.md): one host,
inner arenas with `first/next` links at embedded placement, optional
endpoint fields, path-addressed `set_`; flush scenarios — no-op carry,
inner removal (per-host chain rebuild + composed remap on links *and*
endpoint fields), host removal (cascade), host-agreement corruption
(caught), staged `set_` to removed inner (FlushError). Expected pin:
replay reads only frozen old state; one composed remap serves maps,
links, and fields alike.
- Acceptance: probe green and read; its scenarios become steps 3–4's
  generated tests. Gates both.
- Depends: nothing (hand-emitted).

**Step 3 — embedded-head emission (M3 side).** Contract: growth 2.
- [ ] `addressable_records` predicate (closed set grows; csr keeps
      `direct_records`); retire `impl_intrusive_at_embedded`, replace
      with the positive driver
- [ ] head emission at embedded self; insert op at path-id self;
      walks (r4-advance shape — host from the path id)
- [ ] the host-agreement validate body in the stdlib declaration
      (`host_of` vocabulary); corruption fixture + message pin
- Acceptance:
- [ ] the probe's structural scenarios reproduced as generated code;
      `validate_failures` additions green
- Depends: 2, M3 step 6

**Step 4 — nested `set_` and remap (M4 side).** Contract: growth 3.
- [ ] path-addressed `set_` (a); composed remap extended to field
      values (b); the discipline-(c) applicability check with its
      template; `Existing`-only staging for embedded values
- Acceptance:
- [ ] the probe's flush scenarios reproduced as generated code,
      including endpoint remap on inner-node removal and data carry
- Depends: 1, 2

**Step 5 — vocabulary + goal test.** Contract: the scenario, the goal
test section.
- [ ] fixtures-crate `ports` module (roots as tabled);
      `PortedDigraph`, `EmbeddedPorted`, the `Crossing` composition;
      the hand-written `add_g2_edge` orchestration
- [ ] `tests/edge_crossing.rs`: all six scenarios
- Acceptance:
- [ ] the crossing walk asserts stock-traits-only (test comment pins
      it); the option-endpoint traversal runs at two marker
      instantiations
- Depends: 3, 4, M5

**Step 6 — promotion + docs.** Contract: vocabulary and promotion.
- [ ] promote the ports family, optional twins, and portal pair into
      the stdlib; suites unchanged modulo import paths
- [ ] doc pass: the discipline sentence verbatim ("a pointer into an
      embedded arena is owned by its host") beside M5's boundary
      sentence; M4's flat-only `set_` sentence amended wherever it
      appears; generated-code docs carry the arrow
- [ ] epic fold check (this plan's dock amended `epic.md` and M5's
      frontier pointer; verify current at merge)
- Depends: 5

**Overall gate:**
- [ ] the crossing walk end to end, every hop a stock root-trait call
- [ ] an embedded graph with its own adjacency, traversed by generic
      code at two instantiations
- [ ] optional-`one`, embedded heads, and nested `set_`/remap each
      pinned by tests including their rejections
- [ ] probe's scenarios all reproduced against generated code
- [ ] M1–M5 suites green, unchanged

## Risks

- **Per-host chain rebuild is the likely cost center** — the probe
  exists because M4's rebuild-by-walk was never exercised at nested
  placement; if the probe finds the composed remap insufficient for
  chains (ordering, head repair), step 3's scope grows and the keyed
  fallback (#3, not-done) becomes the live alternative. That is the
  decision point the probe protects.
- **Optional endpoints change the inner read surface** (`Option` on
  every hop through `OptSource`/`OptTarget`). Generic algorithms
  written against total `Target` do not run inside embedded graphs
  unmodified; an adapter trait unifying the two is frontier. Pinned
  as a consequence, not hidden.
- **Fixture retirement** (`impl_intrusive_at_embedded`) must not
  silently delete a pinned `.stderr`; the trybuild suite shrinks by
  one with the driver replacing it — say so in the commit.
- **Grammar spelling** (`one?`) may flip at review; it is one
  production in one parser, and nothing else names it.

## Depends on

M3 complete — specifically step 6 (embedded-target intrusive
emission, the path-aware pre-pass, `records(B)` host×leaf
enumeration). M4 complete (staging, `set_`, nested ops, composed
remap). M5 complete (the `data` clause and gateway the goal test
reads through). Steps 1–2 can start on today's M3/M4 once merged;
steps 3–4 follow their predecessors here.

## Frontier carried

- **Reified portal entities** — unlocked only by relaxing discipline
  (c) for `set_`-written fields with host-liveness validation; the
  crossing does not need them.
- **Keyed adjacency at an embedded plain self** — the fallback, and
  the answer to a different want (edges with stable keyed identity).
- **Depth-2 crossings** (`g3` inside `g2`) — compounds with M2's
  embedding-depth frontier; the pattern is self-similar, the placement
  machinery is not.
- **One-step crossing traversal** (`n1 → n4` as a single derived
  relation) — the M7+ derived-arrows thread; the crossing walk is its
  motivating composition.
- **Ctor supply and `clear` for optional arrows**; **an adapter
  unifying total/optional endpoint traits** for generic algorithms.
- M5's frontier list, minus the items this plan took.
