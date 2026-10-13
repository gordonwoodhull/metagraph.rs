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
    // dangles, attributed to the removal op — AND the keyed carry's
    // key-as-reference rule fires (step 11): the subedge map's entry
    // keyed by e1 names a surviving target, so the key removal is
    // rejected naming the entry (sorted (owner, key) pairs; a store-level
    // map's owner column is 0). Both violations attribute to op 0.
    let err = base
        .edit()
        .tap_flush(|tx| tx.remove_edge(e1))
        .expect_err("dangling main_edge");
    assert_eq!(
        err.violations,
        vec![
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "sub_edge", id: 1 },
                endpoint_arena: "edge",
                endpoint: 1,
            },
            Violation::KeyRemovedWithLiveEntries {
                op: 0,
                arena: "edge",
                key: 1,
                entries: vec![(0, 1)],
                staged_inserts: vec![],
            },
        ]
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

// ---- M4 step 10: induction through the escape hatch -----------------------
//
// The inducing constructor over the transaction: a user-written extension
// (`impl Tx<'_>` in this crate — the store and Tx types are generated
// here, so the impl resolves) stages a custom op whose closure replays
// through the SAME inducing constructor direct construction uses. The
// acceptance tests are /tmp/m4-probe-induct-flush's flat variant recast
// against this generated surface (its phased variant is step 12's). One
// semantic translation: the probe's "removal cascades through the keyed
// map" is step 11's survivors-only cascade, so the recast stages the
// explicit minimal cascade this store requires today (remove the main
// record AND its induced sub record), as the step-8 tests already do.

use metagraph_exper::tx::RefKind;

/// A staged node's key in submission space: an old id, or the op index of
/// a node this change set will create.
fn node_key(r: &NodeRef) -> __SubmissionKey {
    match r.kind {
        RefKind::Existing(id) => __SubmissionKey::Old(id.0),
        RefKind::Pending(i) => __SubmissionKey::Pending(i),
    }
}

impl Tx<'_> {
    /// Stage the inducing constructor for main edge `main` (existing or
    /// pending): at flush the closure replays `EdgeInducedStore::add_subedge`
    /// against the rebuilt store, so induction runs mid-replay against the
    /// carried state — the probe's central question.
    ///
    /// The induced subnode keys are computed HERE, at staging: an `Existing`
    /// edge's endpoints are read through `old()`; a `Pending` one's through
    /// the staged `AddEdge` op's endpoint refs (`__ops`). Reserving them on
    /// the custom op makes a later keyed staging call see them as live (the
    /// induct-flush probe's induced-key staging check), in BOTH orderings.
    /// The named id (the `Existing` main edge) is validated by the flush's
    /// steps 1-2 exactly like any other ref.
    pub fn add_subedge(&mut self, main: impl IntoEdgeArg) {
        let main = main.into_edge_ref(self);
        let mut named = Vec::new();
        let mut reserve = Vec::new();
        match main.kind {
            RefKind::Existing(eid) => {
                named.push(__NamedRef::Edge(eid.0));
                // Range-guarded like the probe's `induction_keys_of`: an
                // out-of-range id yields no reserve keys (the flush's step 1
                // reports the listed UnknownOldId for the named id).
                if (eid.0 as usize) < self.old().edge_count() {
                    for ep in
                        [self.old().main_edge_source(eid), self.old().main_edge_target(eid)]
                    {
                        reserve.push(ReservedKey::Subnode(__SubmissionKey::Old(ep.0)));
                    }
                }
            }
            RefKind::Pending(i) => match &self.__ops()[i as usize] {
                Op::AddEdge { source, target } => {
                    for key in [node_key(source), node_key(target)] {
                        reserve.push(ReservedKey::Subnode(key));
                    }
                }
                _ => {}
            },
        }
        self.__stage_custom(
            Box::new(move |store, resolve| {
                store.add_subedge(resolve.resolve_edge(main));
            }),
            named,
            reserve,
        );
    }
}

