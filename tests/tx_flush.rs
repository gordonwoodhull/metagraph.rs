//! M4 step 3 — the flush pipeline against generated code: spike 7's tx
//! suite reproduced against the `implement!`-emitted `Tx` (Königsberg
//! signature equality, removal/compaction/danglers, the report, ref
//! resolution), plus the out-of-range `Existing` repro (the spike-7 bug:
//! an id flagged in step 1 must never be indexed in step 2 — the flush
//! returns the listed error, it does not panic).
//!
//! Ported from /tmp/spike7-tx/tests (the reference implementation) with
//! three deviations, all deferred by the plan's step order: the
//! `into_edit()` entry point and its tests land with step 5 (every test
//! here goes through `edit()`), the four staging panics already landed
//! with step 2 (tests/tx_staging_panics.rs), and `check()` is step 4.

use metagraph::algo::{bfs, dfs};
use metagraph::implement;
use metagraph::stdlib::{Edges, InEdges, Nodes, OutEdges, Source, Target};
use metagraph::tx::RefKind;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        // `source`/`target` are `one` arrows: field-backed by default.
    }
}

const A: u32 = 0;
const B: u32 = 1;
const C: u32 = 2;
const D: u32 = 3;

// ---- helpers (spike 7's common module, against the generated store) ----

/// The canonical observable signature: counts, adjacency degrees, and
/// iteration orders (per-node out/in lists in intrusive-list order).
#[derive(Clone, PartialEq, Eq, Debug)]
struct Sig {
    node_count: usize,
    edge_count: usize,
    /// Per node, in node-iteration order: (out-list, in-list), each as
    /// (source, target) id pairs in iteration order.
    adjacency: Vec<(Vec<(u32, u32)>, Vec<(u32, u32)>)>,
}

fn signature(s: &GraphStore) -> Sig {
    let g = s.graph();
    let adjacency = g
        .nodes()
        .map(|n| {
            let out = n
                .out_edges()
                .map(|e| (e.source().id().0, e.target().id().0))
                .collect();
            let inc = n
                .in_edges()
                .map(|e| (e.source().id().0, e.target().id().0))
                .collect();
            (out, inc)
        })
        .collect();
    Sig {
        node_count: s.node_count(),
        edge_count: s.edge_count(),
        adjacency,
    }
}

/// Build Königsberg directly, exactly as tests/koenigsberg.rs does.
fn koenigsberg() -> GraphStore {
    let mut s = GraphStore::new();
    let a = s.add_node();
    let b = s.add_node();
    let c = s.add_node();
    let d = s.add_node();
    assert_eq!([a.0, b.0, c.0, d.0], [A, B, C, D]);

    // Two bridges A–B, two A–C, one A–D, one B–D, one C–D;
    // each bridge is a pair of directed edges.
    for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
        s.add_edge(x, y);
        s.add_edge(y, x);
    }
    s
}

/// Three nodes A,B,C; edges e0: A->B, e1: A->C, e2: B->C.
fn triangle() -> GraphStore {
    let mut s = GraphStore::new();
    let a = s.add_node();
    let b = s.add_node();
    let c = s.add_node();
    s.add_edge(a, b);
    s.add_edge(a, c);
    s.add_edge(b, c);
    s
}

trait TapFlush {
    fn tap_flush(
        self,
        f: impl FnOnce(&mut Tx<'_>),
    ) -> Result<FlushOutcome, FlushError>;
}

impl TapFlush for Tx<'_> {
    fn tap_flush(
        mut self,
        f: impl FnOnce(&mut Tx<'_>),
    ) -> Result<FlushOutcome, FlushError> {
        f(&mut self);
        self.flush()
    }
}

fn node_ids(order: Vec<Node<'_>>) -> Vec<u32> {
    order.into_iter().map(|n| n.id().0).collect()
}

// ---- the Königsberg equivalents (spike 7's tx_koenigsberg.rs) ----------

/// Stage the full Königsberg change set: four nodes, then the same edge
/// order `koenigsberg()` uses.
fn stage_koenigsberg(tx: &mut Tx<'_>) {
    let a = tx.add_node();
    let b = tx.add_node();
    let c = tx.add_node();
    let d = tx.add_node();
    for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
        tx.add_edge(x, y);
        tx.add_edge(y, x);
    }
}

