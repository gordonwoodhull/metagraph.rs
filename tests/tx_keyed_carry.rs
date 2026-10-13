//! M4 step 11 — removal + keyed carry: the per-record keyed map (the
//! keyed-carry probe's Hierarchy-lite shape) ungated, with the
//! survivors-only cascade, the composed remap at carry AND staged-insert
//! replay, and the staged-insert coherence checks. These are the
//! generated equivalents of /tmp/m4-probe-keyed-carry's 15 tests (its
//! REPORT.md pins the semantics), recast over the real HierarchyStore
//! (a composed store whose `sub.subnode` is a per-record keyed map:
//! owned by each OuterNode record, keyed by a root-level InnerNode,
//! targeting an embedded SubNode — a raw path id, since no ref can name
//! an embedded record).
//!
//! The probe's tag/note vocabulary maps onto this store as: Tag →
//! InnerNode (the key entity), Outer (host) → OuterNode, Sub (entry
//! target) → SubNode, and the probe's note → a kept SubNode's
//! `main_node` field (a field-one dangler, discovered by the kept-record
//! pass and attributed to the removal op — the probe's DanglingNoteTag
//! analog). The store's second keyed arrow (`sub.subedge`, keyed by
//! InnerEdge) emits the same machinery but is not exercised here.

use metagraph::implement;

mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Hierarchy {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph,
                sub:    m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

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

fn i(i: u32) -> InnerNodeId {
    InnerNodeId(i)
}
fn o(i: u32) -> OuterNodeId {
    OuterNodeId(i)
}
fn s(host: u32, idx: u32) -> SubNodeId {
    SubNodeId(OuterNodeId(host), idx)
}

/// The probe's `sample_store`: inner nodes i0..i2, outers o0/o1.
/// o0's map: i0 -> s00, i2 -> s01; o1's map: i2 -> s10. s11 is a spare
/// target for staged inserts. s10's `main_node` is i2 — the probe's
/// note-analog: removing i2 dangles it (a kept-record field dangler,
/// attributed to the removal op).
fn sample_store() -> HierarchyStore {
    let mut st = HierarchyStore::new();
    for _ in 0..3 {
        st.add_inner_node();
    }
    st.add_outer_node(); // o0
    let s00 = st.add_sub_node(o(0), i(0));
    st.insert_subnode(o(0), i(0), s00);
    let s01 = st.add_sub_node(o(0), i(1));
    st.insert_subnode(o(0), i(2), s01);
    st.add_outer_node(); // o1
    let s10 = st.add_sub_node(o(1), i(2));
    st.insert_subnode(o(1), i(2), s10);
    st.add_sub_node(o(1), i(1)); // s11: spare target
    st.validate().expect("old store validates");
    st
}

/// Assertion 1 (generated equivalent). Happy path: removal of a keyed
/// inner node with every reference covered, removal of an outer WITH its
/// map (the cascade), a staged insert under the kept host with a Pending
/// key. The kept host's id shifts under the outer removal; the carried
/// map's keys and path-id values must be remapped — signature equality
/// against direct construction, the composed remap read off the report.
#[test]
fn happy_path_matches_direct_construction_with_remapped_carry() {
    let st = sample_store();

    let mut tx = st.edit();
    tx.remove_inner_node(i(2)); // op 0: keyed in both hosts' maps…
    tx.remove_sub_node(o(1), 0); // op 1: …this clears o1's entry…
    tx.remove_outer_node(o(0)); // op 2: …and o0 dies with its map
    let i3 = tx.add_inner_node(); // op 3
    tx.insert_subnode(o(1), i3, s(1, 1)); // op 4: staged insert under kept host
    let out = tx.flush().expect("coherent change set");

    // Direct construction of the expected store.
    let mut expect = HierarchyStore::new();
    expect.add_inner_node(); // old i0 -> InnerNodeId(0)
    expect.add_inner_node(); // old i1 -> InnerNodeId(1)
    let e_i3 = expect.add_inner_node(); // staged i3 -> InnerNodeId(2)
    let eh = expect.add_outer_node(); // old o1, shifted 1 -> 0
    let es = expect.add_sub_node(eh, i(1)); // old s11, idx 1 -> 0
    expect.insert_subnode(eh, e_i3, es);
    expect.validate().expect("expected store validates");

    assert_eq!(out.store.inner_node_count(), 3);
    assert_eq!(out.store.outer_node_count(), 1);
    assert_eq!(out.store.sub_node_count(eh), 1);
    assert_eq!(out.store.main_node(es), i(1));
    assert_eq!(
        out.store.subnode_by_key(eh, e_i3),
        Some(s(0, 0)),
        "staged insert replayed with the composed remap"
    );
    // Old keys are gone with old o0's map; o1's carried map held only the
    // i2 entry, cleared by op 1. (i(2) would be wrong here: the staged
    // i3 lands at InnerNodeId(2), exactly e_i3 above.)
    assert_eq!(out.store.subnode_by_key(eh, i(0)), None);
    assert_eq!(out.store.subnode_by_key(eh, i(1)), None);
    out.store.validate().expect("validate");

    // The composed remap, explicitly: kept host shifted, carried keys and
    // path-id values translated (nothing survives of old o0's map).
    assert_eq!(out.report.outer_node_map, vec![None, Some(0)]);
    assert_eq!(out.report.inner_node_map, vec![Some(0), Some(1), None]);
    assert_eq!(out.report.sub_node_map[1], vec![None, Some(0)]);
    assert_eq!(out.report.sub_node_map[0], vec![None, None]);
}

/// Assertion 2 (generated equivalent). Removing a key with live entries
/// in KEPT hosts is a FlushError: entries named sorted by (owner, key),
/// attributed to the removal op; the kept sub whose `main_node` is the
/// key lists as a second violation, same op (the probe's DanglingNoteTag
/// analog — discovered by the kept-record pass, which runs first).
#[test]
fn key_removal_with_live_entries_names_them_sorted_and_attributed() {
    let st = sample_store();

    let err = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_inner_node(i(2)); // op 0
        })
        .expect_err("live entries");

    assert_eq!(
        err.violations,
        vec![
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::KeptNested { arena: "sub_node", host: 1, idx: 0 },
                endpoint_arena: "inner_node",
                endpoint: 2,
            },
            Violation::KeyRemovedWithLiveEntries {
                op: 0,
                arena: "inner_node",
                key: 2,
                entries: vec![(0, 2), (1, 2)],
                staged_inserts: vec![],
            },
        ]
    );
}

