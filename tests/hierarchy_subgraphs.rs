//! Hierarchy (M2 step 7): an outer graph whose nodes contain subgraphs of
//! the inner graph; both levels traverse. `outer.Node <=> sub.Subgraph`
//! embeds `sub`'s arenas inside each outer node record (path ids, nested
//! handles, per-host counts); `sub.MainNode <-> inner.Node` and
//! `sub.MainEdge <-> inner.Edge` close the open entities by reference.
//!
//! Keyed bindings (`sub.subnode`/`sub.subedge: hash_index`) land with step
//! 8; this step binds the arena arrows over the keyed-free `SubPlain`
//! fixture.

use metagraph_exper::algo::bfs;
use metagraph_exper::stdlib::{Edges, Nodes, OutEdges, Source, Target};
use metagraph_exper::implement;
use m2_fixtures::roots::{EdgesOf, MainEdge as _, MainNode as _};

mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Hierarchy {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph,
                sub:    m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

use comp::hierarchy::markers;

implement! {
    comp::Hierarchy as HierarchyStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
        sub.subnodes:    arena,
        sub.subedges:    arena,
        sub.subnode:     hash_index,
        sub.subedge:     hash_index,
    }
}

fn o0() -> OuterNodeId {
    OuterNodeId(0)
}
fn o1() -> OuterNodeId {
    OuterNodeId(1)
}

fn store() -> HierarchyStore {
    let mut s = HierarchyStore::new();
    // Outer: two nodes, one edge o0 -> o1. Each outer node *is* a subgraph.
    let a = s.add_outer_node();
    let b = s.add_outer_node();
    s.add_outer_edge(a, b);

    // Inner: a three-node chain i0 -> i1 -> i2.
    let i0 = s.add_inner_node();
    let i1 = s.add_inner_node();
    let i2 = s.add_inner_node();
    let ie0 = s.add_inner_edge(i0, i1);
    let _ie1 = s.add_inner_edge(i1, i2);

    // Subnodes/subedges live inside the host outer node's record; they
    // reference the inner graph by plain inner ids (reference fusion
    // unifies the id types, so `main_node: InnerNodeId`).
    let s0 = s.add_sub_node(a, i0);
    let s1 = s.add_sub_node(a, i1);
    let s2 = s.add_sub_node(b, i2);
    assert_eq!(s0, SubNodeId(o0(), 0));
    assert_eq!(s1, SubNodeId(o0(), 1));
    assert_eq!(s2, SubNodeId(o1(), 0));
    let _se = s.add_sub_edge(a, ie0);
    // Keyed arrows: explicit inserts through the hash_index hooks. The
    // self argument is the fused class's id (o0 — every outer node is-also
    // a subgraph); the key is the partner's plain inner id (reference
    // fusion collapses `MainNode` to `InnerNodeId`, no wrapper).
    s.insert_subnode(o0(), i0, s0);
    s.insert_subnode(o0(), i1, s1);
    s.insert_subnode(o1(), i2, s2);
    s.insert_subedge(o0(), ie0, _se);
    s
}

#[test]
fn placement_and_counts() {
    let s = store();
    // Root-level classes get global counts; embedded classes per host.
    assert_eq!(s.outer_node_count(), 2);
    assert_eq!(s.inner_node_count(), 3);
    assert_eq!(s.outer_edge_count(), 1);
    assert_eq!(s.inner_edge_count(), 2);
    assert_eq!(s.sub_node_count(o0()), 2);
    assert_eq!(s.sub_node_count(o1()), 1);
    assert_eq!(s.sub_edge_count(o0()), 1);
    assert_eq!(s.sub_edge_count(o1()), 0);
    assert!(s.validate().is_ok());
}

#[test]
fn two_level_traversal() {
    let s = store();
    // Outer level: generic bfs over the outer graph, qualified by marker
    // (two instances of OutEdges exist in this store).
    let og = s.outer_graph();
    let a = og.nodes().next().unwrap();
    let order: Vec<u32> = bfs::<_, _, markers::outer::OutEdges, _>(a)
        .into_iter()
        .map(|n| n.id().0)
        .collect();
    assert_eq!(order, [0, 1]);

    // Inner level: the same generic bfs, still marker-qualified.
    let ig = s.inner_graph();
    let i0h = ig.nodes().next().unwrap();
    let order: Vec<u32> = bfs::<_, _, markers::inner::OutEdges, _>(i0h)
        .into_iter()
        .map(|n| n.id().0)
        .collect();
    assert_eq!(order, [0, 1, 2]);
}

#[test]
fn fused_handle_reads_sub_arrows_through_the_nesting() {
    let s = store();
    let og = s.outer_graph();
    // The fused handle: every outer node is-also a subgraph. Reading the
    // sub arrows through the *outer node* handle reaches the embedded
    // arenas through the same index path the self record occupies.
    let a = og.nodes().next().unwrap();
    let subs: Vec<SubNodeId> =
        <OuterNode<'_> as Nodes<markers::sub::Subnodes>>::nodes(&a)
            .map(|sn| sn.id())
            .collect();
    assert_eq!(subs, [SubNodeId(o0(), 0), SubNodeId(o0(), 1)]);

    // Each subnode's main_node: a field-backed one-arrow whose open target
    // resolved to the partner's plain inner id.
    let sn0 = <OuterNode<'_> as Nodes<markers::sub::Subnodes>>::nodes(&a)
        .next()
        .unwrap();
    assert_eq!(sn0.main_node().id(), InnerNodeId(0));

    // Subedges iterate per host, and each subedge's main_edge resolves to
    // the partner's plain inner edge id.
    let ses: Vec<SubEdgeId> =
        <OuterNode<'_> as EdgesOf<markers::sub::Subedges>>::subedges(&a)
            .map(|se| se.id())
            .collect();
    assert_eq!(ses, [SubEdgeId(o0(), 0)]);
    assert_eq!(
        <OuterNode<'_> as EdgesOf<markers::sub::Subedges>>::subedges(&a)
            .next()
            .unwrap()
            .main_edge()
            .id(),
        InnerEdgeId(0)
    );
}