#[test]
fn tx_store_shape() {
    let direct = koenigsberg();
    let tx_built = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    assert_eq!(tx_built.node_count(), 4);
    assert_eq!(tx_built.edge_count(), 14);
    let g = tx_built.graph();
    let ids: Vec<u32> = g.nodes().map(|n| n.id().0).collect();
    assert_eq!(ids, [A, B, C, D]);
    assert_eq!(signature(&tx_built), signature(&direct));
}

#[test]
fn tx_adjacency_degrees() {
    let s = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    let g = s.graph();
    let nodes: Vec<_> = g.nodes().collect();
    let (a, b, c, d) = (nodes[0], nodes[1], nodes[2], nodes[3]);
    assert_eq!(a.out_edges().count(), 5);
    assert_eq!(a.in_edges().count(), 5);
    assert_eq!(a.edges().count(), 10);
    for n in [b, c, d] {
        assert_eq!(n.out_edges().count(), 3);
        assert_eq!(n.in_edges().count(), 3);
        assert_eq!(n.edges().count(), 6);
    }
}

#[test]
fn tx_edge_endpoints() {
    let s = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    let a = s.graph().nodes().next().unwrap();
    // Reverse insertion order: A's first out-edge is the last inserted
    // from A: A -> D.
    let e = a.out_edges().next().unwrap();
    assert_eq!(e.source().id(), NodeId(A));
    assert_eq!(e.target().id(), NodeId(D));
}

#[test]
fn tx_bfs_order() {
    let s = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    let a = s.graph().nodes().next().unwrap();
    assert_eq!(node_ids(bfs(a)), [A, D, C, B]);
}

#[test]
fn tx_dfs_order() {
    let s = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    let a = s.graph().nodes().next().unwrap();
    assert_eq!(node_ids(dfs(a)), [A, B, D, C]);
}

#[test]
fn tx_bfs_visits_every_node_once_from_any_start() {
    let s = GraphStore::new().edit().tap_flush(stage_koenigsberg).expect("flush").store;
    let g = s.graph();
    for start in g.nodes() {
        let mut ids = node_ids(bfs(start));
        ids.sort_unstable();
        assert_eq!(ids, [A, B, C, D]);
    }
}

#[test]
fn tx_self_loop_appears_in_both_directions() {
    // Self-loop via transaction: one node, one staged self-edge.
    let s = GraphStore::new()
        .edit()
        .tap_flush(|tx| {
            let x = tx.add_node();
            tx.add_edge(x, x);
        })
        .expect("flush")
        .store;
    let n = s.graph().nodes().next().unwrap();
    assert_eq!(n.out_edges().count(), 1);
    assert_eq!(n.in_edges().count(), 1);
    assert_eq!(n.edges().count(), 2);
    s.validate().expect("validate");
}

#[test]
fn tx_via_edit_from_empty_matches_direct() {
    let empty = GraphStore::new();
    let mut tx = empty.edit();
    stage_koenigsberg(&mut tx);
    let out = tx.flush().expect("flush");
    assert_eq!(signature(&out.store), signature(&koenigsberg()));
    assert_eq!(out.report.node_map, Vec::<Option<u32>>::new());
    assert_eq!(out.report.edge_map, Vec::<Option<u32>>::new());
}

#[test]
fn empty_change_set_flush_equals_clone() {
    // Flush with nothing staged: observably a clone; maps are identities.
    let s = koenigsberg();
    let out = s.edit().flush().expect("flush");
    assert_eq!(signature(&out.store), signature(&s));
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(out.report.edge_map, (0..14).map(Some).collect::<Vec<_>>());
}

