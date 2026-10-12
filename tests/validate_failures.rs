//! Runtime drivers for the `validate()` canonical inventory (M2 step 13).
//! Every corruption below is reachable through the public builder ops —
//! the ops trust the builder; `validate()` is the boundary that catches
//! the damage, and M4's flush will run exactly these checks. Each test
//! asserts on the report text, including the failure-shape distinction
//! the intrusive-ownership check makes (shared membership vs cycle).

mod hier_comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Hierarchy {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph,
                sub:    m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

mod refin_comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Refinement {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph,
                chain: m2_fixtures::shapes::Chain;
            outer.Edge <=> chain.Chain;
            chain.Elem <-> inner.Edge;
        }
    }
}

mod hier {
    use metagraph_exper::implement;

    implement! {
        crate::hier_comp::Hierarchy as HierStore {
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
}

mod refin {
    use metagraph_exper::implement;

    implement! {
        crate::refin_comp::Refinement as RefStore {
            outer.nodes:     arena,
            outer.out_edges: intrusive_list,
            outer.in_edges:  intrusive_list,
            inner.nodes:     arena,
            inner.out_edges: intrusive_list,
            inner.in_edges:  intrusive_list,
            chain.elems:     intrusive_list,
        }
    }
}

mod flat {
    use metagraph_exper::implement;

    implement! {
        metagraph_exper::stdlib::Digraph as FlatStore {
            nodes:     arena,
            out_edges: intrusive_list,
            in_edges:  intrusive_list,
        }
    }
}

use hier::HierStore;
use refin::RefStore;

/// Two outer nodes (each is-also a subgraph), two inner nodes, one
/// subnode under each host.
fn hier_store() -> HierStore {
    use hier::OuterNodeId;
    let mut s = HierStore::new();
    let a = s.add_outer_node();
    let b = s.add_outer_node();
    s.add_outer_edge(a, b);
    let i0 = s.add_inner_node();
    let i1 = s.add_inner_node();
    let s0 = s.add_sub_node(a, i0);
    s.insert_subnode(OuterNodeId(0), i0, s0);
    let s1 = s.add_sub_node(b, i1);
    s.insert_subnode(OuterNodeId(1), i1, s1);
    s
}

/// Two outer edges a->b, b->c over three nodes; ab's chain is [e01, e12],
/// bc's chain is [e23].
fn refin_store() -> RefStore {
    let mut s = RefStore::new();
    let (a, b, c) = (s.add_outer_node(), s.add_outer_node(), s.add_outer_node());
    let ab = s.add_outer_edge(a, b);
    let bc = s.add_outer_edge(b, c);
    let (i0, i1, i2, i3) = (
        s.add_inner_node(),
        s.add_inner_node(),
        s.add_inner_node(),
        s.add_inner_node(),
    );
    let e01 = s.add_inner_edge(i0, i1);
    let e12 = s.add_inner_edge(i1, i2);
    let e23 = s.add_inner_edge(i2, i3);
    s.insert_elems(ab, e12);
    s.insert_elems(ab, e01);
    s.insert_elems(bc, e23);
    s
}

#[test]
fn keyed_maps_check_key_existence() {
    use hier::{InnerNodeId, OuterNodeId, SubNodeId};
    let mut s = hier_store();
    assert!(s.validate().is_ok());
    // A ghost key: the insert op checks only liveness of the key in THIS
    // map, not that the key names a record.
    let s0 = SubNodeId(OuterNodeId(0), 0);
    s.insert_subnode(OuterNodeId(0), InnerNodeId(99), s0);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("key InnerNodeId(99) does not name a record"),
        "{msg}"
    );
}

#[test]
fn keyed_maps_check_value_in_arena() {
    use hier::{InnerNodeId, OuterNodeId, SubNodeId};
    let mut s = hier_store();
    let dead = SubNodeId(OuterNodeId(0), 99);
    // Key InnerNodeId(1) is live and not yet in o0's map, so the value is
    // the only thing wrong.
    s.insert_subnode(OuterNodeId(0), InnerNodeId(1), dead);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("value SubNodeId(OuterNodeId(0), 99)") && msg.contains("is not a live record"),
        "{msg}"
    );
}

#[test]
fn keyed_maps_check_value_host_agreement() {
    use hier::{InnerNodeId, OuterNodeId, SubNodeId};
    let mut s = hier_store();
    // SubNodeId(o1, 0) is a live record — in bounds under host o1. Filed
    // into o0's map it passes value-in-arena and only the host check
    // catches it.
    let foreign = SubNodeId(OuterNodeId(1), 0);
    s.insert_subnode(OuterNodeId(0), InnerNodeId(5), foreign);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("is anchored at the wrong host"),
        "{msg}"
    );
}

#[test]
fn primitive_intrusive_lists_report_shared_membership() {
    use refin::{InnerEdgeId, OuterEdgeId};
    let mut s = refin_store();
    assert!(s.validate().is_ok());
    // e23 is already the sole member of bc's chain; inserting it into
    // ab's chain too leaves it in two lists.
    let (ab, e23) = (OuterEdgeId(0), InnerEdgeId(2));
    s.insert_elems(ab, e23);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("appears in more than one list — shared membership"),
        "{msg}"
    );
}

#[test]
fn primitive_intrusive_lists_report_cycles() {
    use refin::{InnerEdgeId, OuterEdgeId};
    let mut s = refin_store();
    assert!(s.validate().is_ok());
    // Inserting an existing head into its own list again self-links it:
    // e01.next = e01.
    let (ab, e01) = (OuterEdgeId(0), InnerEdgeId(0));
    s.insert_elems(ab, e01);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("repeats InnerEdgeId(0) — a cycle"),
        "{msg}"
    );
}

#[test]
fn by_derived_lists_report_determinant_contradiction_and_coverage() {
    let mut s = flat::FlatStore::new();
    let (a, b) = (s.add_node(), s.add_node());
    let e = s.add_edge(a, b);
    assert!(s.validate().is_ok());
    // The field write moves e's determinant without relinking the lists:
    // e stays in a's out_edges list (contradiction) and never appears in
    // b's (missing coverage).
    s.set_target(e, a);
    let msg = s.validate().unwrap_err();
    assert!(
        msg.contains("membership contradicts the determinant")
            && msg.contains("is missing from the list of its `target`"),
        "{msg}"
    );
}
