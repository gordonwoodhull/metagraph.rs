//! M4 step 7 — `set_` staging + the grown re-point hooks: probe 3
//! (/tmp/m4-probe-set-unlink) reproduced against the generated surface.
//!
//! The builder's `set_source` and the Tx staging verb lower through the
//! SAME grown `__set_source` hook (unlink the by-derived memberships the
//! field determines, write, re-link under the new endpoint), so a flush
//! replays a staged `set_` exactly as the direct build plays the builder
//! verb. Dangler detection runs on the HONOR reading: a kept edge whose
//! old source the change set removes dangles only if no staged `set_`
//! re-points it, and a `set_` naming a removed endpoint dangles at the
//! set op. The honor skeleton writes the effective endpoint and is born
//! unlinked (the removed old host's chain is skipped by the walk-rebuild);
//! a Pending effective endpoint is predicted (kept count + preceding adds)
//! and asserted at replay, never trusted.

use metagraph::implement;
use metagraph::stdlib::{InEdges, Nodes, OutEdges, Source, Target};

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        // `source`/`target` are `one` arrows: field-backed by default.
    }
}

// ---- helpers (the probe's store, against the generated GraphStore) -------

/// The probe's old store: 4 nodes, 6 edges, interesting chains.
///   e0: n1->n0  e1: n2->n0  e2: n3->n0   (n0's in-list [e2,e1,e0])
///   e3: n0->n1  e4: n0->n2  e5: n0->n3   (n0's out-list [e5,e4,e3]:
///                                         e3 tail, e4 middle, e5 head)
fn build_old() -> GraphStore {
    let mut b = GraphStore::new();
    let n0 = b.add_node();
    let n1 = b.add_node();
    let n2 = b.add_node();
    let n3 = b.add_node();
    assert_eq!([n0, n1, n2, n3], [NodeId(0), NodeId(1), NodeId(2), NodeId(3)]);
    let e0 = b.add_edge(n1, n0);
    let e1 = b.add_edge(n2, n0);
    let e2 = b.add_edge(n3, n0);
    let e3 = b.add_edge(n0, n1);
    let e4 = b.add_edge(n0, n2);
    let e5 = b.add_edge(n0, n3);
    assert_eq!([e0, e1, e2, e3, e4, e5], [EdgeId(0), EdgeId(1), EdgeId(2), EdgeId(3), EdgeId(4), EdgeId(5)]);
    b
}

fn out_ids(s: &GraphStore, n: NodeId) -> Vec<u32> {
    s.graph()
        .nodes()
        .find(|x| x.id() == n)
        .unwrap()
        .out_edges()
        .map(|e| e.id().0)
        .collect()
}

fn in_ids(s: &GraphStore, n: NodeId) -> Vec<u32> {
    s.graph()
        .nodes()
        .find(|x| x.id() == n)
        .unwrap()
        .in_edges()
        .map(|e| e.id().0)
        .collect()
}

/// Whole-store signature: counts plus per-node (out, in) iteration orders
/// as endpoint id pairs — the flush-must-match-direct observable.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Sig {
    node_count: usize,
    edge_count: usize,
    adjacency: Vec<(Vec<(u32, u32)>, Vec<(u32, u32)>)>,
}

fn signature(s: &GraphStore) -> Sig {
    let g = s.graph();
    let adjacency = g
        .nodes()
        .map(|n| {
            let out = n.out_edges().map(|e| (e.source().id().0, e.target().id().0)).collect();
            let inc = n.in_edges().map(|e| (e.source().id().0, e.target().id().0)).collect();
            (out, inc)
        })
        .collect();
    Sig { node_count: s.node_count(), edge_count: s.edge_count(), adjacency }
}

// ---- probe 3's assertions, against generated code ------------------------