#[test]
fn edit_leaves_old_store_usable_old_handles_valid_after_flush() {
    // Build the A-B half directly, then stage the rest in one pass. The
    // pass discipline: handles live during staging (reads happen), the old
    // store and its handles survive the flush untouched.
    let mut base = GraphStore::new();
    let a = base.add_node();
    let b = base.add_node();
    base.add_edge(a, b);
    base.add_edge(b, a);
    base.add_edge(a, b);
    base.add_edge(b, a);

    // Handles created before the transaction; they borrow the old store
    // sharedly alongside the transaction.
    let g = base.graph();
    let nodes: Vec<_> = g.nodes().collect();
    let (ha, hb) = (nodes[0], nodes[1]);
    assert_eq!(ha.out_edges().count(), 2);

    // Stage the C/D half, addressing kept nodes through handle→ref
    // conversion (which is where the store-identity check fires).
    let mut tx = base.edit();
    let c = tx.add_node();
    let d = tx.add_node();
    tx.add_edge(ha, c);
    tx.add_edge(c, ha);
    tx.add_edge(ha, c);
    tx.add_edge(c, ha);
    tx.add_edge(ha, d);
    tx.add_edge(d, ha);
    tx.add_edge(hb, d);
    tx.add_edge(d, hb);
    tx.add_edge(c, d);
    tx.add_edge(d, c);
    let out = tx.flush().expect("flush");

    // Old handles still valid, old store observably untouched.
    assert_eq!(ha.id(), NodeId(A));
    assert_eq!(ha.out_edges().count(), 2);
    assert_eq!(base.node_count(), 2);
    assert_eq!(base.edge_count(), 4);

    // The new store is observably the Königsberg store.
    assert_eq!(signature(&out.store), signature(&koenigsberg()));
    assert_eq!(out.report.node_map, vec![Some(0), Some(1)]);
    assert_eq!(out.report.edge_map, (0..4).map(Some).collect::<Vec<_>>());

    // And BFS on the new store agrees with the direct store.
    let a_new = out.store.graph().nodes().next().unwrap();
    assert_eq!(node_ids(bfs(a_new)), [A, D, C, B]);
}

#[test]
fn new_inserts_push_front_onto_remapped_heads() {
    // Order contract: kept elements keep iteration order; genuinely new
    // inserts push-front onto remapped heads, so they iterate first.
    let mut base = GraphStore::new();
    let a = base.add_node();
    let b = base.add_node();
    let e_ab = base.add_edge(a, b); // A's out-list: [e_ab]

    let out = base
        .edit()
        .tap_flush(|tx| {
            let c = tx.add_node();
            tx.add_edge(a, c); // new head
            tx.add_edge(a, b); // another new head
        })
        .expect("flush");

    // Direct equivalent: same store extended in the same order.
    let mut direct = GraphStore::new();
    let a2 = direct.add_node();
    let b2 = direct.add_node();
    direct.add_edge(a2, b2);
    let c2 = direct.add_node();
    direct.add_edge(a2, c2);
    direct.add_edge(a2, b2);

    assert_eq!(signature(&out.store), signature(&direct));
    // Report maps are old-id-indexed with `None` holes; the old store had
    // one edge, kept and remapped to new id 0 (kept records compact first;
    // new records append in replay order — plan order contracts).
    assert_eq!(out.report.node_map, vec![Some(0), Some(1)]);
    assert_eq!(out.report.edge_map, vec![Some(0)]);
    assert_eq!(e_ab.0, 0); // old id retained in the old store

    // The push-front contract, pinned explicitly: the kept edge (new id 0)
    // is the TAIL of A's out-list; the two new inserts (ids 1, 2) replayed
    // in submission order each pushed front, so they iterate first.
    let a_new = out.store.graph().nodes().next().unwrap();
    let out_ids: Vec<u32> = a_new.out_edges().map(|e| e.id().0).collect();
    assert_eq!(out_ids, vec![2, 1, 0]);
}