#[test]
fn trait_impls_under_instance_markers() {
    let s = store();
    let og = s.outer_graph();
    let a = og.nodes().next().unwrap();
    // Qualification is the norm on multi-instance handles; the full trait
    // path spelling works alongside the partial turbofish.
    let e = <OuterNode<'_> as OutEdges<markers::outer::OutEdges>>::out_edges(&a)
        .next()
        .unwrap();
    assert_eq!(e.target().id(), o1());
    assert_eq!(e.source().id(), o0());

    let ig = s.inner_graph();
    let i0h = ig.nodes().next().unwrap();
    let ie = <InnerNode<'_> as OutEdges<markers::inner::OutEdges>>::out_edges(&i0h)
        .next()
        .unwrap();
    assert_eq!(ie.target().id(), InnerNodeId(1));

    // sum arrows chain their siblings under the instance's marker.
    assert_eq!(
        <OuterNode<'_> as Edges<markers::outer::Edges>>::edges(&a).count(),
        1
    );
    assert_eq!(
        <InnerNode<'_> as Edges<markers::inner::Edges>>::edges(&i0h).count(),
        1
    );
    let _ = e.source().id();
}

#[test]
fn keyed_lookups_round_trip() {
    use m2_fixtures::roots::{SubEdgeOf, SubNodeOf};
    let s = store();
    let og = s.outer_graph();
    let a = og.nodes().next().unwrap();
    let b = og.nodes().nth(1).unwrap();

    // Keyed reads through the fused handle: `<->` collapses the key type to
    // the partner's plain id (`InnerNodeId`, no wrapper).
    let sn0 = <OuterNode<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&a, InnerNodeId(0))
        .expect("i0 is a member of o0's subgraph");
    assert_eq!(sn0.id(), SubNodeId(o0(), 0));
    assert_eq!(sn0.main_node().id(), InnerNodeId(0));

    // Runtime multiplicity is unconstrained by reference fusion: one inner
    // node keys into two different hosts' maps.
    let from_b =
        <OuterNode<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&b, InnerNodeId(2))
            .expect("i2 is a member of o1's subgraph");
    assert_eq!(from_b.id(), SubNodeId(o1(), 0));

    // Non-members and cross-host misses are None (the map is per self).
    assert!(
        <OuterNode<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&a, InnerNodeId(2)).is_none()
    );
    assert!(
        <OuterNode<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&b, InnerNodeId(99))
            .is_none()
    );

    // The keyed subedge read dispatches through its own marker.
    let se = <OuterNode<'_> as SubEdgeOf<markers::sub::Subedge>>::subedge(&a, InnerEdgeId(0))
        .expect("ie0 is a member of o0's subgraph");
    assert_eq!(se.id(), SubEdgeId(o0(), 0));
    assert_eq!(se.main_edge().id(), InnerEdgeId(0));
    assert!(
        <OuterNode<'_> as SubEdgeOf<markers::sub::Subedge>>::subedge(&b, InnerEdgeId(0))
            .is_none()
    );
}

#[test]
#[should_panic(expected = "duplicate key")]
fn duplicate_key_rejected_at_the_insert_call() {
    let mut s = HierarchyStore::new();
    let a = s.add_outer_node();
    let i0 = s.add_inner_node();
    let s0 = s.add_sub_node(a, i0);
    let s1 = s.add_sub_node(a, i0);
    s.insert_subnode(o0(), i0, s0);
    // Inserting onto a live key panics (check-then-write: the hook's
    // live-key predicate is callable without performing the write — M4's
    // staging needs the pure form).
    s.insert_subnode(o0(), i0, s1);
}

#[test]
fn builder_read_view_getters() {
    // The builder read view is id-based: getters take and return ids, never
    // handles. Cross-boundary reads are named `<read-through>_<read>`:
    // `main_edge_source` reads inner's `source` field where the id arrived
    // through sub's `main_edge` closure.
    let mut s = HierarchyStore::new();
    let a = s.add_outer_node();
    let _b = s.add_outer_node();
    let i0 = s.add_inner_node();
    let i1 = s.add_inner_node();
    let ie0 = s.add_inner_edge(i0, i1);
    let sn = s.add_sub_node(a, i0);

    // Same-instance spellings.
    assert_eq!(s.inner_source(ie0), i0); // `source` collides across instances, so the getter prefixes
    assert_eq!(s.main_node(sn), i0);
    // The cross-boundary form: read inner's fields on a main-side id.
    assert_eq!(s.main_edge_source(ie0), i0);
    assert_eq!(s.main_edge_target(ie0), i1);
    // Keyed lookups read the map.
    s.insert_subnode(a, i0, sn);
    assert_eq!(s.subnode_by_key(a, i0), Some(sn));
    assert_eq!(s.subnode_by_key(a, i1), None);
    assert!(s.validate().is_ok());
}
