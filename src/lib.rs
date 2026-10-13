//! metagraph — a toolkit for constructing higher-order graphs.
//!
//! Graph *types* are assembled from atomic pieces, in two halves kept
//! strictly apart:
//!
//! - [`macro@pattern`] declares an abstract type graph — entities and
//!   single-arrow relations ("capabilities") with cardinalities. No
//!   storage is mentioned. The *root* single-arrow patterns own their
//!   traits; every other arrow in a pattern *instantiates* a root, so
//!   trait identity lives with the arrow pattern, not with any store.
//! - [`macro@compose_pattern`] composes pattern instances by *fusing*
//!   entities — colocation (`<=>`, is-also: one record, summed
//!   capabilities) or reference (`<->`, closing a member's open entity
//!   onto another instance's entity: id types unify, records stay
//!   separate). Composition only ever adds.
//! - [`macro@implement`] binds each arrow of a pattern — flat or
//!   composed — to a *backing*: a storage strategy contributing fields,
//!   mutation hooks, and an access protocol. It emits the concrete
//!   store, handles, and trait impls at the call site.
//!
//! The same abstract pattern can be implemented multiple ways, and
//! generic algorithms ([`algo`]) written against the arrow traits apply
//! to every store that instantiates them.
//!
//! The base directed graph is not built in: [`stdlib`] declares the six
//! root arrow patterns (`Nodes`, `OutEdges`, `InEdges`, `Edges`,
//! `Source`, `Target`), the `Digraph` pattern that instantiates them,
//! and the composition roots M2 discovered (`EdgesOf`, `MainNode`,
//! `MainEdge`, `SubNodeOf`, `SubEdgeOf` — the subgraph-refinement
//! vocabulary, promoted from the test fixtures).
//!
//! ```no_run
//! use metagraph::algo::bfs;
//! use metagraph::implement;
//! use metagraph::stdlib::{Nodes, OutEdges, Target};
//!
//! implement! {
//!     metagraph::stdlib::Digraph as GraphStore {
//!         nodes:     arena,
//!         out_edges: intrusive_list,
//!         in_edges:  intrusive_list,
//!         // `source`/`target` are `one` arrows: they default to `field`.
//!     }
//! }
//!
//! let mut s = GraphStore::new();
//! let a = s.add_node();
//! let b = s.add_node();
//! s.add_edge(a, b);
//! assert!(s.validate().is_ok());
//! let g = s.graph();                  // the root handle
//! let a = g.nodes().next().unwrap();
//! let e = a.out_edges().next().unwrap(); // reverse insertion order
//! assert_eq!(e.target().id(), b);
//! assert_eq!(bfs(a).len(), 2);        // generic traversal, no markers spelled
//! ```
//!
//! # Backings (built-ins)
//!
//! | backing | for | shape |
//! |---|---|---|
//! | `arena` | `many` | index-range iteration over the target entity's arena; membership is existence (`add_<entity>` is the write) |
//! | `field` | `one` | one typed field on the self record; total by construction |
//! | `intrusive_list` | `many` | `Option<<id>>` head on the self record, next link on the target record; push-front, so iteration is reverse insertion order |
//! | `hash_index` | `keyed` | per-self-object `HashMap` from key id to target id (store-level when the self is an unfused root); inserts onto a live key panic |
//! | `csr_adjacency` | `many` (phased) | per-self buckets accumulate during build, compact into offset ranges + one adjacency array at the single `freeze()`; forward insertion order |
//!
//! `many` arrows bind explicitly (`arena` vs `intrusive_list` vs
//! `csr_adjacency` is a real choice with no safe default); `one` arrows
//! default to `field`; keyed arrows bind `hash_index`. Derived
//! membership (`by <one-arrow>`) fires an arrow's insert hook from
//! constructors; derived reads (`:= sum(a, b)`) take no backing at all.
//!
//! # Composition (M2)
//!
//! ```no_run
//! mod comp {
//!     use metagraph::compose_pattern;
//!
//!     compose_pattern! {
//!         EdgeInduced {
//!             instances:
//!                 main: metagraph::stdlib::Digraph,
//!                 sub:  m2_fixtures::shapes::Subgraph;
//!             sub.MainNode <-> main.Node;   // sub nodes reference main nodes
//!             sub.MainEdge <-> main.Edge;   // sub edges reference main edges
//!         }
//!     }
//! }
//!
//! use metagraph::implement;
//!
//! implement! {
//!     comp::EdgeInduced as Store {
//!         main.nodes:     arena,
//!         main.out_edges: intrusive_list,
//!         main.in_edges:  intrusive_list,
//!         sub.subnodes:   arena,
//!         sub.subedges:   arena,
//!         sub.subnode:    hash_index,
//!         sub.subedge:    hash_index,
//!         // `one` arrows default to `field`.
//!     }
//! }
//!
//! let mut s = Store::new();
//! let a = s.add_node();
//! let b = s.add_node();
//! s.add_edge(a, b);
//! // Induce a subgraph node for main node `a` (hand-written policy —
//! // which nodes induce is not the toolkit's to decide).
//! let sn = s.add_sub_node(a);
//! s.insert_subnode(a, sn);
//! assert!(s.validate().is_ok());
//! assert_eq!(s.sub_node_count(), 1);
//! ```
//!
//! Composition is assembly, never renunciation: instances keep their own
//! arrows and markers. Markers are minted per instance at
//! `<composition module>::<snake(name)>::markers::<instance>::<Arrow>`
//! and are spelled at call sites only where two instances put the same
//! root on one handle:
//!
//! ```ignore
//! <OuterNode<'_> as OutEdges<markers::outer::OutEdges>>::out_edges(&n)
//! <OuterNode<'_> as OutEdges<markers::chain::Elems>>::out_edges(&e)
//! ```
//!
//! The implementing crate must depend directly on every member pattern's
//! declaring crate (renames honored).
//!
//! Every generated store has a `validate()` boundary running the
//! canonical inventory — one authored `validate` clause per backing
//! declaration, summoned in binding order:
//!
//! - `field`: endpoint-in-range over its contributed field.
//! - `arena`: nothing — existence cannot dangle.
//! - `intrusive_list`: the ownership contract — the partition walk for
//!   primitive membership (shared membership distinguished from cycles in
//!   the report), determinant consistency and coverage for `by`-derived
//!   membership.
//! - `hash_index`: key existence, value-in-arena, and value-host
//!   agreement (an embedded-target map holds host-anchored path ids; a
//!   value in-bounds in its own host's arena but filed under another host
//!   fails the agreement check alone).
//! - `csr_adjacency`: the compacted form's invariant — offsets length
//!   against the self arena, the span [0, adjacency length), monotonicity
//!   (assuming compaction has run at `freeze()`).
//!
//! A store binding a phased backing (`csr_adjacency` today) is a
//! type-state store: ONE store type parameterized by a phase marker
//! (`Store<P = Frozen>`), emitted only for such bindings. `add_*`/
//! `insert_*`, the backing hooks, and the consuming `freeze()` exist on
//! the build phase only; handles, iteration, and `validate()` exist on
//! the frozen phase only; `on store build` fields (the CSR accumulators)
//! are released at the type level — their frozen type is `()`. `finish()`
//! is freeze then validate, M4's flush ordering. Non-phased stores keep
//! the single-phase shape: their own builder, no parameter.
//!
//! Before each authored body, a uniform range pre-pass walks every
//! contributed `Option<id>` field of that binding — heads and links, on
//! records and at the store level — and checks each against the target
//! class's arena, path-aware for host-anchored ids (the host component
//! must name a live record before the leaf index is even compared). On
//! damage the pre-pass reports every dead link it found and returns
//! early: the authored walk never runs, so it may assume links in range
//! and use `rec(x)` freely. M4's flush calls `validate()` unconditionally.
//!
//! # Transactions (M4)
//!
//! A store whose every bound arrow classifies — existence, field-one,
//! intrusive, store-level keyed, the hosted per-record keyed map, or the
//! csr walk on a phased backing — also emits its write side beside the
//! read side. A store with an unclassified shape emits no transaction:
//! its `edit()` arrives with the milestone step that defines the carry.
//!
//! `edit()` borrows the store sharedly and starts a transaction;
//! `into_edit()` (non-phased stores only) consumes it, and the
//! flush then reuses its arena allocations. A phased store's
//! transaction is borrowed-only and presupposes the frozen old store —
//! the csr carry reads its frozen ranges, and reads exist only on
//! `Frozen`.
//!
//! Staging queues ops and never touches the store. The verbs are the
//! per-entity `add_*`/`remove_*`, the primitive `insert_<arrow>`
//! (membership asserted directly), the keyed `insert_<arrow>(key,
//! target)` (a key live in the borrowed old store or already staged
//! panics at the call — a staged removal frees no key), and `set_<arrow>`
//! re-points on kept records. Verbs take old-store handles, raw ids
//! (accepted as `Existing`), or the refs other verbs returned
//! (`Pending`, stamped with the transaction's token — a foreign or stale
//! ref panics at staging). The dividing line: staging *panics* are for
//! ops whose acceptance would silently corrupt the change set;
//! `FlushError` is for individually legal ops invalid against the
//! store or change set as a whole — unknown old ids, double removals,
//! danglers (a removed key with live map entries included: the map is
//! carried, never rebuilt, and staged inserts keyed by a removed key
//! count as live references, attributed to the removal op).
//!
//! `flush()` consumes the queue and rebuilds: removals and range checks
//! in old-id space, dangler detection attributed to the op that caused
//! it, compaction with an old→new id report, the kept skeleton
//! (intrusive lists carried by walking the old chains; keyed maps
//! filtered and remapped; a phased store's buckets fed from the old
//! frozen ranges), then submission-order replay through the same
//! constructors and hooks direct construction uses — replay order is
//! observable, and a re-pointed membership takes its replay-time
//! position. The outcome pairs the new store with the report; a phased
//! store's flush ends in `finish()` (freeze, then validate — finalize
//! before validate). `check()` predicates the flush's analysis
//! without building anything. The old store is untouched; a rejection
//! leaves nothing half-applied.
//!
//! Generic algorithms over the write side bound the marker-generic
//! traits — `Add<Entity>`, `Remove<Entity>`, `Insert<Arrow>`,
//! `Set<Arrow>`, and `tx::HandleToRef` (old-store handles → refs) — one
//! marker parameter per entity, the same equality web the read side
//! uses; an inducing constructor that computes its own membership rides
//! the `__stage_custom` escape hatch.
//!
//! ```no_run
//! use metagraph::implement;
//!
//! implement! {
//!     metagraph::stdlib::Digraph as GraphStore {
//!         nodes:     arena,
//!         out_edges: intrusive_list,
//!         in_edges:  intrusive_list,
//!     }
//! }
//!
//! let mut s = GraphStore::new();
//! let a = s.add_node();
//! let b = s.add_node();
//! let e = s.add_edge(a, b);
//!
//! let mut tx = s.edit();
//! tx.remove_edge(e);
//! let c = tx.add_node();
//! tx.add_edge(a, c);
//! let out = tx.flush().expect("coherent change set");
//!
//! // Report maps index OLD ids; staged adds land past the map's end.
//! assert_eq!(out.report.node_map, vec![Some(0), Some(1)]);
//! assert_eq!(out.report.edge_map, vec![None, Some(0)]);
//! assert_eq!(out.store.edge_count(), 2);
//! out.store.validate().expect("flushed store valid");
//! ```
//!
//! Stores are append-only while being built; handles borrow the store,
//! so the borrow checker freezes it once handles exist — handle identity
//! is (store pointer, id), never the id alone.

// Generated absolute paths (`::metagraph::…` chosen by
// proc-macro-crate) must resolve inside this crate too: the stdlib is the
// first consumer. Without this alias, expansion inside this crate fails
// with E0433.
extern crate self as metagraph;

pub mod algo;
pub mod backings;
pub mod stdlib;
pub mod tx;

pub use metagraph_macros::{backing, compose_pattern, implement, pattern};
#[doc(hidden)]
pub use metagraph_macros::{__compose_step, __implement_gen};