// ---- removal, compaction, danglers, the report (tx_removal.rs) ---------

#[test]
fn remove_edge_compacts_and_remaps() {
    let base = triangle();
    let out = base
        .edit()
        .tap_flush(|tx| tx.remove_edge(EdgeId(1))) // A->C
        .expect("flush");

    assert_eq!(out.store.node_count(), 3);
    assert_eq!(out.store.edge_count(), 2);
    assert_eq!(out.report.edge_map, vec![Some(0), None, Some(1)]); // e0 kept, e1 removed, e2 shifted
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2)]);

    // Old store untouched.
    assert_eq!(base.edge_count(), 3);

    // Direct equivalent: the same store built without the removed edge.
    let mut direct = GraphStore::new();
    let a = direct.add_node();
    let b = direct.add_node();
    let c = direct.add_node();
    direct.add_edge(a, b);
    direct.add_edge(b, c);
    assert_eq!(signature(&out.store), signature(&direct));

    out.store.validate().expect("validate");
}

#[test]
fn remove_edge_in_middle_of_chain_preserves_kept_order() {
    // Naive per-field link remapping truncates the list when a *middle*
    // link's target is removed; the rebuild-by-walk must preserve the kept
    // relative order.
    let mut base = GraphStore::new();
    let a = base.add_node();
    let b = base.add_node();
    let c = base.add_node();
    let d = base.add_node();
    base.add_edge(a, b); // e0
    base.add_edge(a, c); // e1  <- removed
    base.add_edge(a, d); // e2
    // A's out-list: [e2, e1, e0].

    let out = base
        .edit()
        .tap_flush(|tx| tx.remove_edge(EdgeId(1)))
        .expect("flush");

    let g = out.store.graph();
    let a_new = g.nodes().next().unwrap();
    let outs: Vec<_> = a_new
        .out_edges()
        .map(|e| (e.source().id().0, e.target().id().0))
        .collect();
    assert_eq!(outs, vec![(0, 3), (0, 1)]); // e2' then e0': kept order kept
    out.store.validate().expect("validate");
}

#[test]
fn remove_node_requires_explicit_dependent_removals() {
    // Minimal cascade policy: removing node C without removing its incident
    // edges is a dangler violation naming old ids.
    let base = triangle();
    let err = base
        .edit()
        .tap_flush(|tx| tx.remove_node(NodeId(2)))
        .expect_err("danglers must be rejected");
    // e1 (A->C) and e2 (B->C) both dangle; each violation points at the
    // removal op (op 0), in old ids.
    assert_eq!(err.violations.len(), 2);
    let mut records: Vec<_> = err
        .violations
        .iter()
        .map(|v| match v {
            Violation::DanglingEndpoint { op, record, endpoint_arena, endpoint } => {
                assert_eq!(*op, 0);
                assert_eq!(*endpoint, 2u32);
                assert_eq!(*endpoint_arena, "node");
                *record
            }
            other => panic!("unexpected violation: {other:?}"),
        })
        .collect();
    records.sort_by_key(|r| match r {
        RecordDesc::Kept { id, .. } => *id,
        RecordDesc::Staged(i) => *i,
    });
    assert_eq!(
        records,
        vec![
            RecordDesc::Kept { arena: "edge", id: 1 },
            RecordDesc::Kept { arena: "edge", id: 2 },
        ]
    );
    // The store is untouched after a failed flush.
    assert_eq!(base.node_count(), 3);
    assert_eq!(base.edge_count(), 3);
}