/// Assertion 3 (generated equivalent), pure case: a key whose ONLY
/// entries sit under a host removed in the same change set is
/// cascade-covered — the map dies with its host, so nothing the key
/// names survives, and (survivors-only liveness) the removal is legal.
/// Works in either submission order.
#[test]
fn doomed_host_entries_only_are_cascade_covered() {
    for remove_outer_first in [false, true] {
        let mut st = HierarchyStore::new();
        st.add_inner_node(); // i0, keyed only under o0 (removed)
        st.add_inner_node(); // i1, the subs' main_node (stays live)
        st.add_outer_node(); // o0
        let s0 = st.add_sub_node(o(0), i(1));
        st.insert_subnode(o(0), i(0), s0);
        st.add_outer_node(); // o1
        let s1 = st.add_sub_node(o(1), i(1));
        st.insert_subnode(o(1), i(1), s1);
        st.validate().expect("old store validates");

        let mut tx = st.edit();
        if remove_outer_first {
            tx.remove_outer_node(o(0));
            tx.remove_inner_node(i(0));
        } else {
            tx.remove_inner_node(i(0));
            tx.remove_outer_node(o(0));
        }
        let out = tx
            .flush()
            .expect("key whose only entries die with a doomed host is cascade-covered");
        assert_eq!(out.store.outer_node_count(), 1);
        assert_eq!(out.store.inner_node_count(), 1);
        assert_eq!(out.store.sub_node_count(o(0)), 1);
        // Old i1 is the only survivor, so it compacts to InnerNodeId(0)
        // — the carried entry's key remaps with it.
        assert_eq!(out.store.subnode_by_key(o(0), i(0)), Some(s(0, 0)));
        assert_eq!(out.store.main_node(s(0, 0)), i(0));
        out.store.validate().expect("validate");
    }
}

