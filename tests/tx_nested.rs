//! M4 step 9 — nested stores in the transaction. `within(host)` opens an
//! owned staging scope (token + the host REF as path prefix — never a
//! resolved id, so a `Pending` host names its own submission position);
//! nested adds thread `&mut Tx`; nested removals name host ref + old
//! embedded-arena index. The flush resolves path segments top-down:
//! `Existing` through the host's compaction map (host ids SHIFT under
//! removal — the translation is load-bearing), `Pending` through the
//! replay table the root pass fills first (bounds-checked: a miss is an
//! internal bug, unreachable through the public API). Removed-host path
//! coherence is a flush error naming both the nested ops and the removal;
//! a removed host with nothing staged under it cascades silently.
//!
//! These are the generated equivalents of /tmp/m4-probe-nested-pending's
//! 10 tests (its REPORT.md pins the semantics), over a Hierarchy-shaped
//! store: `outer.Node <=> sub.Subgraph` embeds sub's arenas inside each
//! outer node record, keyed-free (SubPlain) so the step-8 gate's keyed
//! exclusions don't apply. The probe's builder-phase test halves that
//! assume live store removal have no generated equivalent (removal is
//! Tx-only); the path-id half is kept.

use metagraph::implement;
use metagraph::tx::RefKind;

mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        NestPlain {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph,
                sub:    m2_fixtures::shapes::SubPlain;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

implement! {
    comp::NestPlain as NestStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        inner.nodes:     arena,
        inner.out_edges: intrusive_list,
        inner.in_edges:  intrusive_list,
        sub.subnodes:    arena,
        sub.subedges:    arena,
    }
}

fn o(i: u32) -> OuterNodeId {
    OuterNodeId(i)
}
fn i(i: u32) -> InnerNodeId {
    InnerNodeId(i)
}

/// The observable structure equality assertions are made on: per outer
/// node, its subnodes' `main_node` values in embedded-arena order.
fn signature(s: &NestStore) -> Vec<Vec<u32>> {
    (0..s.outer_node_count())
        .map(|h| {
            let h = o(h as u32);
            (0..s.sub_node_count(h))
                .map(|idx| s.main_node(SubNodeId(h, idx as u32)).0)
                .collect()
        })
        .collect()
}

/// Old store: inner nodes i0..i6 plus ie0 (i0 -> i1); outer o0, o1, o2
/// with subnodes o0: [i0, i1, i2], o1: [i3, i4], o2: [i5, i6, i0] (runtime
/// multiplicity is unconstrained — i0 keys into two hosts' arenas), and
/// one subedge under o0. i7 is an unreferenced inner node (the dangler
/// tests' target).
fn old_store() -> NestStore {
    let mut s = NestStore::new();
    for _ in 0..8 {
        s.add_inner_node();
    }
    let ie0 = s.add_inner_edge(i(0), i(1));
    assert_eq!(ie0, InnerEdgeId(0));
    let o0 = s.add_outer_node();
    let o1 = s.add_outer_node();
    let o2 = s.add_outer_node();
    for m in [0, 1, 2] {
        s.add_sub_node(o0, i(m));
    }
    for m in [3, 4] {
        s.add_sub_node(o1, i(m));
    }
    for m in [5, 6, 0] {
        s.add_sub_node(o2, i(m));
    }
    s.add_sub_edge(o0, ie0);
    s.validate().expect("old store validates");
    s
}

/// The shared change set (the probe's `sample_flush`, generated surface):
/// a Pending host with two nested adds submitted before any other root
/// op; a second root add AFTER those nested submissions; a nested add
/// under Existing o1; removal of unrelated o0 (shifting o1/o2); removal
/// of one sub under kept o2.
fn sample_flush() -> (NestStore, FlushOutcome) {
    let store = old_store();
    let mut tx = store.edit();
    let p = tx.add_outer_node(); // op 0: mints the provisional host
    let n8 = tx.add_inner_node(); // op 1: the new subs' main node
    let scope = tx.within(p); // path prefix [Pending(0)] — the ref
    scope.add_sub_node(&mut tx, n8); // op 2
    scope.add_sub_node(&mut tx, n8); // op 3
    let _q = tx.add_outer_node(); // op 4: root op after nested submissions
    tx.within(o(1)).add_sub_node(&mut tx, i(3)); // op 5: Existing host, old id 1
    tx.remove_outer_node(o(0)); // op 6: shifts kept outer ids
    tx.remove_sub_node(o(2), 1); // op 7: compacts o2's embedded arena
    let out = tx.flush().expect("coherent change set");
    (store, out)
}