#[test]
fn explicit_removal_of_node_and_dependents_flushes() {
    let base = triangle();
    let out = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_node(NodeId(2));
            tx.remove_edge(EdgeId(1)); // A->C
            tx.remove_edge(EdgeId(2)); // B->C
        })
        .expect("flush");
    assert_eq!(out.store.node_count(), 2);
    assert_eq!(out.store.edge_count(), 1);
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), None]);
    assert_eq!(out.report.edge_map, vec![Some(0), None, None]);

    let mut direct = GraphStore::new();
    let a = direct.add_node();
    let b = direct.add_node();
    direct.add_edge(a, b);
    assert_eq!(signature(&out.store), signature(&direct));
    out.store.validate().expect("validate");
}

#[test]
fn staged_edge_to_removed_node_is_a_dangler() {
    // Dangler detection covers staged constructive ops too, attributed to
    // the staging op, named in old ids.
    let base = triangle();
    let err = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_node(NodeId(2));
            tx.remove_edge(EdgeId(1));
            tx.remove_edge(EdgeId(2));
            let x = tx.add_node();
            let _ = x;
            tx.add_edge(NodeId(0), NodeId(2)); // staged edge to removed C
        })
        .expect_err("staged dangler must be rejected");
    assert_eq!(err.violations.len(), 1);
    // Op indices count EVERY staged op: remove_node=0, remove_edge=1,
    // remove_edge=2, add_node=3, so the staged AddEdge is op 4.
    assert_eq!(
        err.violations[0],
        Violation::DanglingEndpoint {
            op: 4, // the AddEdge op
            record: RecordDesc::Staged(4),
            endpoint_arena: "node",
            endpoint: 2,
        }
    );
}

#[test]
fn unknown_old_id_is_a_flush_error() {
    let base = triangle();
    let err = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_node(NodeId(99));
        })
        .expect_err("unknown old id must be rejected");
    assert_eq!(
        err.violations,
        vec![Violation::UnknownOldId {
            op: 0,
            arena: "node",
            id: 99,
        }]
    );
}

#[test]
fn double_removal_is_a_flush_error() {
    let base = triangle();
    let err = base
        .edit()
        .tap_flush(|tx| {
            tx.remove_edge(EdgeId(0));
            tx.remove_edge(EdgeId(0));
        })
        .expect_err("double removal must be rejected");
    assert_eq!(
        err.violations,
        vec![Violation::DoubleRemoval {
            op: 1,
            arena: "edge",
            id: 0,
        }]
    );
}

#[test]
fn removal_via_old_handle_and_report_round_trip() {
    // Removal staged through a handle→ref conversion; the report's edge map
    // lets a raw-id holder find the surviving edge's new id.
    let base = triangle();
    let g = base.graph();
    let nodes: Vec<_> = g.nodes().collect();
    let b_handle = nodes[1];

    let mut tx = base.edit();
    let bc = tx.add_edge(b_handle, NodeId(2)); // B->C, a second one
    let _ = bc;
    tx.remove_edge(EdgeId(0)); // old A->B
    let out = tx.flush().expect("flush");

    assert_eq!(out.store.edge_count(), 3);
    // e0 removed; old e1 (A->C), e2 (B->C) -> new 0,1; the staged B->C is a
    // genuinely new record (new id 2) and has no old-id entry: the map is
    // old-id-indexed, one slot per OLD arena record.
    assert_eq!(out.report.edge_map, vec![None, Some(0), Some(1)]);
    assert_eq!(out.store.validate(), Ok(()));
}

#[test]
fn raw_id_and_handle_endpoints_mix_freely() {
    // "Anywhere an Existing ref is expected, an old-store handle or raw id
    // is accepted" — and Pending refs mix with both in one op.
    let base = triangle();
    let g = base.graph();
    let a_handle = g.nodes().next().unwrap();
    let out = base
        .edit()
        .tap_flush(|tx| {
            let x = tx.add_node();
            tx.add_edge(a_handle, x); // handle -> Pending
            tx.add_edge(x, NodeId(1)); // Pending -> raw id
        })
        .expect("flush");
    assert_eq!(out.store.node_count(), 4);
    assert_eq!(out.store.edge_count(), 5);
    out.store.validate().expect("validate");
}

