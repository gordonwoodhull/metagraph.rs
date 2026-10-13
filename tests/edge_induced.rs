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
use metagraph_exper::stdlib::{EdgesOf, InEdges, MainEdge as _, MainNode as _, SubEdgeOf, SubNodeOf};

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

// ---- the transaction (M4 step 8: composed Tx emission starts here) ----
//
// The edge-induced store is the composed shape step 8 targets: every
// fusion class at root level, the keyed arrows store-level at the
// unfused sub root. The staged surface mirrors the builder's — including
// the self-less keyed `insert_subnode(key, target)` — and the flush
// carries the keyed maps (filter + remap) beside the intrusive ones.

/// n1's in-edges, walked through the read trait (push-front order).
fn in_ids(s: &EdgeInducedStore, n: NodeId) -> Vec<u32> {
    let g = s.main_graph();
    let node = g.nodes().find(|x| x.id() == n).unwrap();
    <Node<'_> as InEdges<markers::main::InEdges>>::in_edges(&node)
        .map(|e| e.id().0)
        .collect()
}

#[test]
fn tx_stages_the_induced_shape_and_flushes() {
    let (base, _e0, _e1, e2, _n3) = store();
    assert_eq!(base.sub_edge_count(), 2);

    // Mirror the inducing constructor by hand through the Tx: a new
    // subedge for e2 whose endpoints are already keyed (n0 by e0, n2 by
    // e1 — dedup is the user's `subnode_by_key` check, read through
    // `old()` while staging).
    let mut tx = base.edit();
    assert!(tx.old().subnode_by_key(NodeId(0)).is_some());
    assert!(tx.old().subnode_by_key(NodeId(2)).is_some());
    let se = tx.add_sub_edge(e2);
    tx.insert_subedge(e2, se);
    let out = tx.flush().expect("flush");

    assert_eq!(out.store.sub_edge_count(), 3);
    // Old subedges 0, 1 kept (no removals): the staged one is new id 2.
    assert_eq!(out.store.subedge_by_key(e2), Some(SubEdgeId(2)));
    assert_eq!(out.store.subedge_by_key(EdgeId(0)), Some(SubEdgeId(0)));
    assert_eq!(out.store.subnode_by_key(NodeId(1)), Some(SubNodeId(1)));
    out.store.validate().expect("validate");
    // Old store untouched.
    assert_eq!(base.sub_edge_count(), 2);
}

#[test]
fn tx_wholly_staged_keyed_entry_round_trips() {
    let (base, _e0, _e1, _e2, _n3) = store();

    // A staged key record AND a staged target: the refs are Pending, the
    // key check sees no old entry, replay resolves both through the
    // replay table.
    let mut tx = base.edit();
    let n4 = tx.add_node();
    let sn = tx.add_sub_node(n4);
    tx.insert_subnode(n4, sn);
    let out = tx.flush().expect("flush");

    assert_eq!(out.store.node_count(), 5);
    assert_eq!(out.store.sub_node_count(), 4);
    assert_eq!(out.store.subnode_by_key(NodeId(4)), Some(SubNodeId(3)));
    out.store.validate().expect("validate");
}

#[test]
fn tx_handle_to_ref_mints_existing_refs() {
    use metagraph_exper::tx::HandleToRef;

    let (base, _e0, _e1, _e2, _n3) = store();
    let mut tx = base.edit();
    // The marker-generic mapping trait under the composed per-instance
    // entity markers (the only trait surface the composed Tx carries in
    // this step).
    let handle = base.main_graph().nodes().next().unwrap();
    let r: NodeRef = HandleToRef::<markers::main::Node>::to_ref(&tx, handle);
    let _e = tx.add_edge(r, r);
    let out = tx.flush().expect("flush");
    assert_eq!(out.store.edge_count(), 4);
    assert_eq!(out.store.source(EdgeId(3)), NodeId(0));
    out.store.validate().expect("validate");
}

#[test]
#[should_panic(expected = "exists in the borrowed old store")]
fn tx_live_key_panics_at_staging() {
    let (base, _e0, _e1, _e2, _n3) = store();
    let mut tx = base.edit();
    let sn = tx.add_sub_node(NodeId(3));
    tx.insert_subnode(NodeId(0), sn); // n0 keys a subnode: live
}

#[test]
fn tx_removal_splices_intrusive_and_keyed_carries() {
    let (base, _e0, e1, _e2, _n3) = store();

    // Removing a main edge requires removing its subedge explicitly (the
    // minimal cascade): alone, the kept SubEdge's `main_edge` field
    // dangles, attributed to the removal op.
    let err = base
        .edit()
        .tap_flush(|tx| tx.remove_edge(e1))
        .expect_err("dangling main_edge");
    assert_eq!(
        err.violations,
        vec![Violation::DanglingEndpoint {
            op: 0,
            record: RecordDesc::Kept { arena: "sub_edge", id: 1 },
            endpoint_arena: "edge",
            endpoint: 1,
        }]
    );

    // With the subedge removed too, both carries splice: the intrusive
    // in/out lists drop e1, the keyed subedge map drops its entry.
    let out = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_edge(e1);
            tx.remove_sub_edge(SubEdgeId(1));
        })
        .expect("flush");
    assert_eq!(out.store.edge_count(), 2);
    assert_eq!(out.store.sub_edge_count(), 1);
    assert_eq!(out.store.subedge_by_key(e1), None, "entry filtered with its key");
    assert_eq!(out.store.subedge_by_key(EdgeId(0)), Some(SubEdgeId(0)));
    // n2's in-list was [e2, e1] — the walk splices e1 out, and e2
    // compacts to new id 1 (e0 kept -> 0, e1 removed).
    assert_eq!(in_ids(&out.store, NodeId(2)), vec![1]);
    out.store.validate().expect("validate");
}

#[test]
fn tx_set_repoints_by_derived_membership() {
    let (base, e0, _e1, _e2, _n3) = store();

    // Re-point e0's target (n1 -> n2) through the grown re-point hook:
    // n1's in-list loses e0, n2's gains it at the front, and the replay
    // runs against the rebuilt skeleton.
    let out = base
        .edit()
        .tap_flush(|tx| tx.set_target(e0, NodeId(2)))
        .expect("flush");

    assert_eq!(out.store.main_edge_target(e0), NodeId(2));
    assert_eq!(in_ids(&out.store, NodeId(1)), vec![], "e0 unlinked from n1");
    // n2's in-list was [e2, e1]; e0 re-links at the front.
    assert_eq!(in_ids(&out.store, NodeId(2)), vec![0, 2, 1]);
    out.store.validate().expect("validate");
}

/// A per-module flush tap: stage, then flush, in one expression (the
/// tx_flush_shapes pattern).
trait TapFlush {
    fn tap_flush(self, f: impl FnOnce(&mut Tx<'_>)) -> Result<FlushOutcome, FlushError>;
}

impl TapFlush for Tx<'_> {
    fn tap_flush(mut self, f: impl FnOnce(&mut Tx<'_>)) -> Result<FlushOutcome, FlushError> {
        f(&mut self);
        self.flush()
    }
}
