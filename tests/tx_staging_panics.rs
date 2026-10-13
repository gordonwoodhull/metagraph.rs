//! M4 step 2 (collected at step 13) — staging-time programming errors
//! panic at the staging call. The dividing line (the plan's flush
//! section): panics are for ops whose acceptance would silently corrupt
//! or mis-wire the change set; `FlushError` is for individually legal
//! ops invalid against the store or change set as a whole.
//!
//! This file collects every panic exerciseable over the FLAT surfaces:
//!
//! - the core ref-discipline panics over the plain Digraph store (step 2
//!   + step 7's `set_`): foreign/stale stamp, `Pending` removal, `Pending`
//!   re-point, foreign-store handle;
//! - the keyed staging panics over the flat KeyedRoot store (steps
//!   8/13): a live key in the borrowed old store, a key already staged in
//!   this transaction (moved here from tx_keyed.rs).
//!
//! The fixture-bound remainder of the panic inventory, and why each stays
//! with its fixture:
//!
//! - `tx_keyed_carry.rs` — the hosted per-record keyed panics ("already
//!   live in host", "already claimed by a staged insert"): the
//!   HierarchyStore composed fixture is the only hosted-keyed surface;
//! - `tx_nested.rs` — the SubTx foreign-token panic and the "replay table
//!   miss" internal-bug panic: both forge path-shaped refs through the
//!   nested fixture's private surfaces;
//! - `edge_induced.rs` — the induction escape hatch's reserve-ordering
//!   panics (×2 pairs) and its foreign-token panic: they stage through a
//!   user-written `Tx` extension over the edge-induced fixture;
//! - `tx_phased_flush.rs` — the flat core panics re-exercised over the
//!   phased store (the step-12 surface: a phased Tx is borrowed-only and
//!   ends in `finish()`);
//! - `flat_keyed.rs` / `hierarchy_subgraphs.rs` — builder-side duplicate
//!   key panics (direct construction, not transaction staging — listed
//!   for the inventory only).

use metagraph::implement;
use metagraph::stdlib::Nodes;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

// The flat keyed store lives in its own module: each `implement!` emits
// the Tx types (`Op`, `Tx`, `Resolve`, ...) into the invoking module
// unqualified, so only one Tx-emitting store fits per module.
mod keyed {
    use metagraph::implement;

    implement! {
        m2_fixtures::shapes::KeyedRoot as KeyedStore {
            nodes: arena,
            marks: hash_index,
        }
    }

    /// n0..n3, m0..m2, entries n0->m0 and n1->m1 (n2, n3 unkeyed).
    pub fn old_store() -> KeyedStore {
        let mut s = KeyedStore::new();
        for _ in 0..4 {
            s.add_n();
        }
        for _ in 0..3 {
            s.add_m();
        }
        s.insert_marks(NId(0), MId(0));
        s.insert_marks(NId(1), MId(1));
        s
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
    // fixed by its own add op — there is no old record to re-point.
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

#[test]
#[should_panic(expected = "exists in the borrowed old store")]
fn live_key_in_old_store_panics_at_staging() {
    // Steps 8/13's keyed staging: a keyed insert onto a key that is live
    // in the borrowed old store would silently double-claim it.
    let base = keyed::old_store();
    let mut tx = base.edit();
    let m = tx.add_m();
    tx.insert_marks(keyed::NId(0), m); // n0 keys m0: live
}

#[test]
#[should_panic(expected = "already staged in this transaction")]
fn already_staged_key_panics_at_staging() {
    // ... and the same Pending key twice in one change set (a staged
    // removal frees no key; re-pointing one is two flushes).
    let base = keyed::old_store();
    let mut tx = base.edit();
    let n = tx.add_n();
    let m0 = tx.add_m();
    let m1 = tx.add_m();
    tx.insert_marks(n, m0);
    tx.insert_marks(n, m1); // the same Pending key twice
}