/// The probe's `old_chain`: n0 -> n1 -> n2, n0 -> n2 (skip), plus uninduced
/// n3. No sub records — assertion 1 induces them during replay.
fn chain() -> (EdgeInducedStore, EdgeId, EdgeId, EdgeId) {
    let mut s = EdgeInducedStore::new();
    let n0 = s.add_node();
    let n1 = s.add_node();
    let n2 = s.add_node();
    let _n3 = s.add_node();
    let e0 = s.add_edge(n0, n1);
    let e1 = s.add_edge(n1, n2);
    let e2 = s.add_edge(n0, n2);
    (s, e0, e1, e2)
}

/// The probe's `old_with_subs`: 5 nodes; e0: 0->1, e1: 1->2, e2: 3->4.
/// Subedges induced for e0 and e2 only: n0,n1,n3,n4 have subnodes, n2 does
/// NOT, and e1 has no subedge. Sub arenas: [main 0, 1, 3, 4] / [main e0, e2].
fn with_subs() -> EdgeInducedStore {
    let mut s = EdgeInducedStore::new();
    let n0 = s.add_node();
    let n1 = s.add_node();
    let n2 = s.add_node();
    let n3 = s.add_node();
    let n4 = s.add_node();
    let e0 = s.add_edge(n0, n1);
    let _e1 = s.add_edge(n1, n2);
    let e2 = s.add_edge(n3, n4);
    s.add_subedge(e0);
    s.add_subedge(e2);
    assert_eq!(s.sub_node_count(), 4);
    assert_eq!(s.sub_edge_count(), 2);
    s.validate().expect("validate");
    s
}

/// The comparable observable (the probe's signature): counts, per-node
/// (out, in) orders, sorted keyed-map contents, sub records' main_* fields.
#[allow(clippy::type_complexity)]
fn sig(
    s: &EdgeInducedStore,
) -> (
    usize,
    usize,
    usize,
    usize,
    Vec<(Vec<u32>, Vec<u32>)>,
    Vec<(u32, u32)>,
    Vec<(u32, u32)>,
    Vec<u32>,
    Vec<u32>,
) {
    let g = s.main_graph();
    let lists = g
        .nodes()
        .map(|n| {
            let out = <Node<'_> as OutEdges<markers::main::OutEdges>>::out_edges(&n)
                .map(|e| e.id().0)
                .collect();
            let in_ = <Node<'_> as InEdges<markers::main::InEdges>>::in_edges(&n)
                .map(|e| e.id().0)
                .collect();
            (out, in_)
        })
        .collect();
    let ni = (0..s.node_count() as u32)
        .filter_map(|k| s.subnode_by_key(NodeId(k)).map(|v| (k, v.0)))
        .collect();
    let ei = (0..s.edge_count() as u32)
        .filter_map(|k| s.subedge_by_key(EdgeId(k)).map(|v| (k, v.0)))
        .collect();
    (
        s.node_count(),
        s.edge_count(),
        s.sub_node_count(),
        s.sub_edge_count(),
        lists,
        ni,
        ei,
        (0..s.sub_node_count() as u32).map(|i| s.main_node(SubNodeId(i)).0).collect(),
        (0..s.sub_edge_count() as u32).map(|i| s.main_edge(SubEdgeId(i)).0).collect(),
    )
}

// -- Assertion 1: induction during replay -----------------------------------