/// Assertion 1 — flush with staged set_ ops (middle / head / tail of the
/// old out-list), an add_node + add_edge staged BEFORE them. The direct
/// build replays the SAME verbs in submission order (base inserts, the
/// staged add, then the same set_ calls through the builder): a
/// born-to-final-source build would NOT match — a re-pointed edge's list
/// position depends on replay time (push-front), not its birth position.
#[test]
fn flush_set_source_mid_head_tail_with_interleaved_adds() {
    let old = build_old();
    let mut tx = old.edit();
    let p = tx.add_node(); // op0
    tx.add_edge(p, NodeId(0)); // op1 -> e6: pending->n0
    tx.set_source(EdgeId(4), NodeId(1)); // op2: e4, middle of n0's out
    tx.set_source(EdgeId(5), NodeId(2)); // op3: e5, head of n0's out
    tx.set_source(EdgeId(3), p); // op4: e3, tail -> the staged node
    let out = tx.flush().expect("flush");
    let f = &out.store;

    assert_eq!(f.node_count(), 5);
    assert_eq!(f.edge_count(), 7);
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(out.report.edge_map, vec![Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]);

    // Push-front onto the new host: e4 (re-pointed) iterates FIRST on n1,
    // ahead of e0 which was inserted earlier.
    assert_eq!(out_ids(f, NodeId(1)), vec![4, 0]);
    // Un-linked from the old host by splicing — no truncation of
    // neighbors: e5 then e4 then e3 each left individually.
    assert_eq!(out_ids(f, NodeId(0)), Vec::<u32>::new());
    assert_eq!(out_ids(f, NodeId(2)), vec![5, 1]);
    assert_eq!(out_ids(f, NodeId(4)), vec![3, 6]);

    // In-lists are untouched by the set_ ops (set_source never moves the
    // target) except n0's, which legitimately gained the staged e6.
    for i in 1..4u32 {
        assert_eq!(in_ids(f, NodeId(i)), in_ids(&old, NodeId(i)), "in-list of n{i}");
    }
    assert_eq!(in_ids(f, NodeId(0)), vec![6, 2, 1, 0]);

    // Signature equality vs the direct build (the same-verbs normalization).
    let mut d = GraphStore::new();
    let (d0, d1, d2, d3) = (d.add_node(), d.add_node(), d.add_node(), d.add_node());
    d.add_edge(d1, d0); // e0
    d.add_edge(d2, d0); // e1
    d.add_edge(d3, d0); // e2
    d.add_edge(d0, d1); // e3
    d.add_edge(d0, d2); // e4
    d.add_edge(d0, d3); // e5
    let d4 = d.add_node();
    d.add_edge(d4, d0); // e6 (the staged add)
    d.set_source(EdgeId(4), d1);
    d.set_source(EdgeId(5), d2);
    d.set_source(EdgeId(3), d4);
    assert_eq!(signature(f), signature(&d), "flushed == direct build");
    assert!(f.validate().is_ok());
}

/// Assertion 2 — remove node A (old source of e4) AND stage
/// set_source(e4, B): the HONOR reading flushes; e4 lands on B. (Assertion
/// 3 is the same change set without the set_: a FlushError.)
#[test]
fn set_source_honored_against_removed_old_source() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_node(NodeId(0)); // op0: A, the old source of e4
    tx.remove_edge(EdgeId(0)); // op1..op5: every OTHER edge incident to A
    tx.remove_edge(EdgeId(1)); //   (in-edges cannot be cured by set_source
    tx.remove_edge(EdgeId(2)); //   — it never moves a target)
    tx.remove_edge(EdgeId(3));
    tx.remove_edge(EdgeId(5));
    tx.set_source(EdgeId(4), NodeId(1)); // op6: re-point onto B
    let out = tx.flush().expect("honor reading: no dangler for e4");
    let f = &out.store;

    // n1,n2,n3 compact to 0,1,2; e4 is the only kept edge (new id 0).
    assert_eq!(f.node_count(), 3);
    assert_eq!(f.edge_count(), 1);
    assert_eq!(out.report.node_map, vec![None, Some(0), Some(1), Some(2)]);
    assert_eq!(out.report.edge_map, vec![None, None, None, None, Some(0), None]);
    // e4's skeleton named its EFFECTIVE source (B = new 0) and was born
    // unlinked; the replay's re-point hook linked it under B.
    assert_eq!(out_ids(f, NodeId(0)), vec![0]);
    assert_eq!(in_ids(f, NodeId(1)), vec![0]); // target n2 untouched
    assert!(f.validate().is_ok());
}