/// Corner, survivors-only at entry level: a kept host's entry keyed by
/// i2 is cleared by a staged sub removal of its target in the same
/// change set — but the OTHER host's i2 entry survives, so removing i2
/// still errors, naming only the surviving entry.
#[test]
fn staged_sub_removal_clears_only_its_own_hosts_entry() {
    let st = sample_store();
    let err = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_sub_node(o(1), 0); // op 0: clears o1's i2 entry
            tx.remove_inner_node(i(2)); // op 1: o0's i2 entry survives -> still live
        })
        .expect_err("surviving entry");
    assert_eq!(
        err.violations,
        vec![Violation::KeyRemovedWithLiveEntries {
            op: 1,
            arena: "inner_node",
            key: 2,
            entries: vec![(0, 2)],
            staged_inserts: vec![],
        }]
    );
}

/// Companion: when the other i2 entry dies too (its host doomed in the
/// same change set) and the referencing sub is removed, the key removal
/// is fully covered.
#[test]
fn doomed_host_plus_sub_removal_fully_covers_key_removal() {
    let st = sample_store();
    let out = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_sub_node(o(1), 0); // clears o1's i2 entry (and the sub)
            tx.remove_inner_node(i(2));
            tx.remove_outer_node(o(0)); // o0's map dies with it
        })
        .expect("no surviving reference to i2 anywhere");
    assert_eq!(out.store.inner_node_count(), 2);
    assert_eq!(out.store.subnode_by_key(o(0), i(2)), None);
    out.store.validate().expect("validate");
}

/// Corner: the change set's own staged insert keyed by the removed key
/// counts as a live reference — attributed to the removal op, the
/// insert's op index listed alongside the surviving old entries, and the
/// kept sub referencing the key (its `main_node`) listing as a second
/// violation on the same op. (The insert targets o1's spare sub; o1's
/// map has no i0 key, so staging passes and the violation surfaces at
/// flush.)
#[test]
fn staged_insert_keyed_by_removed_key_is_a_live_reference() {
    let st = sample_store();
    let err = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_inner_node(i(0)); // op 0: keyed in o0's map, s00's main_node
            tx.insert_subnode(o(1), i(0), s(1, 1)); // op 1: names i0 from a new place
        })
        .expect_err("live staged reference");
    assert_eq!(
        err.violations,
        vec![
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::KeptNested { arena: "sub_node", host: 0, idx: 0 },
                endpoint_arena: "inner_node",
                endpoint: 0,
            },
            Violation::KeyRemovedWithLiveEntries {
                op: 0,
                arena: "inner_node",
                key: 0,
                entries: vec![(0, 0)],
                staged_inserts: vec![1],
            },
        ]
    );
}

/// Corner: a staged insert targeting a sub the same change set removes
/// is a step-2 FlushError attributed to the insert op (the probe's
/// DanglingInsertTarget — a path-id endpoint names (host, idx)).
#[test]
fn staged_insert_targeting_removed_sub_is_a_dangler() {
    let st = sample_store();
    let err = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_sub_node(o(1), 0); // op 0
            tx.insert_subnode(o(1), i(1), s(1, 0)); // op 1: target dies
        })
        .expect_err("doomed target");
    assert_eq!(
        err.violations,
        vec![Violation::DanglingInsertTarget {
            op: 1,
            target_host: 1,
            target_idx: 0,
        }]
    );
}

