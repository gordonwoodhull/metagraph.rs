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
//! - [`macro@implement`] binds each arrow of a pattern to a *backing* —
//!   a storage strategy contributing fields, mutation hooks, and an
//!   access protocol — and emits the concrete store, handles, and trait
//!   impls at the call site.
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
//! # Backings (M1 built-ins)
//!
//! | backing | for | shape |
//! |---|---|---|
//! | `arena` | `many` | index-range iteration over the target entity's arena; membership is existence (`add_<entity>` is the write) |
//! | `field` | `one` | one typed field on the self record; total by construction |
//! | `intrusive_list` | `many` | `Option<<id>>` head on the self record, next link on the target record; push-front, so iteration is reverse insertion order |
//!
//! `many` arrows bind explicitly (`arena` vs `intrusive_list` is a real
//! choice with no safe default); `one` arrows default to `field`.
//! Derived membership (`by <one-arrow>`) fires an arrow's insert hook
//! from constructors; derived reads (`:= sum(a, b)`) take no backing
//! at all. Every generated store has a `validate()` boundary (a no-op
//! in M1) that later milestones' checks and flush hang on.
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
