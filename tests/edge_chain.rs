//! Edge-chain refinement (M2 step 11): each outer edge is-also a chain of
//! inner edges. `chain.Elem` is open and closed by reference onto
//! `inner.Edge` — there is no Elem arena: chain membership threads through
//! the inner-edge records (a `first_elems` head on the fused outer-edge
//! record, a `next_elems` link on each inner edge), and walking a chain is
//! literally the `out_edges` walk — a generic `OutEdges<M>` traversal
//! applies unchanged.
//!
//! `chain` is the store-less instance: its root is colocation-fused into
//! `outer.Edge` and its only other entity is `<->`-closed, so it owns no
//! arenas — but its `many` arrow still gets an insert op on the store
//! hosting the fused self record.

use metagraph::stdlib::{Nodes, OutEdges};
use metagraph::implement;

mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Refinement {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph,
                chain: m2_fixtures::shapes::Chain;
            outer.Edge  <=> chain.Chain;  // outer edge is-also chain
            chain.Elem <-> inner.Edge;    // elements are inner edges, by reference
        }
    }
}

use comp::refinement::markers;

implement! {
    comp::Refinement as RefinementStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
        chain.elems:     intrusive_list,
    }
}

/// Outer: two chained edges a -> b -> c. Inner: edges the chains thread.
fn store() -> (RefinementStore, OuterEdgeId, OuterEdgeId) {
    let mut s = RefinementStore::new();
    let a = s.add_outer_node();
    let b = s.add_outer_node();
    let c = s.add_outer_node();
    let ab = s.add_outer_edge(a, b);
    let bc = s.add_outer_edge(b, c);

    let i0 = s.add_inner_node();
    let i1 = s.add_inner_node();
    let i2 = s.add_inner_node();
    let i3 = s.add_inner_node();
    let e01 = s.add_inner_edge(i0, i1);
    let e12 = s.add_inner_edge(i1, i2);
    let e23 = s.add_inner_edge(i2, i3);

    // ab refines to e01, e12; bc refines to e23. Insertion order is
    // reversed on walk (push-front), so insert in walk order reversed.
    s.insert_elems(ab, e12);
    s.insert_elems(ab, e01);
    s.insert_elems(bc, e23);
    (s, ab, bc)
}

#[test]
fn chain_walk_is_out_edges() {
    let (s, ab, bc) = store();
    assert!(s.validate().is_ok());
    let og = s.outer_graph();
    let a = og.nodes().next().unwrap();
    let ab_h = <OuterNode<'_> as OutEdges<markers::outer::OutEdges>>::out_edges(&a)
        .next()
        .unwrap();
    assert_eq!(ab_h.id(), ab);

    // Walking the chain is a generic OutEdges<M> traversal applied
    // unchanged — the plan's central claim for this sketch.
    let chain: Vec<InnerEdgeId> =
        <OuterEdge<'_> as OutEdges<markers::chain::Elems>>::out_edges(&ab_h)
            .map(|e| e.id())
            .collect();
    assert_eq!(chain, [InnerEdgeId(0), InnerEdgeId(1)]);

    let b = og.nodes().nth(1).unwrap();
    let bc_h = <OuterNode<'_> as OutEdges<markers::outer::OutEdges>>::out_edges(&b)
        .next()
        .unwrap();
    assert_eq!(bc_h.id(), bc);
    let chain: Vec<InnerEdgeId> =
        <OuterEdge<'_> as OutEdges<markers::chain::Elems>>::out_edges(&bc_h)
            .map(|e| e.id())
            .collect();
    assert_eq!(chain, [InnerEdgeId(2)]);
}

#[test]
fn both_graphs_intact() {
    let (s, _ab, _bc) = store();
    assert_eq!(s.outer_node_count(), 3);
    assert_eq!(s.outer_edge_count(), 2);
    assert_eq!(s.inner_node_count(), 4);
    assert_eq!(s.inner_edge_count(), 3);
    // The store-less instance owns no arena and no count: there is no
    // chain arena, only the fused head fields on outer-edge records.
    let ig = s.inner_graph();
    let i0 = ig.nodes().next().unwrap();
    let outs: Vec<InnerEdgeId> =
        <InnerNode<'_> as OutEdges<markers::inner::OutEdges>>::out_edges(&i0)
            .map(|e| e.id())
            .collect();
    assert_eq!(outs, [InnerEdgeId(0)]);
}

#[test]
fn store_less_instance_still_has_write_ops() {
    // `chain` owns no arenas, but its `many` arrow's insert op is emitted on
    // the store hosting the fused self record, named from the local arrow
    // name by the usual convention — `insert_elems(OuterEdgeId, InnerEdgeId)`
    // (the spike's `insert_chain_elem` was a deviation, not a precedent).
    let (mut s, ab, _bc) = store();
    let i3 = s.add_inner_node();
    let _i4 = s.add_inner_node();
    let e34 = s.add_inner_edge(i3, _i4);
    s.insert_elems(ab, e34); // push-front: e34 first
    let og = s.outer_graph();
    let a = og.nodes().next().unwrap();
    let ab_h = <OuterNode<'_> as OutEdges<markers::outer::OutEdges>>::out_edges(&a)
        .next()
        .unwrap();
    let chain: Vec<InnerEdgeId> =
        <OuterEdge<'_> as OutEdges<markers::chain::Elems>>::out_edges(&ab_h)
            .map(|e| e.id())
            .collect();
    assert_eq!(chain, [e34, InnerEdgeId(0), InnerEdgeId(1)]);
}