/// Corner, forced by the shape of the op set: a keyed insert's target is
/// a raw path id (Existing-only), so an entry staged under a PENDING
/// host can never satisfy value-host agreement (the target's host is an
/// old id; remap is identity-preserving; the fresh host gets a fresh
/// id). It is a step-2 FlushError, not a staging panic — symmetric with
/// RemovedPathHost.
#[test]
fn insert_under_pending_host_always_mismatches() {
    let st = sample_store();
    let err = st
        .edit()
        .tap_flush(|tx| {
            let _p = tx.add_outer_node(); // op 0
            tx.insert_subnode(_p, i(1), s(1, 0)); // op 1
        })
        .expect_err("agreement unreachable");
    assert_eq!(
        err.violations,
        vec![Violation::InsertHostTargetMismatch {
            op: 1,
            entry_host: HostName::Pending(0),
            target_host: 1,
            target_idx: 0,
        }]
    );
}

/// Assertion 4a (generated equivalent). Two-flush re-pointing: flush 1
/// removes the sub (its entries clear by target-as-membership); flush 2
/// inserts a new entry with the freed key — succeeds.
#[test]
fn two_flush_repointing_succeeds() {
    let mut st = HierarchyStore::new();
    st.add_inner_node(); // i0
    st.add_inner_node(); // i1
    st.add_outer_node(); // o0
    let s0 = st.add_sub_node(o(0), i(1));
    let _s1 = st.add_sub_node(o(0), i(0));
    st.insert_subnode(o(0), i(0), s0);

    let out1 = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_sub_node(o(0), 0);
        })
        .expect("sub removal clears its entries");
    assert_eq!(out1.store.subnode_by_key(o(0), i(0)), None);

    // New id space: old o0 -> new 0, the surviving sub -> new idx 0, i0 kept.
    let out2 = out1
        .store
        .edit()
        .tap_flush(|tx| {
            tx.insert_subnode(o(0), i(0), s(0, 0));
        })
        .expect("key was freed by flush 1");
    assert_eq!(out2.store.subnode_by_key(o(0), i(0)), Some(s(0, 0)));
    out2.store.validate().expect("validate");
}

/// Assertion 4b (generated equivalent). The single-flush equivalent
/// (remove sub + insert with the freed key in one change set) panics AT
/// STAGING: the key is live until flush. This is the pinned
/// duplicate-key check, and it is what makes re-pointing a two-flush
/// operation by design.
#[test]
#[should_panic(expected = "already live in host")]
fn single_flush_reinsert_panics_at_staging() {
    let mut st = HierarchyStore::new();
    st.add_inner_node(); // i0
    st.add_inner_node(); // i1
    st.add_outer_node(); // o0
    let s0 = st.add_sub_node(o(0), i(1));
    st.add_sub_node(o(0), i(0));
    st.insert_subnode(o(0), i(0), s0);

    let mut tx = st.edit();
    tx.remove_sub_node(o(0), 0);
    tx.insert_subnode(o(0), i(0), s(0, 1)); // key still live -> panic HERE
}

/// Staging duplicate-key check, staged-vs-staged arm: the same Pending
/// key minted once collides with itself in one host.
#[test]
#[should_panic(expected = "already claimed by a staged insert")]
fn duplicate_key_staged_vs_staged_panics() {
    let mut st = HierarchyStore::new();
    st.add_inner_node(); // i0
    st.add_outer_node(); // o0
    st.add_sub_node(o(0), i(0));

    let mut tx = st.edit();
    let k = tx.add_inner_node();
    tx.insert_subnode(o(0), k, s(0, 0));
    tx.insert_subnode(o(0), k, s(0, 0)); // same Pending key, same host
}