#[test]
fn pending_resolution_by_submission_order() {
    // Pending(u32) names the minting op's index; refs minted earlier in the
    // same change set resolve during replay.
    let base = triangle();
    let mut tx = base.edit();
    let n0 = tx.add_node();
    let n1 = tx.add_node();
    let e = tx.add_edge(n0, n1);
    match e.kind {
        RefKind::Pending(i) => assert_eq!(i, 2), // third op
        _ => panic!("edge ref should be Pending"),
    }
    let out = tx.flush().expect("flush");
    assert_eq!(out.store.node_count(), 5);
    assert_eq!(out.store.edge_count(), 4); // 3 old + 1 staged
    out.store.validate().expect("validate");
}

// ---- the spike-7 bug, as a regression (/tmp/spike7-oob-repro) ----------

#[test]
fn out_of_range_existing_ref_is_a_flush_error_not_a_panic() {
    // An `Existing` ref naming an out-of-range id is *flagged* in flush
    // step 1; the flush must return that error — never index the
    // unvalidated id in step 2's staged-dangler scan (the spike-7 bug).
    let mut base = GraphStore::new();
    let a = base.add_node();
    let out = base
        .edit()
        .tap_flush(|tx| {
            tx.add_edge(NodeId(999), a); // out-of-range Existing endpoint
        });
    match out {
        Ok(_) => panic!("flushed an out-of-range endpoint (unexpected)"),
        Err(e) => {
            assert_eq!(
                e.violations,
                vec![Violation::UnknownOldId { op: 0, arena: "node", id: 999 }]
            );
            // Display names the violation without panicking.
            let msg = e.to_string();
            assert!(msg.contains("UnknownOldId"), "{msg}");
        }
    }
}

// ---- step 4: check() + the deterministic error surface ------------------

#[test]
fn check_green_then_flush_commits() {
    // The committed-mode contract's read half: a green check means the
    // subsequent flush of the SAME change set passes steps 1-2 (the
    // analysis is shared). The owning half — check on an `into_edit`
    // transaction — lands with step 5.
    let base = koenigsberg();
    let mut tx = base.edit();
    let x = tx.add_node();
    tx.add_edge(x, NodeId(0));
    tx.remove_edge(EdgeId(13));
    assert_eq!(tx.check(), Ok(()));
    let out = tx.flush().expect("green check -> flush commits");
    assert_eq!(out.store.edge_count(), 14); // 13 kept + 1 staged
    out.store.validate().expect("validate");
}

#[test]
fn check_reports_the_same_violations_as_flush() {
    // check is the flush's own analysis, not a re-implementation: the
    // violation lists agree exactly on the same change set.
    let base = koenigsberg();
    let render = |tx: Tx<'_>| -> FlushError {
        match tx.flush() {
            Ok(_) => panic!("this change set dangles"),
            Err(e) => e,
        }
    };
    let mut tx1 = base.edit();
    tx1.remove_node(NodeId(0)); // five incident edges dangle
    let flush_err = render(tx1);
    let mut tx2 = base.edit();
    tx2.remove_node(NodeId(0));
    let check_err = tx2.check().expect_err("danglers");
    assert_eq!(check_err, flush_err);
    assert_eq!(check_err.violations.len(), 10); // 5 out + 5 in
}

#[test]
fn error_display_is_deterministic() {
    // Six independently built identical stores, the same failing change
    // set -> byte-identical Display every time (probe 4's pattern). The
    // payload spans several violation kinds and op indices so any
    // discovery-order leak would surface.
    let render = || {
        let mut base = koenigsberg();
        let _d = base.add_node(); // shift nothing; keep ids stable
        let mut tx = base.edit();
        tx.remove_node(NodeId(0)); // ops 0: five out- and five in-danglers
        tx.remove_edge(EdgeId(13)); // op 1: cures one
        tx.remove_edge(EdgeId(0)); // op 2: cures another
        tx.add_edge(NodeId(1), NodeId(0)); // op 3: staged dangler at its op
        tx.remove_edge(EdgeId(42)); // op 4: unknown old id
        tx.flush().unwrap_err().to_string()
    };
    let a = render();
    for _ in 0..5 {
        assert_eq!(a, render());
    }
}

