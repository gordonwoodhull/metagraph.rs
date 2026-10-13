//! M4 step 1 — the change-set data model: `Tx` over `&Store` (`edit()`),
//! token-stamped refs (`Existing | Pending`, `Copy`), one flat op queue
//! whose indices count every op, staging methods with local spellings.
//! Refs are opaque tokens: no read traits (the compile-fail fixture
//! `compilefail/read_on_ref.rs`); the change set is a plain struct — a
//! queue on the transaction, not a clone of the store.

use metagraph::implement;
use metagraph::stdlib::{Nodes, OutEdges};
use metagraph::tx::RefKind;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn old_store() -> (GraphStore, NodeId, EdgeId) {
    let mut s = GraphStore::new();
    let a = s.add_node();
    let b = s.add_node();
    let e = s.add_edge(a, b);
    (s, a, e)
}

#[test]
fn pending_refs_count_every_queued_op() {
    let (s, a, _e) = old_store();
    let mut tx = s.edit();
    let p0 = tx.add_node(); // op 0
    let _e1 = tx.add_edge(p0, a); // op 1
    tx.remove_edge(EdgeId(0)); // op 2 — removals occupy indices too
    let p3 = tx.add_node(); // op 3
    assert!(matches!(p0.kind, RefKind::Pending(0)));
    assert!(
        matches!(p3.kind, RefKind::Pending(3)),
        "indices count every op, removals included"
    );
}

#[test]
fn ids_and_handles_convert_to_existing_refs() {
    let (s, a, _e) = old_store();
    let mut tx = s.edit();
    // Raw old ids are accepted anywhere as `Existing` (unstamped — the id
    // is validated at flush).
    let er = tx.add_edge(a, NodeId(1));
    assert!(matches!(er.kind, RefKind::Pending(0)));
    // Old-store handles convert at the staging call (store identity
    // asserted inside the conversion).
    let g = s.graph();
    let ha = g.nodes().next().unwrap();
    let he = ha.out_edges().next().unwrap();
    tx.remove_edge(he);
    let _ = ha;
    // Refs are Copy.
    let q = er;
    let _ = er;
    let _ = q;
}

#[test]
fn tx_borrows_the_old_store_sharedly() {
    let (s, _a, _e) = old_store();
    let mut tx = s.edit();
    let p = tx.add_node();
    let _ = p;
    // The old store stays readable while staging — `old()` is the same
    // store, and reads through real handles still work.
    assert!(::core::ptr::eq(tx.old(), &s));
    assert_eq!(s.node_count(), 2);
    let g = s.graph();
    assert_eq!(g.nodes().count(), 2);
}