#[test]
fn tx_induction_flush_induces_subnodes_during_replay() {
    let (old, e0, _e1, _e2) = chain();
    assert_eq!(old.sub_node_count(), 0);

    let mut tx = old.edit();
    tx.add_subedge(e0); // op0
    let out = tx.flush().expect("flush");

    // The inducing constructor ran during replay: both endpoints of e0
    // (n0, n1) got subnodes, plus the subedge for e0 itself.
    assert_eq!(out.store.sub_node_count(), 2);
    assert_eq!(out.store.sub_edge_count(), 1);
    assert_eq!(out.store.subnode_by_key(NodeId(0)), Some(SubNodeId(0)));
    assert_eq!(out.store.subnode_by_key(NodeId(1)), Some(SubNodeId(1)));
    assert_eq!(out.store.subnode_by_key(NodeId(2)), None);
    assert_eq!(out.store.subedge_by_key(e0), Some(SubEdgeId(0)));
    // Identity change set: the maps are all identity.
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(out.report.edge_map, vec![Some(0), Some(1), Some(2)]);
    out.store.validate().expect("validate");
}

#[test]
fn tx_induction_dedupes_shared_endpoint_through_keyed_map() {
    let (old, e0, e1, _e2) = chain();
    let mut tx = old.edit();
    tx.add_subedge(e0); // op0: induces n0, n1
    tx.add_subedge(e1); // op1: endpoints n1, n2 — n1 already present
    let out = tx.flush().expect("flush");
    let f = &out.store;

    assert_eq!(f.sub_node_count(), 3);
    assert_eq!(f.sub_edge_count(), 2);
    // EXACTLY ONE subnode per main node: n1's subnode appears once even
    // though two staged subedges named it.
    assert_eq!(
        f.subnode_by_key(NodeId(1)),
        Some(SubNodeId(1)),
        "n1's subnode deduped through the keyed map"
    );
    assert_eq!(f.subedge_by_key(e0), Some(SubEdgeId(0)));
    assert_eq!(f.subedge_by_key(e1), Some(SubEdgeId(1)));
    f.validate().expect("validate");
}

#[test]
fn tx_induction_signature_equals_direct_construction() {
    let (old, e0, e1, _e2) = chain();
    let mut tx = old.edit();
    tx.add_subedge(e0); // op0
    tx.add_subedge(e1); // op1
    let out = tx.flush().expect("flush");

    // Direct construction of the same op sequence: the identity-mapped
    // flush must be indistinguishable (the probe's assertion (a)).
    let mut d = EdgeInducedStore::new();
    let n0 = d.add_node();
    let n1 = d.add_node();
    let n2 = d.add_node();
    d.add_node();
    let de0 = d.add_edge(n0, n1);
    let de1 = d.add_edge(n1, n2);
    d.add_edge(n0, n2);
    d.add_subedge(de0);
    d.add_subedge(de1);

    assert_eq!(sig(&out.store), sig(&d), "flushed == direct");
    out.store.validate().expect("validate");
}

// -- Assertion 2: induction against carried state ---------------------------

#[test]
fn tx_induction_dedupes_against_carried_keyed_map() {
    let old = with_subs();
    let mut tx = old.edit();
    tx.remove_node(NodeId(3)); // op0
    tx.remove_sub_node(SubNodeId(2)); // op1: explicit minimal cascade
    tx.remove_edge(EdgeId(2)); // op2
    tx.remove_sub_edge(SubEdgeId(1)); // op3
    tx.add_subedge(EdgeId(1)); // op4: e1 endpoints n1 (carried!), n2
    let out = tx.flush().expect("flush");
    let f = &out.store;

    // ids shifted: old n4 is new node 3; the explicit cascade removed old
    // n3's subnode and old e2's subedge.
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), None, Some(3)]);
    assert_eq!(out.report.edge_map, vec![Some(0), Some(1), None]);
    assert_eq!(out.report.sub_node_map, vec![Some(0), Some(1), None, Some(2)]);
    assert_eq!(out.report.sub_edge_map, vec![Some(0), None]);

    // The CARRIED map was remapped: old n4's entry moved to new key 3, and
    // its record's main_node moved with it.
    assert_eq!(f.subnode_by_key(NodeId(3)), Some(SubNodeId(2)));
    assert_eq!(f.main_node(SubNodeId(2)), NodeId(3));

    // THE CENTRAL PIN: the replayed induction deduped against the CARRIED
    // map. n1 already had a carried subnode (SubNodeId(1)); no duplicate was
    // created for it, while n2 (never induced) got exactly one NEW subnode
    // appended after the carried records.
    assert_eq!(f.sub_node_count(), 4);
    assert_eq!(f.subnode_by_key(NodeId(1)), Some(SubNodeId(1)));
    assert_eq!(f.subnode_by_key(NodeId(2)), Some(SubNodeId(3)));
    assert_eq!(f.main_node(SubNodeId(3)), NodeId(2));

    assert_eq!(f.sub_edge_count(), 2);
    assert_eq!(f.subedge_by_key(EdgeId(0)), Some(SubEdgeId(0)));
    assert_eq!(f.subedge_by_key(EdgeId(1)), Some(SubEdgeId(1)));
    f.validate().expect("validate");
}