/// The expected store, built directly: outer arena = kept old order then
/// staged root submission order; each embedded arena = that host's kept
/// old order then its staged order. Inner: 8 kept + 1 staged.
fn expected_store() -> NestStore {
    let mut expect = NestStore::new();
    for _ in 0..9 {
        expect.add_inner_node();
    }
    expect.add_inner_edge(i(0), i(1));
    let e0 = expect.add_outer_node(); // old o1, kept first
    expect.add_sub_node(e0, i(3));
    expect.add_sub_node(e0, i(4));
    expect.add_sub_node(e0, i(3)); // staged under Existing o1, at its NEW id
    let e1 = expect.add_outer_node(); // old o2, sub idx 1 removed
    expect.add_sub_node(e1, i(5));
    expect.add_sub_node(e1, i(0));
    let e2 = expect.add_outer_node(); // Pending host from op 0
    expect.add_sub_node(e2, i(8));
    expect.add_sub_node(e2, i(8));
    let _e3 = expect.add_outer_node(); // op 4, no subs
    expect
}

#[test]
fn nested_over_pending_and_existing_matches_direct_construction() {
    let (old, out) = sample_flush();
    assert_eq!(signature(&out.store), signature(&expected_store()));
    assert_eq!(out.store.inner_node_count(), 9);
    assert_eq!(out.store.inner_edge_count(), 1);
    out.store.validate().expect("flushed store validates");
    // Borrowed flush: the old store is untouched and old ids still name
    // their old records.
    assert_eq!(
        signature(&old),
        vec![
            vec![0, 1, 2],
            vec![3, 4],
            vec![5, 6, 0],
        ]
    );
}

#[test]
fn ordering_kept_then_root_staged_then_per_host_staged() {
    let (_, out) = sample_flush();
    let report = &out.report;

    // Old o0 removed; old o1 -> 0; old o2 -> 1.
    assert_eq!(report.outer_node_map, vec![None, Some(0), Some(1)]);
    // Inner nodes all kept: identity map; the staged one appends at 8.
    assert_eq!(
        report.inner_node_map,
        (0..8).map(Some).collect::<Vec<_>>()
    );
    // Per-host sub maps: o1 both kept (staged sub appends, not in the
    // old-id-indexed map); o2's idx 1 removed, idx 2 compacts down.
    assert_eq!(report.sub_node_map[1], vec![Some(0), Some(1)]);
    assert_eq!(report.sub_node_map[2], vec![Some(0), None, Some(1)]);
    // Removed host: one None per old sub idx — uniform per-old-index
    // shape, nothing kept (its subedge's map rides the same cascade).
    assert_eq!(report.sub_node_map[0], vec![None, None, None]);
    assert_eq!(report.sub_edge_map[0], vec![None]);

    // Root adds keep submission order across the grouping: the op-0 host
    // lands at new id 2 (its subs prove it), op 4's at new id 3 — the
    // nested ops submitted between them did not displace it.
    assert_eq!(out.store.sub_node_count(o(2)), 2);
    assert_eq!(out.store.main_node(SubNodeId(o(2), 0)), i(8));
    assert_eq!(out.store.main_node(SubNodeId(o(2), 1)), i(8));
    assert_eq!(out.store.sub_node_count(o(3)), 0);
}

#[test]
fn existing_host_translates_through_compaction_map() {
    let (_, out) = sample_flush();

    // Old o1 shifted to new id 0 when o0 was removed; its staged sub must
    // land at the host's NEW position — resolution through the compaction
    // map, not raw id reuse.
    assert_eq!(out.report.outer_node_map[1], Some(0));
    let at_new0: Vec<u32> = (0..out.store.sub_node_count(o(0)))
        .map(|idx| out.store.main_node(SubNodeId(o(0), idx as u32)).0)
        .collect();
    assert_eq!(at_new0, vec![3, 4, 3]);
    // The record now at arena position 1 is old o2 — proof the staged sub
    // was translated, not placed by old id.
    let at_new1: Vec<u32> = (0..out.store.sub_node_count(o(1)))
        .map(|idx| out.store.main_node(SubNodeId(o(1), idx as u32)).0)
        .collect();
    assert_eq!(at_new1, vec![5, 0]);
}