/// Assertion 3 — the SAME change set without the set_: FlushError naming
/// the dangler, attributed to the REMOVAL op.
#[test]
fn same_change_set_without_set_source_dangles() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_node(NodeId(0)); // op0
    for e in [0u32, 1, 2, 3, 5] {
        tx.remove_edge(EdgeId(e)); // op1..op5
    }
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DanglingEndpoint {
            op: 0,
            record: RecordDesc::Kept { arena: "edge", id: 4 },
            endpoint_arena: "node",
            endpoint: 0,
        }]
    );
    let msg = err.to_string();
    assert!(msg.contains("id: 4"), "names the dangler: {msg}");
    assert!(msg.contains("endpoint: 0"), "names the removed endpoint: {msg}");
}

/// Assertion 4 — a kept-edge set_ replays AFTER the skeleton: the old
/// host's final out-list is the walk-rebuild WITHOUT the edge (computed
/// from the old store), and the edge iterates first on the new host even
/// though it sat mid-chain in the old store.
#[test]
fn kept_edge_replays_after_skeleton_mid_chain() {
    let old = build_old();
    let mut tx = old.edit();
    tx.set_source(EdgeId(4), NodeId(1)); // op0 only
    let out = tx.flush().expect("flush");
    let f = &out.store;

    // Old host's final list == walk of its OLD chain, e4 skipped.
    let walked_without_e: Vec<u32> = out_ids(&old, NodeId(0)).into_iter().filter(|&e| e != 4).collect();
    assert_eq!(walked_without_e, vec![5, 3]);
    assert_eq!(out_ids(f, NodeId(0)), walked_without_e);
    assert_eq!(out_ids(f, NodeId(1)), vec![4, 0]);
    assert_eq!(in_ids(f, NodeId(2)), in_ids(&old, NodeId(2)));

    let mut d = GraphStore::new();
    let (d0, d1, d2, d3) = (d.add_node(), d.add_node(), d.add_node(), d.add_node());
    d.add_edge(d1, d0);
    d.add_edge(d2, d0);
    d.add_edge(d3, d0);
    d.add_edge(d0, d1);
    d.add_edge(d0, d2);
    d.add_edge(d0, d3);
    d.set_source(EdgeId(4), d1);
    assert_eq!(signature(f), signature(&d));
    assert!(f.validate().is_ok());
}

/// Assertion 5 — set_ of a REMOVED edge: FlushError naming both ops
/// (SetOfRemoved — decision 7's generic spelling).
#[test]
fn set_source_of_removed_edge_errors() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_edge(EdgeId(4)); // op0
    tx.set_source(EdgeId(4), NodeId(1)); // op1
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::SetOfRemoved { op: 1, arena: "edge", id: 4, removed_by: 0 }]
    );
}

/// Assertion 6 — double set_ (x then y): both replay in submission order,
/// last wins, and the intermediate host's list is left splice-clean. y is
/// a PENDING node, so the second replay resolves through the replay table.
#[test]
fn double_set_source_last_wins() {
    let old = build_old();
    let mut tx = old.edit();
    tx.set_source(EdgeId(4), NodeId(1)); // op0: x
    let p = tx.add_node(); // op1
    tx.set_source(EdgeId(4), p); // op2: y (pending)
    let out = tx.flush().expect("flush");
    let f = &out.store;

    assert_eq!(out_ids(f, NodeId(1)), vec![0], "x's list no longer holds e4");
    assert_eq!(out_ids(f, NodeId(4)), vec![4], "last set_ wins: e4 on y");
    assert_eq!(out_ids(f, NodeId(0)), vec![5, 3], "original host splice-clean");
    assert_eq!(in_ids(f, NodeId(2)), vec![4], "target untouched throughout");
    assert!(f.validate().is_ok());
}

/// Honor case with a PENDING new source: the skeleton record's source
/// field must be a concrete id before replay exists — the prediction
/// (kept count + adds before the pending add), asserted at replay.
#[test]
fn honor_case_with_pending_new_source() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_node(NodeId(0)); // op0
    for e in [0u32, 1, 2, 3, 5] {
        tx.remove_edge(EdgeId(e)); // op1..op5
    }
    let p = tx.add_node(); // op6
    tx.set_source(EdgeId(4), p); // op7: -> the pending node
    let out = tx.flush().expect("pending new source resolves at replay");
    let f = &out.store;
    // kept = 3 nodes; the one AddNode precedes op7 -> prediction NodeId(3).
    assert_eq!(out_ids(f, NodeId(3)), vec![0]);
    assert!(f.validate().is_ok());
}

