//! The standard library arrows: six root single-arrow patterns, each the
//! permanent owner of its trait, plus the `Digraph` instantiation block.
//!
//! A root pattern (no instantiation source) declares a fresh trait and
//! registers its metadata through its registration courier — user-declared
//! roots work exactly the same way; the stdlib enjoys no privilege.
//! Instantiation blocks name roots by path and never restate cardinality
//! or associated types: `subnodes: stdlib::Nodes` is still called `.nodes()`
//! and yields `Nodes::Nodes<'a>`.
//!
//! Schema paths are resolved against the *declaring* crate (M2's
//! declaring-crate resolver): `pattern!` bakes `CARGO_PKG_NAME` into the
//! courier, and the implement/compose site maps it to a local dependency
//! name — so these `stdlib::…` paths mean `metagraph_exper::stdlib::…`
//! wherever the pattern travels. Leading-`::` paths pass verbatim;
//! `crate`-rooted spellings name the declaring crate, not the implement
//! site.

use metagraph_exper_macros::pattern;

pattern! { Nodes    { entities: root Graph, Node; arrows { nodes:     Graph -> Node, many; } } }
pattern! { OutEdges { entities: root Node, Edge; arrows { out_edges: Node -> Edge, many; } } }
pattern! { InEdges  { entities: root Node, Edge; arrows { in_edges:  Node -> Edge, many; } } }
pattern! { Edges    { entities: root Node, Edge; arrows { edges:     Node -> Edge, many; } } }
pattern! { Source   { entities: root Edge, Node; arrows { source:    Edge -> Node, one; } } }
pattern! { Target   { entities: root Edge, Node; arrows { target:    Edge -> Node, one; } } }

pattern! {
    Digraph {
        entities: root Graph, Node, Edge;
        arrows {
            nodes:     stdlib::Nodes,    Graph -> Node;
            out_edges: stdlib::OutEdges, Node -> Edge, by source;
            in_edges:  stdlib::InEdges,  Node -> Edge, by target;
            source:    stdlib::Source,   Edge -> Node;
            target:    stdlib::Target,   Edge -> Node;
            edges:     stdlib::Edges,    Node -> Edge := sum(out_edges, in_edges);
        }
    }
}

// The stdlib is the first consumer of its own patterns: this in-crate
// `implement!` exercises the full courier chain (schema courier bounce,
// six registration hops, codegen) with `extern crate self` resolving the
// generated absolute paths.
#[cfg(test)]
mod same_crate_tests {
    use metagraph_exper_macros::implement;

    use crate::stdlib::{Edges, InEdges, Nodes, OutEdges, Source, Target};

    implement! {
        crate::stdlib::Digraph as SameCrateGraphStore {
            nodes:     arena,
            out_edges: intrusive_list,
            in_edges:  intrusive_list,
        }
    }

    #[test]
    fn same_crate_smoke() {
        let mut s = SameCrateGraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let d = s.add_node();
        assert_eq!([a.0, b.0, c.0, d.0], [0, 1, 2, 3]);
        for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
            s.add_edge(x, y);
            s.add_edge(y, x);
        }
        assert_eq!(s.node_count(), 4);
        assert_eq!(s.edge_count(), 14);
        assert!(s.validate().is_ok());

        let g = s.graph();
        let a = g.nodes().next().unwrap();
        // Reverse insertion order: A's first out-edge is the last inserted
        // from A: A -> D.
        let e = a.out_edges().next().unwrap();
        assert_eq!(e.source().id(), NodeId(0));
        assert_eq!(e.target().id(), NodeId(3));
        assert_eq!(a.out_edges().count(), 5);
        assert_eq!(a.in_edges().count(), 5);
        assert_eq!(a.edges().count(), 10);
        let _ = EdgeId(0); // generated ids are nameable at the implement! site
    }
}
