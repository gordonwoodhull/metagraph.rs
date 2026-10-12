//! A bipartite graph as a plain `pattern!` (M2 step 14): two node sorts
//! and edges whose endpoints are typed by sort — arrows picked à la
//! carte from the stdlib, no composition, no renunciation. Everything
//! here is M1 machinery.
//!
//! This file is also the marked limit of design invariant 6 (structural
//! rejection, not semantic laws). The two *sorts* make the sides
//! structural: `source` is a `Left` and `target` is a `Right` by type,
//! so every edge crosses by construction. Bipartiteness of an *unsorted*
//! graph — a 2-coloring property — is a semantic constraint the arrows
//! cannot state, and nothing here pretends otherwise: the store happily
//! accepts any `(LeftId, RightId)` pair, and no `validate()` check
//! could decide side-membership for a plain `Digraph` node.

use metagraph_exper::stdlib::{InEdges, Nodes, OutEdges, Source, Target};

mod pat {
    use metagraph_exper::pattern;

    pattern! {
        Bipartite {
            entities: root Graph, Left, Right, Edge;
            arrows {
                lefts:  metagraph_exper::stdlib::Nodes,    Graph -> Left;
                rights: metagraph_exper::stdlib::Nodes,    Graph -> Right;
                from:   metagraph_exper::stdlib::OutEdges, Left  -> Edge, by src;
                into:   metagraph_exper::stdlib::InEdges,  Right -> Edge, by dst;
                src:    metagraph_exper::stdlib::Source,   Edge  -> Left;
                dst:    metagraph_exper::stdlib::Target,   Edge  -> Right;
            }
        }
    }
}

use metagraph_exper::implement;

implement! {
    pat::Bipartite as BipartiteStore {
        lefts:  arena,
        rights: arena,
        from:   intrusive_list,
        into:   intrusive_list,
        // `src`/`dst` are `one` arrows: field-backed by default.
    }
}

use pat::bipartite::markers;

/// K_{2,3}: two lefts, three rights, every pair crossed.
fn complete_2_3() -> BipartiteStore {
    let mut s = BipartiteStore::new();
    let lefts = [s.add_left(), s.add_left()];
    let rights = [s.add_right(), s.add_right(), s.add_right()];
    for l in lefts {
        for r in rights {
            s.add_edge(l, r);
        }
    }
    s
}

#[test]
fn two_sorts_assemble_a_la_carte() {
    let s = complete_2_3();
    assert!(s.validate().is_ok());
    assert_eq!(s.left_count(), 2);
    assert_eq!(s.right_count(), 3);
    assert_eq!(s.edge_count(), 6);

    // Two instantiations of one root on the same handle: the `nodes()`
    // calls are marker-qualified, the M1 spelling.
    let g = s.graph();
    let left_count = <Graph<'_> as Nodes<markers::Lefts>>::nodes(&g).count();
    let right_count = <Graph<'_> as Nodes<markers::Rights>>::nodes(&g).count();
    assert_eq!((left_count, right_count), (2, 3));

    // Each left sees all three rights through its `from` list, and each
    // edge crosses by type: `source()` is a Left handle, `target()` a
    // Right handle. The structural typing is the whole story — there is
    // no bipartiteness check to appeal to, and none is needed.
    let l0 = <Graph<'_> as Nodes<markers::Lefts>>::nodes(&g).next().unwrap();
    let seen_rights: Vec<RightId> = l0.out_edges().map(|e| e.target().id()).collect();
    assert_eq!(seen_rights, [RightId(2), RightId(1), RightId(0)]); // reverse insertion

    let r0 = <Graph<'_> as Nodes<markers::Rights>>::nodes(&g).next().unwrap();
    let seen_lefts: Vec<LeftId> = r0.in_edges().map(|e| e.source().id()).collect();
    assert_eq!(seen_lefts, [LeftId(1), LeftId(0)]); // reverse insertion order

    // Every edge reports both endpoints; the determinant walk in
    // validate() covers list membership against these fields.
    let e = l0.out_edges().next().unwrap();
    assert_eq!(e.source().id(), LeftId(0));
    assert_eq!(e.target().id(), RightId(2));
}

#[test]
fn sides_are_disjoint_by_type() {
    // The pattern exports two id types; an edge cannot hold the wrong
    // side (`add_edge` takes `(LeftId, RightId)`), so K_{2,3} needs no
    // runtime side check. What the arrows cannot do is partition an
    // unsorted node sort — that is invariant 6's limit, documented in
    // the module header, not asserted here.
    let s = complete_2_3();
    assert_eq!(s.edge_count(), 6);
    assert!(s.validate().is_ok());
}