/// set_source canNOT cure a target-side dangler: re-pointing e3's source
/// away from removed n1 (its TARGET) leaves it dangling, attributed to
/// the removal op; e0 dangles for the plain reason. Nothing is attributed
/// to the set op — it did its job; the target side is out of its reach.
#[test]
fn set_source_cannot_cure_target_dangler() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_node(NodeId(1)); // op0: n1 is e3's target and e0's source
    tx.set_source(EdgeId(3), NodeId(2)); // op1: moves the SOURCE only
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "edge", id: 0 },
                endpoint_arena: "node",
                endpoint: 1,
            },
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "edge", id: 3 },
                endpoint_arena: "node",
                endpoint: 1,
            },
        ]
    );
}

/// A set_ op NAMING a removed node as its new source dangles at the SET
/// op (the staged-op-names-removed-node precedent), not at the removal —
/// plus the plain danglers from the removal itself, attributed to op0.
#[test]
fn set_source_to_removed_node_dangles_at_set_op() {
    let old = build_old();
    let mut tx = old.edit();
    tx.remove_node(NodeId(2)); // op0: removes e1's source, e4's target
    tx.set_source(EdgeId(3), NodeId(2)); // op1: names the removed node
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![
            Violation::DanglingEndpoint {
                op: 1,
                record: RecordDesc::Kept { arena: "edge", id: 3 },
                endpoint_arena: "node",
                endpoint: 2,
            },
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "edge", id: 1 },
                endpoint_arena: "node",
                endpoint: 2,
            },
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "edge", id: 4 },
                endpoint_arena: "node",
                endpoint: 2,
            },
        ]
    );
}

/// A set_ naming an out-of-range raw id is the listed error, never an
/// index panic (the spike-7 class, extended to the new op's refs).
#[test]
fn set_source_out_of_range_id_is_listed_not_a_panic() {
    let old = build_old();
    let mut tx = old.edit();
    tx.set_source(EdgeId(3), NodeId(99)); // op0: unknown old node id
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::UnknownOldId { op: 0, arena: "node", id: 99 }]
    );
}

// ---- the write trait + the check()-then-flush contract -------------------

/// The `Set<Arrow>` trait verb is the trait-bound form of the inherent
/// `set_source`: same op, same flush (step 6 emitted the trait; step 7
/// wires the Tx impl).
#[test]
fn set_source_stages_through_the_trait() {
    use metagraph::stdlib::SetSource;

    let old = build_old();
    let via_trait = {
        let mut tx = old.edit();
        SetSource::set_source(&mut tx, EdgeRef::existing(EdgeId(4)), NodeRef::existing(NodeId(1)));
        tx.flush().expect("trait flush")
    };
    let via_inherent = {
        let mut tx = old.edit();
        tx.set_source(EdgeId(4), NodeId(1));
        tx.flush().expect("inherent flush")
    };
    assert_eq!(via_trait.report.edge_map, via_inherent.report.edge_map);
    assert_eq!(signature(&via_trait.store), signature(&via_inherent.store));
    assert!(via_trait.store.validate().is_ok());
}

/// The committed-mode contract on a change set with set_ ops: a green
/// `check()` means the flush of the SAME change set passes steps 1-2 and
/// commits; a red one reports byte-identical violations either way.
#[test]
fn check_and_flush_agree_on_set_change_sets() {
    let old = build_old();

    // Green: an honor change set — check passes, the flush commits, and
    // the reports agree with a flush staged independently.
    let mut tx = old.edit();
    tx.remove_node(NodeId(0));
    for e in [0u32, 1, 2, 3, 5] {
        tx.remove_edge(EdgeId(e));
    }
    tx.set_source(EdgeId(4), NodeId(1));
    tx.check().expect("green check");
    let checked = tx.flush().expect("committing flush");
    assert!(checked.store.validate().is_ok());

    // Red: the same set_ naming a removed node — check's violations are
    // exactly flush's (same analysis, same discovery order).
    let mut tx = old.edit();
    tx.remove_node(NodeId(2));
    tx.set_source(EdgeId(3), NodeId(2));
    let from_check = tx.check().unwrap_err();
    let from_flush = tx.flush().unwrap_err();
    assert_eq!(from_check.violations, from_flush.violations);
    assert_eq!(from_check.to_string(), from_flush.to_string());
}
