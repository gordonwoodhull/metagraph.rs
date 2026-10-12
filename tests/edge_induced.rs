//! Edge-induced subgraph — the milestone-2 goal test. One `implement!` of
//! the composed `EdgeInduced` pattern generates one root store holding the
//! main graph and exactly one subgraph over it: `sub`'s root is unfused, so
//! `sub`'s arenas sit at the store's root level and every SubNode/SubEdge
//! belongs to that single subgraph.
//!
//! The inducing `add_subedge` is hand-written user code — an inherent impl
//! on the generated store — reading endpoints through the builder's
//! id-based read view and deduplicating through the keyed lookup (probe
//! shape: /tmp/m2-probe-edge-induced).

use metagraph_exper::stdlib::{Nodes, OutEdges, Source, Target};
use metagraph_exper::implement;
use metagraph_exper::stdlib::{EdgesOf, MainEdge as _, MainNode as _, SubEdgeOf, SubNodeOf};

mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        EdgeInduced {
            instances:
                main: metagraph_exper::stdlib::Digraph,
                sub:  m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> main.Node;
            sub.MainEdge <-> main.Edge;
        }
    }
}

use comp::edge_induced::markers;

implement! {
    comp::EdgeInduced as EdgeInducedStore {
        main.nodes:     arena,
        main.out_edges: intrusive_list,
        main.in_edges:  intrusive_list,
        sub.subnodes:   arena,
        sub.subedges:   arena,
        sub.subnode:    hash_index,
        sub.subedge:    hash_index,
    }
}

impl EdgeInducedStore {
    /// Inducing constructor: mirror one main edge as a subedge and induce
    /// subnodes for its endpoints, deduplicated via the hash index. This is
    /// constructor logic (not schema vocabulary), written against the
    /// builder's read view — getters take and return ids, never handles.
    pub fn add_subedge(&mut self, me: EdgeId) -> SubEdgeId {
        for endpoint in [self.main_edge_source(me), self.main_edge_target(me)] {
            if self.subnode_by_key(endpoint).is_none() {
                let sn = self.add_sub_node(endpoint);
                self.insert_subnode(endpoint, sn);
            }
        }
        let se = self.add_sub_edge(me);
        self.insert_subedge(me, se);
        se
    }
}

/// Chain plus a skip: n0 -> n1 -> n2, n0 -> n2, and one uninduced node.
fn store() -> (EdgeInducedStore, EdgeId, EdgeId, EdgeId, NodeId) {
    let mut s = EdgeInducedStore::new();
    let n0 = s.add_node();
    let n1 = s.add_node();
    let n2 = s.add_node();
    let n3 = s.add_node();
    let e0 = s.add_edge(n0, n1);
    let e1 = s.add_edge(n1, n2);
    let e2 = s.add_edge(n0, n2); // never induced
    let mut s = s;
    s.add_subedge(e0);
    s.add_subedge(e1);
    (s, e0, e1, e2, n3)
}

#[test]
fn one_store_two_levels() {
    let (s, _e0, _e1, _e2, _n3) = store();
    assert!(s.validate().is_ok());
    // main's graph is intact; sub owns its own arenas at root level.
    assert_eq!(s.node_count(), 4);
    assert_eq!(s.edge_count(), 3);
    assert_eq!(s.sub_node_count(), 3); // n0, n1, n2 — n3 is not a member
    assert_eq!(s.sub_edge_count(), 2);
}

#[test]
fn round_trips_and_none_for_non_members() {
    let (s, e0, e1, e2, n3) = store();
    let sg = s.sub_subgraph();

    // Keyed lookups: every induced endpoint round-trips; `<->` collapses
    // the key to main's plain id type.
    let sn0 = <Subgraph<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&sg, NodeId(0))
        .expect("n0 induced");
    let sn1 = <Subgraph<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&sg, NodeId(1))
        .expect("n1 induced");
    let sn2 = <Subgraph<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&sg, NodeId(2))
        .expect("n2 induced");
    assert_eq!(
        [sn0.id(), sn1.id(), sn2.id()],
        [SubNodeId(0), SubNodeId(1), SubNodeId(2)]
    );
    // Each subnode's main_node round-trips to its key.
    for (sn, n) in [(sn0, 0u32), (sn1, 1), (sn2, 2)] {
        assert_eq!(sn.main_node().id(), NodeId(n));
    }
    // Non-members are None.
    assert!(<Subgraph<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&sg, n3).is_none());

    // Same for subedges.
    let se0 = <Subgraph<'_> as SubEdgeOf<markers::sub::Subedge>>::subedge(&sg, e0)
        .expect("e0 induced");
    let se1 = <Subgraph<'_> as SubEdgeOf<markers::sub::Subedge>>::subedge(&sg, e1)
        .expect("e1 induced");
    assert_eq!([se0.id(), se1.id()], [SubEdgeId(0), SubEdgeId(1)]);
    assert_eq!(se0.main_edge().id(), e0);
    assert!(<Subgraph<'_> as SubEdgeOf<markers::sub::Subedge>>::subedge(&sg, e2).is_none());

    // The subgraph's arena arrows iterate its members.
    let subs: Vec<SubNodeId> = <Subgraph<'_> as Nodes<markers::sub::Subnodes>>::nodes(&sg)
        .map(|sn| sn.id())
        .collect();
    assert_eq!(subs, [SubNodeId(0), SubNodeId(1), SubNodeId(2)]);
    let subedges: Vec<SubEdgeId> =
        <Subgraph<'_> as EdgesOf<markers::sub::Subedges>>::subedges(&sg)
            .map(|se| se.id())
            .collect();
    assert_eq!(subedges, [SubEdgeId(0), SubEdgeId(1)]);
}

#[test]
fn main_graph_still_walks() {
    let (s, _e0, _e1, _e2, _n3) = store();
    // The main graph traverses exactly as a flat Digraph store would.
    let g = s.main_graph();
    let n0 = g.nodes().next().unwrap();
    let outs: Vec<NodeId> =
        <Node<'_> as OutEdges<markers::main::OutEdges>>::out_edges(&n0)
            .map(|e| e.target().id())
            .collect();
    // Push-front: n0's out-edges are n2's edge then n1's edge.
    assert_eq!(outs, [NodeId(2), NodeId(1)]);
    let e = <Node<'_> as OutEdges<markers::main::OutEdges>>::out_edges(&n0).next().unwrap();
    assert_eq!(e.source().id(), NodeId(0));
}

#[test]
fn duplicate_key_rejected_where_the_op_is_submitted() {
    let (mut s, e0, _e1, _e2, _n3) = store();
    let se = s.add_sub_edge(e0);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        s.insert_subedge(e0, se)
    }));
    assert!(r.is_err(), "inserting onto a live key panics");
}