/// Assertion 5 (generated equivalent). Value-host agreement catches
/// damage below the transaction's checks: poke a store so an entry's
/// target names a different host — the flush's validate() reports it in
/// the path-id shape (names both hosts). Also covers the
/// value-in-arena arm (ghost target idx).
#[test]
fn validate_catches_host_agreement_damage() {
    let mut st = HierarchyStore::new();
    st.add_inner_node(); // i0
    st.add_inner_node(); // i1
    st.add_outer_node(); // o0
    let s0 = st.add_sub_node(o(0), i(1));
    st.insert_subnode(o(0), i(0), s0);
    st.add_outer_node(); // o1
    st.add_sub_node(o(1), i(0));
    st.validate().expect("clean store validates");

    // Corrupt below the transaction's checks. A flush would only launder
    // the damage — carry's embedded-target remap assumes the
    // host-agreement invariant and re-anchors the value — so validate the
    // store directly: the composed keyed validate is what names both
    // hosts in the path-id shape.
    st.outer_node_arena[0].subnode_index.insert(i(1), s(1, 0));
    let err = st.validate().expect_err("host agreement must fail");
    assert!(
        err.contains("OuterNodeId(1)") && err.contains("OuterNodeId(0)"),
        "validate said: {}",
        err
    );

    // Value-in-arena arm: ghost idx, right host.
    st.outer_node_arena[0]
        .subnode_index
        .insert(i(1), s(0, 9));
    let err2 = st.validate().expect_err("ghost idx must fail");
    assert!(
        err2.contains("SubNodeId(OuterNodeId(0), 9)") && err2.contains("not a live record"),
        "validate said: {}",
        err2
    );
}

/// Assertion 6 (generated equivalent). Deterministic error text: several
/// independently built identical stores (independent HashMap
/// RandomState), same failing change set -> byte-identical Display every
/// time. The payload is entries across many owners so any HashMap-order
/// leak would surface.
#[test]
fn error_display_is_deterministic() {
    let render = || {
        let mut st = HierarchyStore::new();
        let k = st.add_inner_node();
        for _ in 0..4 {
            st.add_outer_node();
            let sub = st.add_sub_node(o((st.outer_node_count() - 1) as u32), i(0));
            st.insert_subnode(o((st.outer_node_count() - 1) as u32), k, sub);
        }
        st.edit()
            .tap_flush(|tx| {
                tx.remove_inner_node(k);
            })
            .expect_err("live entries")
            .to_string()
    };
    let a = render();
    for _ in 0..5 {
        assert_eq!(a, render());
    }
}

/// Assertion 7 (generated equivalent). Removed host + staged inserts
/// (and nested adds, and nested removals) under it -> RemovedPathHost
/// naming nested op indices ascending + the removal op; a staged op
/// under the KEPT host is not named.
#[test]
fn removed_host_names_staged_ops_ascending_plus_removal() {
    let st = sample_store();
    let err = st
        .edit()
        .tap_flush(|tx| {
            tx.within(o(1)).add_sub_node(tx, i(1)); // op 0
            tx.insert_subnode(o(1), i(0), s(1, 0)); // op 1
            tx.remove_sub_node(o(1), 0); // op 2: under o1 too
            tx.within(o(0)).add_sub_node(tx, i(1)); // op 3: kept host, not named
            tx.remove_outer_node(o(1)); // op 4
        })
        .expect_err("staged under a removed host");
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

/// Corner: a removed referencing record does not dangle — removing a key
/// referenced only by a sub the same change set removes is legal
/// (survivors-only liveness at the record level), with the entry cleared
/// by target-as-membership and the other host doomed.
#[test]
fn removed_sub_covers_key_removal() {
    let st = sample_store();
    let out = st
        .edit()
        .tap_flush(|tx| {
            tx.remove_inner_node(i(2)); // op 0: s10's main_node, but…
            tx.remove_sub_node(o(1), 0); // op 1: …the sub dies too (clears o1's entry)
            tx.remove_outer_node(o(0)); // op 2: o0's map dies with it
        })
        .expect("removed referrer does not dangle");
    assert_eq!(out.store.inner_node_count(), 2);
    out.store.validate().expect("validate");
}

/// A per-module flush tap: stage, then flush, in one expression.
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