#[test]
fn tx_induction_carried_signature_equals_direct_construction() {
    let old = with_subs();
    let mut tx = old.edit();
    tx.remove_node(NodeId(3));
    tx.remove_sub_node(SubNodeId(2));
    tx.remove_edge(EdgeId(2));
    tx.remove_sub_edge(SubEdgeId(1));
    tx.add_subedge(EdgeId(1));
    let out = tx.flush().expect("flush");

    // Direct construction of the final state, normalized: kept nodes in
    // old-arena order, kept edges likewise, then the sub-record creation
    // order the flush produced. Old n4's subnode was INDUCED by old e2, so
    // the old keyed map held (n4 -> S3) and the flush carries it — the
    // direct build must insert that entry too, or it is not the same state
    // (the probe's failure-1 lesson).
    let mut d = EdgeInducedStore::new();
    let d0 = d.add_node();
    let d1 = d.add_node();
    let d2 = d.add_node();
    let d3 = d.add_node(); // old n4
    let de0 = d.add_edge(d0, d1); // old e0
    let de1 = d.add_edge(d1, d2); // old e1
    d.add_subedge(de0); // subnodes main 0,1; subedge 0
    let carried = d.add_sub_node(d3); // old n4's induced subnode
    d.insert_subnode(d3, carried); // its carried keyed entry
    d.add_subedge(de1); // dedups 1, induces 2; subedge 1

    assert_eq!(sig(&out.store), sig(&d), "flushed == direct");
    out.store.validate().expect("validate");
}

// -- Assertion 4: standalone AddSubNode + induced AddSubedge ----------------

#[test]
fn tx_standalone_and_induced_subnode_flush_together() {
    let (old, _e0, e1, _e2) = chain(); // e1: 1->2
    let mut tx = old.edit();
    let sn = tx.add_sub_node(NodeId(2)); // op0: standalone
    tx.insert_subnode(NodeId(2), sn); // op1: joins the keyed index
    tx.add_subedge(e1); // op2: induces for 1, dedups 2 against op0/1
    let out = tx.flush().expect("flush");
    let f = &out.store;

    assert_eq!(f.sub_node_count(), 2);
    assert_eq!(f.sub_edge_count(), 1);
    // The standalone is NOT duplicated by the induction: the keyed map
    // still points at the STANDALONE's id for n2.
    assert_eq!(f.subnode_by_key(NodeId(2)), Some(SubNodeId(0)), "standalone's id kept");
    assert_eq!(f.subnode_by_key(NodeId(1)), Some(SubNodeId(1)));
    assert_eq!(f.subedge_by_key(e1), Some(SubEdgeId(0)));

    // Signature-equal to the direct interleaving: the standalone joins the
    // keyed index there too (the m2 pattern), which is exactly what lets
    // the induction dedup against it.
    let mut d = EdgeInducedStore::new();
    let n0 = d.add_node();
    let n1 = d.add_node();
    let n2 = d.add_node();
    d.add_node();
    d.add_edge(n0, n1);
    let de1 = d.add_edge(n1, n2);
    d.add_edge(n0, n2);
    let dsn = d.add_sub_node(n2);
    d.insert_subnode(n2, dsn);
    d.add_subedge(de1);
    assert_eq!(sig(f), sig(&d), "flushed == direct");
    f.validate().expect("validate");
}