#[test]
fn removed_host_with_staged_ops_names_ops_and_removal() {
    let store = old_store();
    let mut tx = store.edit();
    tx.within(o(1)).add_sub_node(&mut tx, i(0)); // op 0 — under o1
    tx.within(o(1)).add_sub_node(&mut tx, i(0)); // op 1 — under o1
    tx.remove_sub_node(o(1), 0); // op 2 — under o1 too: a remove of a record the change set deletes
    tx.within(o(0)).add_sub_node(&mut tx, i(0)); // op 3 — under kept o0, not named
    tx.remove_outer_node(o(1)); // op 4 — the removal
    let err = tx.flush().unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::RemovedPathHost {
            arena: "outer_node",
            host: 1,
            nested_ops: vec![0, 1, 2],
            removal_op: 4,
        }]
    );
}

#[test]
fn removed_host_without_staged_ops_cascades_silently() {
    let store = old_store();
    let mut tx = store.edit();
    tx.remove_outer_node(o(1)); // live subs [i3, i4], nothing staged under o1
    let out = tx.flush().expect("cascade exception: nothing staged under o1");
    assert_eq!(signature(&out.store), vec![vec![0, 1, 2], vec![5, 6, 0]]);
    assert_eq!(
        out.report.outer_node_map,
        vec![Some(0), None, Some(1)]
    );
    assert_eq!(out.report.sub_node_map[1], vec![None, None]); // removed host: all-None
    out.store.validate().expect("flushed store validates");
}

#[test]
#[should_panic(expected = "stamped with a different transaction token")]
fn foreign_token_ref_rejected_at_within() {
    let store = old_store();
    let stale = {
        let mut tx = store.edit();
        tx.add_outer_node() // minted under a token that dies with this tx
    };
    let tx2 = store.edit();
    let _scope = tx2.within(stale); // must panic here
}

#[test]
#[should_panic(expected = "replay table miss")]
fn replay_table_miss_panics() {
    // Unreachable through the public API: Pending outer refs are minted
    // only by add_outer_node, at their own op index, and the root pass
    // fills the whole table before the nested pass runs. This test forges
    // a stamped Pending ref naming an op index no add occupies — possible
    // because the implement! expansion and this test share a crate (the
    // ref's fields are public runtime data; the token is pub(crate)).
    let store = old_store();
    let mut tx = store.edit();
    let forged = OuterNodeRef {
        token: tx.token,
        kind: RefKind::Pending(999),
    };
    // Survives staging and steps 1-2 (a Pending host cannot be removed,
    // so no coherence check applies); panics in the nested replay pass.
    tx.within(forged).add_sub_node(&mut tx, i(0));
    let _ = tx.flush();
}

#[test]
fn unknown_old_id_reports_the_staging_op() {
    let store = old_store();
    // A nested add under an out-of-range Existing host: step 1 validates
    // path segments by op index, like any other Existing ref.
    let err = store
        .edit()
        .tap(|tx| tx.within(o(7)).add_sub_node(tx, i(0)))
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::UnknownOldId {
            op: 0,
            arena: "outer_node",
            id: 7
        }]
    );
    // A nested removal past the host's old embedded arena.
    let err = store
        .edit()
        .tap(|tx| tx.remove_sub_node(o(0), 99))
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::UnknownNestedIdx {
            op: 0,
            arena: "sub_node",
            host: 0,
            idx: 99
        }]
    );
}

#[test]
fn double_removal_reports_second_op() {
    let store = old_store();
    let err = store
        .edit()
        .tap(|tx| {
            tx.remove_outer_node(o(0));
            tx.remove_outer_node(o(0));
        })
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DoubleRemoval {
            op: 1,
            arena: "outer_node",
            id: 0
        }]
    );
    let err = store
        .edit()
        .tap(|tx| {
            tx.remove_sub_node(o(0), 0);
            tx.remove_sub_node(o(0), 0);
        })
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DoubleNestedRemoval {
            op: 1,
            arena: "sub_node",
            host: 0,
            idx: 0
        }]
    );
}

