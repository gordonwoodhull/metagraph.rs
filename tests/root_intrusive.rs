//! The relaxed store-level intrusive placement (M3, the merge chore from
//! M2's frontier): a `many` arrow sourced at an unfused root is DSL-legal —
//! the head is a store-level field, the insert hook is self-less (the root
//! owns no id), the op is a store-level method (never a root-handle
//! method), and the read rides the root handle. The flat side of this
//! placement was probe-verified (/tmp/m3-probe-r3-root-intrusive, 5/5);
//! this drives the composed side, emitted since the M3 codegen port.

use metagraph_exper::stdlib::OutEdges;
use metagraph_exper::implement;

mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Relaxed {
            instances:
                m: metagraph_exper::stdlib::Digraph,
                r: m2_fixtures::neg::RootMany;
        }
    }
}

use comp::relaxed::markers;

implement! {
    crate::comp::Relaxed as RelaxedStore {
        m.nodes:     arena,
        m.out_edges: intrusive_list,
        m.in_edges:  intrusive_list,
        r.cs:        intrusive_list,
    }
}

#[test]
fn root_intrusive_walks() {
    let mut s = RelaxedStore::new();
    let a = s.add_node();
    let b = s.add_node();
    s.add_edge(a, b);
    let c0 = s.add_c();
    let c1 = s.add_c();
    // Self-less store-level op: insert_cs(member) — no self id exists.
    s.insert_cs(c1);
    s.insert_cs(c0);
    // Read rides the root handle: G's list, walked in reverse insertion.
    let g = s.r_g();
    let walked: Vec<CId> = <G<'_> as OutEdges<markers::r::Cs>>::out_edges(&g)
        .map(|c| c.id())
        .collect();
    assert_eq!(walked, [c0, c1]);
    assert!(s.validate().is_ok());
}

#[test]
fn tx_self_less_insert_and_root_headed_carry() {
    // M4 step 8: the composed Tx over the same store — the root-sourced
    // list's head folds to a store field (one chain for the whole
    // store), and the self-less `insert_cs(member)` staging verb queues
    // an op the flush replays through the store-level hook on top of the
    // carried chain.
    let mut base = RelaxedStore::new();
    let a = base.add_node();
    let b = base.add_node();
    base.add_edge(a, b);
    let c0 = base.add_c();
    let c1 = base.add_c();
    base.insert_cs(c1);
    base.insert_cs(c0); // chain [c0, c1]

    let mut tx = base.edit();
    tx.remove_c(c1); // splice the tail; the chain becomes [c0]
    let c2 = tx.add_c(); // born unlisted (a staged record joins no list
    // without an insert op)
    tx.insert_cs(c2); // self-less: no host ref exists to pass
    let out = tx.flush().expect("flush");

    // Carried [c0'], then the staged c2' push-fronts: [c2', c0']. New
    // ids: c0 -> 0 (c1 removed), and the staged c2 appends after the one
    // kept record -> 1.
    let g = out.store.r_g();
    let walked: Vec<CId> = <G<'_> as OutEdges<markers::r::Cs>>::out_edges(&g)
        .map(|c| c.id())
        .collect();
    assert_eq!(walked, [CId(1), CId(0)]);
    assert_eq!(out.store.c_count(), 2); // c0 kept, c1 removed, c2 staged
    assert_eq!(out.report.c_map, vec![Some(0), None]);
    out.store.validate().expect("validate");
    // Old store untouched.
    let g = base.r_g();
    let old_walked: Vec<CId> = <G<'_> as OutEdges<markers::r::Cs>>::out_edges(&g)
        .map(|c| c.id())
        .collect();
    assert_eq!(old_walked, [c0, c1]);
}