/// The other ordering, rejected where M4's policy says: a standalone keyed
/// insert whose key a staged inducing op (earlier in the same change set)
/// will induce is a live-key staging panic, not a replay-time overwrite.
#[test]
#[should_panic(expected = "reserved by an already-staged inducing op")]
fn tx_induced_key_panics_at_staging() {
    let (old, _e0, e1, _e2) = chain();
    let mut tx = old.edit();
    tx.add_subedge(e1); // op0 will induce keys for n1, n2
    let sn = tx.add_sub_node(NodeId(2)); // op1
    tx.insert_subnode(NodeId(2), sn); // staging sees op0's reservation: panic
}

/// The carried-key form of the same policy: the borrowed old store's map
/// counts, and a staged removal frees no key.
#[test]
#[should_panic(expected = "exists in the borrowed old store")]
fn tx_carried_key_with_staged_removal_panics_at_staging() {
    let old = with_subs(); // n1 carries a keyed subnode
    let mut tx = old.edit();
    tx.remove_node(NodeId(1)); // frees no key: re-pointing is two flushes
    let sn = tx.add_sub_node(NodeId(1));
    tx.insert_subnode(NodeId(1), sn); // still live at staging: panic
}

/// The Pending-key form: a wholly-staged induction reserves PENDING
/// submission keys, and a standalone keyed onto one panics the same way.
#[test]
#[should_panic(expected = "reserved by an already-staged inducing op")]
fn tx_pending_induced_key_panics_at_staging() {
    let (old, _e0, _e1, _e2) = chain();
    let mut tx = old.edit();
    let n4 = tx.add_node(); // op0
    let e3 = tx.add_edge(n4, NodeId(3)); // op1: endpoints Pending(op0), Old(3)
    tx.add_subedge(e3); // op2 reserves both
    let sn = tx.add_sub_node(n4); // op3
    tx.insert_subnode(n4, sn); // matches the Pending(op0) reservation: panic
}

// -- Assertion 5: induction naming a removed main edge ----------------------

#[test]
fn tx_induction_naming_removed_edge_errors_at_custom_op() {
    let (old, _e0, e1, _e2) = chain();
    let mut tx = old.edit();
    tx.remove_edge(e1); // op0
    tx.add_subedge(e1); // op1: names the removed edge (the `named` id)
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DanglingEndpoint {
            op: 1,
            record: RecordDesc::Staged(1),
            endpoint_arena: "edge",
            endpoint: 1,
        }],
        "attributed to the custom op that named the removed edge: {err}"
    );
}

/// check() consumes the same analysis: the named-ref violation is visible
/// to the predicate exactly as to the flush.
#[test]
fn tx_induction_check_parity_with_flush() {
    let (old, _e0, e1, _e2) = chain();
    let mut tx = old.edit();
    tx.remove_edge(e1);
    tx.add_subedge(e1);
    let check_err = tx.check().expect_err("check sees the naming violation");
    let flush_err = tx.flush().expect_err("flush agrees");
    assert_eq!(check_err, flush_err);
}

/// An out-of-range named id is the listed error, never a panic (the
/// pipeline's range-guard contract).
#[test]
fn tx_induction_out_of_range_named_id_is_listed() {
    let (old, _e0, _e1, _e2) = chain();
    let mut tx = old.edit();
    tx.add_subedge(EdgeId(99)); // op0
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::UnknownOldId { op: 0, arena: "edge", id: 99 }]
    );
}