// ---- step 5: into_edit — the owned path + in-place reuse ----------------

trait TapStage {
    fn tap_stage(self, f: impl FnOnce(&mut Tx<'_>)) -> FlushOutcome;
}

impl TapStage for Tx<'static> {
    fn tap_stage(mut self, f: impl FnOnce(&mut Tx<'_>)) -> FlushOutcome {
        f(&mut self);
        self.flush().expect("flush")
    }
}

#[test]
fn into_edit_koenigsberg_matches_direct() {
    let out = GraphStore::new().into_edit().tap_stage(stage_koenigsberg);
    assert_eq!(out.store.node_count(), 4);
    assert_eq!(out.store.edge_count(), 14);
    assert_eq!(signature(&out.store), signature(&koenigsberg()));
    out.store.validate().expect("validate");
}

#[test]
fn into_edit_self_loop() {
    // The tap_stage shape: an empty store consumed, one self-edge staged.
    let out = GraphStore::new().into_edit().tap_stage(|tx| {
        let x = tx.add_node();
        tx.add_edge(x, x);
    });
    let s = &out.store;
    let n = s.graph().nodes().next().unwrap();
    assert_eq!(n.out_edges().count(), 1);
    assert_eq!(n.in_edges().count(), 1);
    s.validate().expect("validate");
}

#[test]
fn into_edit_empty_change_set_equals_clone() {
    let s = koenigsberg();
    let out = koenigsberg().into_edit().flush().expect("flush");
    assert_eq!(signature(&out.store), signature(&s));
    assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(out.report.edge_map, (0..14).map(Some).collect::<Vec<_>>());
}

#[test]
fn into_edit_in_place_removal_replay_agrees_with_borrowed() {
    // The owned path forward-compacts in place (write slot <= read slot,
    // all derived data precomputed). The strongest check available: the
    // SAME change set through edit() and into_edit() must produce
    // observably identical stores and reports — removals shift ids, so
    // the compaction is real work.
    // A dies with its ten incident edges (e0..e9); the B-D and C-D
    // bridges survive, so compaction shifts real ids.
    let stage = |tx: &mut Tx<'_>| {
        tx.remove_node(NodeId(0)); // op 0
        for e in 0u32..10 {
            tx.remove_edge(EdgeId(e)); // ops 1..=10
        }
        let x = tx.add_node(); // op 11
        tx.add_edge(x, NodeId(1)); // op 12: staged -> B
        tx.add_edge(NodeId(2), x); // op 13: C -> staged
    };
    let base = koenigsberg();
    let borrowed = base.edit().tap_flush(stage).expect("flush");
    let owned = koenigsberg().into_edit().tap_stage(|tx| stage(tx));

    assert_eq!(signature(&borrowed.store), signature(&owned.store));
    assert_eq!(borrowed.report.node_map, owned.report.node_map);
    assert_eq!(borrowed.report.edge_map, owned.report.edge_map);
    // Old-id-indexed: A gone; e0..e9 gone, e10..e13 shift to 0..=3.
    assert_eq!(owned.report.node_map, vec![None, Some(0), Some(1), Some(2)]);
    assert_eq!(
        owned.report.edge_map,
        vec![None, None, None, None, None, None, None, None, None, None, Some(0), Some(1), Some(2), Some(3)]
    );
    owned.store.validate().expect("validate");
    assert_eq!(owned.store.node_count(), 4); // B, C, D + the staged node
    assert_eq!(owned.store.edge_count(), 6); // 4 kept + 2 staged
}
