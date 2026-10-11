//! metagraph — a toolkit for constructing higher-order graphs.
//!
//! Graph *types* are assembled from atomic pieces:
//!
//! - [`macro@pattern`] declares an abstract type graph: entities and
//!   single-arrow relations ("capabilities") with cardinalities. No storage.
//!   Traits are generated from arrows.
//! - [`macro@implement`] binds each arrow of a pattern to a *backing* — a
//!   storage strategy contributing fields, mutation hooks, and an access
//!   protocol — and emits the concrete store, handles, and trait impls.
//!
//! The base directed graph is not built in: it is assembled from six
//! single-arrow patterns in [`stdlib`]. Generic algorithms ([`algo`]) are
//! written against the stdlib arrow traits and apply to every pattern that
//! instantiates them.
//!
//! ```no_run
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
//! let g = s.graph();
//! let a = g.nodes().next().unwrap();
//! let e = a.out_edges().next().unwrap(); // reverse insertion order
//! assert_eq!(e.target().id(), b);
//! ```

// Generated absolute paths (`::metagraph_exper::…` chosen by
// proc-macro-crate) must resolve inside this crate too: the stdlib is the
// first consumer. Without this alias, expansion inside this crate fails
// with E0433.
extern crate self as metagraph_exper;

pub mod algo;
pub mod stdlib;

pub use metagraph_exper_macros::{implement, pattern};
#[doc(hidden)]
pub use metagraph_exper_macros::__implement_gen;