/// Symmetric naming-op rule for the standalone form (the regular staged-add
/// arm, pinned here for the record).
#[test]
fn tx_standalone_naming_removed_node_errors_at_naming_op() {
    let (old, _e0, _e1, _e2) = chain();
    let mut tx = old.edit();
    tx.remove_node(NodeId(2)); // op0 (kept e1/e2 dangle unless removed)
    tx.remove_edge(EdgeId(1)); // op1
    tx.remove_edge(EdgeId(2)); // op2
    tx.add_sub_node(NodeId(2)); // op3: names the removed node
    let err = tx.flush().unwrap_err();
    assert!(err.violations.contains(&Violation::DanglingEndpoint {
        op: 3,
        record: RecordDesc::Staged(3),
        endpoint_arena: "node",
        endpoint: 2,
    }));
}

// -- bonus coverage ----------------------------------------------------------

#[test]
#[should_panic(expected = "different transaction token")]
fn tx_induction_pending_ref_from_another_tx_panics_at_staging() {
    let (old, _e0, _e1, _e2) = chain();
    let mut tx1 = old.edit();
    let pe = tx1.add_edge(NodeId(0), NodeId(1)); // pending edge
    let mut tx2 = old.edit();
    // Eager stamp check fires HERE, at staging — not at flush.
    tx2.add_subedge(pe);
}

#[test]
fn tx_empty_change_set_flush_is_identity() {
    let old = with_subs();
    let before = sig(&old);
    let out = old.edit().flush().expect("empty tx flushes");
    assert_eq!(sig(&out.store), before, "identity flush");
    assert_eq!(out.report.sub_node_map, vec![Some(0), Some(1), Some(2), Some(3)]);
}

/// A wholly-staged induction: the induced edge is Pending, one endpoint is
/// Pending — reserve keys in submission space, replay resolving the pending
/// edge through the replay table, and the induced subnode appended in
/// replay order.
#[test]
fn tx_induction_wholly_staged_edge() {
    let (old, _e0, _e1, _e2) = chain();
    let mut tx = old.edit();
    let n4 = tx.add_node(); // op0
    let e3 = tx.add_edge(n4, NodeId(3)); // op1: Pending edge
    tx.add_subedge(e3); // op2: reserves Pending(op0), Old(3)
    let out = tx.flush().expect("flush");
    let f = &out.store;

    assert_eq!(f.node_count(), 5);
    assert_eq!(f.edge_count(), 4);
    assert_eq!(f.sub_node_count(), 2);
    assert_eq!(f.sub_edge_count(), 1);
    // The Pending endpoint's subnode and the Existing one's, in induction
    // order; the subedge keyed by the resolved new edge id 3.
    assert_eq!(f.subnode_by_key(NodeId(4)), Some(SubNodeId(0)));
    assert_eq!(f.subnode_by_key(NodeId(3)), Some(SubNodeId(1)));
    assert_eq!(f.subedge_by_key(EdgeId(3)), Some(SubEdgeId(0)));
    assert_eq!(f.main_node(SubNodeId(0)), NodeId(4));
    f.validate().expect("validate");
}

/// The owned path: the same change set through `into_edit` forward-
/// compacts in place, and the custom op replays against the compacted
/// store with ids resolved through the same maps.
#[test]
fn tx_induction_into_edit_owned_path() {
    let old = with_subs();
    let mut tx = old.into_edit();
    tx.remove_node(NodeId(3));
    tx.remove_sub_node(SubNodeId(2));
    tx.remove_edge(EdgeId(2));
    tx.remove_sub_edge(SubEdgeId(1));
    tx.add_subedge(EdgeId(1));
    let out = tx.flush().expect("flush");
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), None, Some(3)]);
    assert_eq!(out.store.sub_node_count(), 4);
    assert_eq!(out.store.subnode_by_key(NodeId(2)), Some(SubNodeId(3)));
    out.store.validate().expect("validate");
}
