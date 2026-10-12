//! The composition courier chain (M2 step 5): `compose_pattern!` collects
//! member schemas hop by hop through each member's schema courier, and the
//! final hop mints per-instance markers and emits the composed pattern's
//! own courier. No fusions and no `implement!` here — this pins collection
//! and the diamond property: two instances of one pattern in one
//! composition mint two distinct markers, never a conflicting impl.

use std::any::TypeId;

use metagraph_exper::compose_pattern;

compose_pattern! {
    Trio {
        instances:
            outer: metagraph_exper::stdlib::Digraph,
            inner: metagraph_exper::stdlib::Digraph,
            third: metagraph_exper::stdlib::Digraph;
        // No fusions: three disjoint instances, all arenas at root level.
    }
}

#[test]
fn chain_collects_three_instances() {
    // The composed pattern's markers module exists with one sub-module per
    // instance and one marker per arrow instantiation of that member.
    use trio::markers::{inner, outer, third};

    // Each Digraph member instantiates six roots (nodes, out_edges,
    // in_edges, source, target, edges-sum).
    let _ = outer::Nodes;
    let _ = outer::OutEdges;
    let _ = outer::InEdges;
    let _ = outer::Source;
    let _ = outer::Target;
    let _ = outer::Edges;
    let _ = inner::Nodes;
    let _ = third::Edges;
}

#[test]
fn diamond_markers_are_distinct() {
    // The diamond case: the same root arrow instantiated by two instances
    // of one pattern. Re-minted per instance, so the markers are distinct
    // types — two impls on (potentially) one handle, not a conflict.
    use trio::markers::{inner, outer};
    assert_ne!(TypeId::of::<outer::OutEdges>(), TypeId::of::<inner::OutEdges>());
    assert_ne!(TypeId::of::<outer::Nodes>(), TypeId::of::<inner::Nodes>());
    assert_ne!(TypeId::of::<outer::Nodes>(), TypeId::of::<outer::OutEdges>());
}

#[test]
fn markers_derive_the_debug_clone_copy_shape() {
    use trio::markers::outer::Nodes;
    let m = Nodes;
    let copied = m;
    assert_eq!(format!("{m:?}"), format!("{copied:?}"));
}
