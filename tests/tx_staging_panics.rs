//! M4 step 2 — staging-time programming errors panic at the staging
//! call: a foreign/stale ref stamp, removal of a `Pending` ref, and a
//! foreign-store handle at conversion. The dividing line (the plan's
//! flush section): panics are for ops whose acceptance would silently
//! corrupt or mis-wire the change set; `FlushError` is for individually
//! legal ops invalid against the store or change set as a whole.
//!
//! The remaining panic of the plan's surface arrives with its verb:
//! keyed insert onto a live key with keyed staging (steps 8/11) — a panic
//! cannot be exercised before the staging call exists. (`set_` of a
//! `Pending` ref landed with step 7's `set_` staging below.)

use metagraph_exper::implement;
use metagraph_exper::stdlib::Nodes;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn old_store() -> GraphStore {
    let mut s = GraphStore::new();
    let a = s.add_node();
    let b = s.add_node();
    s.add_edge(a, b);
    s
}

#[test]
#[should_panic(expected = "different transaction token")]
fn pending_ref_from_foreign_tx_panics_at_staging() {
    let s = old_store();
    let mut tx1 = s.edit();
    let p = tx1.add_node(); // stamped with tx1's token
    let mut tx2 = s.edit(); // a second, independent fork on the same store
    tx2.add_edge(p, NodeId(0)); // stamp check fires HERE
}

#[test]
#[should_panic(expected = "different transaction token")]
fn stale_pending_ref_panics_after_flush_epoch() {
    // The token is never re-minted, so a ref left over from a completed
    // transaction can never match a live one even on a fresh store.
    let s = old_store();
    let mut tx1 = s.edit();
    let p = tx1.add_node();
    drop(tx1);
    let mut tx2 = s.edit();
    tx2.add_edge(p, NodeId(0));
}

#[test]
#[should_panic(expected = "no old record to remove")]
fn remove_of_pending_ref_panics_at_staging() {
    let s = old_store();
    let mut tx = s.edit();
    let p = tx.add_node();
    tx.remove_node(p); // staged entities have no old record to remove
}

#[test]
#[should_panic(expected = "no old record to re-point")]
fn set_of_pending_ref_panics_at_staging() {
    // Step 7's `set_` staging: a staged record's determining fields are
    // fixed by its own add op — there is no old record to re-point. The
    // remaining panic of the plan's surface (keyed insert onto a live
    // key) lands with keyed staging (steps 8/11).
    let s = old_store();
    let mut tx = s.edit();
    let p = tx.add_edge(NodeId(0), NodeId(1));
    tx.set_source(p, NodeId(0));
}

#[test]
#[should_panic(expected = "foreign store")]
fn foreign_store_handle_panics_at_conversion() {
    let s1 = old_store();
    let s2 = old_store();
    let h = s1.graph().nodes().next().unwrap();
    let mut tx = s2.edit();
    // `h` borrows s1, which outlives the borrow of s2 — lifetimes alone
    // cannot catch this; the store-identity assert does.
    tx.remove_node(h);
}