#[test]
fn construction_ids_are_path_ids() {
    // The builder-phase half of the probe's construction test: sub ids
    // are path ids, and each host's embedded arena numbers from zero.
    // (The probe's live-removal half has no generated equivalent —
    // removal is Tx-only; flush-side compaction is covered above.)
    let mut s = NestStore::new();
    let a = s.add_outer_node();
    let b = s.add_outer_node();
    let s0 = s.add_sub_node(a, i(0));
    let s1 = s.add_sub_node(a, i(0));
    let s2 = s.add_sub_node(b, i(0));
    assert_eq!(s0, SubNodeId(o(0), 0));
    assert_eq!(s1, SubNodeId(o(0), 1));
    assert_eq!(s2, SubNodeId(o(1), 0));
}

#[test]
fn kept_sub_record_danglers_name_host_and_idx() {
    let store = old_store();
    // i6 is named by o2's kept sub idx 1 (and by nothing else).
    let err = store
        .edit()
        .tap(|tx| tx.remove_inner_node(i(6)))
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DanglingEndpoint {
            op: 0,
            record: RecordDesc::KeptNested {
                arena: "sub_node",
                host: 2,
                idx: 1,
            },
            endpoint_arena: "inner_node",
            endpoint: 6,
        }]
    );
}

#[test]
fn staged_nested_add_naming_removed_target_dangles() {
    let store = old_store();
    // i7 is unreferenced: removing it dangles no kept record, so the only
    // violation is the staged add that names it, attributed to that op.
    let err = store
        .edit()
        .tap(|tx| {
            tx.remove_inner_node(i(7)); // op 0
            tx.within(o(0)).add_sub_node(tx, i(7)); // op 1
        })
        .flush()
        .unwrap_err();
    assert_eq!(
        err.violations,
        vec![Violation::DanglingEndpoint {
            op: 1,
            record: RecordDesc::Staged(1),
            endpoint_arena: "inner_node",
            endpoint: 7,
        }]
    );
}

#[test]
fn subedge_ops_ride_the_same_scope() {
    // The second embedded entity shares the host's scope: staged subedge
    // add under a Pending host, nested subedge removal, per-host maps.
    let store = old_store();
    let mut tx = store.edit();
    tx.remove_sub_edge(o(0), 0); // op 0
    let p = tx.add_outer_node(); // op 1
    tx.within(p).add_sub_edge(&mut tx, InnerEdgeId(0)); // op 2
    let out = tx.flush().expect("coherent");
    assert_eq!(out.report.sub_edge_map[0], vec![None]);
    // No outer was removed: the kept hosts land at 0..2 and the Pending
    // host appends at new id 3 — the staged subedge lands there.
    assert_eq!(out.store.sub_edge_count(o(3)), 1);
    assert_eq!(
        out.store.main_edge(SubEdgeId(o(3), 0)),
        InnerEdgeId(0)
    );
    out.store.validate().expect("validates");
}

#[test]
fn check_and_flush_agree_on_nested_violations() {
    let store = old_store();
    let stage = |tx: &mut Tx<'_>| {
        tx.within(o(1)).add_sub_node(tx, i(0));
        tx.remove_outer_node(o(1));
    };
    let mut tx = store.edit();
    stage(&mut tx);
    let checked = tx.check().unwrap_err();
    let mut tx = store.edit();
    stage(&mut tx);
    let flushed = tx.flush().unwrap_err();
    assert_eq!(checked, flushed);
}

#[test]
fn owned_flush_compacts_nested_in_place() {
    // into_edit: the owned path forward-compacts the outer arena in place
    // and rebuilds each kept host's embedded arenas as fresh Vecs — same
    // signature as the borrowed flush.
    let out = {
        let store = old_store();
        let mut tx = store.into_edit();
        let p = tx.add_outer_node();
        let n8 = tx.add_inner_node();
        let scope = tx.within(p);
        scope.add_sub_node(&mut tx, n8);
        scope.add_sub_node(&mut tx, n8);
        let _q = tx.add_outer_node();
        tx.within(o(1)).add_sub_node(&mut tx, i(3));
        tx.remove_outer_node(o(0));
        tx.remove_sub_node(o(2), 1);
        tx.flush().expect("coherent change set")
    };
    assert_eq!(signature(&out.store), signature(&expected_store()));
    out.store.validate().expect("flushed store validates");
}

/// Staging-helper: run `f` on the transaction, return it for chaining.
trait Tap {
    fn tap(self, f: impl FnOnce(&mut Tx<'_>)) -> Self;
}

impl Tap for Tx<'_> {
    fn tap(mut self, f: impl FnOnce(&mut Tx<'_>)) -> Self {
        f(&mut self);
        self
    }
}
