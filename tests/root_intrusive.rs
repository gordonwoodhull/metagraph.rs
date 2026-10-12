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
