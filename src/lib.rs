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
//! `Source`, `Target`) and the `Digraph` pattern that instantiates them.
//!
//! ```no_run
//! use metagraph_exper::algo::bfs;
//! use metagraph_exper::implement;
//! use metagraph_exper::stdlib::{Nodes, OutEdges, Target};
//!
//! implement! {
//!     metagraph_exper::stdlib::Digraph as GraphStore {
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
//!
//! `many` arrows bind explicitly (`arena` vs `intrusive_list` is a real
//! choice with no safe default); `one` arrows default to `field`; keyed
//! arrows bind `hash_index`. Derived membership (`by <one-arrow>`) fires
//! an arrow's insert hook from constructors; derived reads
//! (`:= sum(a, b)`) take no backing at all.
//!
//! # Composition (M2)
//!
//! ```no_run
//! mod comp {
//!     use metagraph_exper::compose_pattern;
//!
//!     compose_pattern! {
//!         EdgeInduced {
//!             instances:
//!                 main: metagraph_exper::stdlib::Digraph,
//!                 sub:  m2_fixtures::shapes::Subgraph;
//!             sub.MainNode <-> main.Node;   // sub nodes reference main nodes
//!             sub.MainEdge <-> main.Edge;   // sub edges reference main edges
//!         }
//!     }
//! }
//!
//! use metagraph_exper::implement;
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
//! canonical inventory: keyed maps check key existence, value-in-arena,
//! and value-host agreement; list backings check endpoint ranges and the
//! intrusive-ownership contract (the partition walk for primitive
//! membership, determinant consistency for `by`-derived membership).
//! M4's flush calls it unconditionally.
//!
//! Stores are append-only while being built; handles borrow the store,
//! so the borrow checker freezes it once handles exist — handle identity
//! is (store pointer, id), never the id alone.

// Generated absolute paths (`::metagraph_exper::…` chosen by
// proc-macro-crate) must resolve inside this crate too: the stdlib is the
// first consumer. Without this alias, expansion inside this crate fails
// with E0433.
extern crate self as metagraph_exper;

pub mod algo;
pub mod stdlib;

pub use metagraph_exper_macros::{compose_pattern, implement, pattern};
#[doc(hidden)]
pub use metagraph_exper_macros::{__compose_step, __implement_gen};
