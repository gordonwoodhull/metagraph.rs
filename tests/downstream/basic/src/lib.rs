//! A downstream crate implementing the stdlib `Digraph` pattern — the
//! ordinary user path: nothing declared locally, everything assembled from
//! the standard library.

use metagraph_exper::implement;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metagraph_exper::algo::{bfs, dfs};
    use metagraph_exper::stdlib::{Edges, Nodes, OutEdges, Source, Target};

    #[test]
    fn downstream_koenigsberg_smoke() {
        let mut s = GraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let d = s.add_node();
        for (x, y) in [(a, b), (a, b), (a, c), (a, c), (a, d), (b, d), (c, d)] {
            s.add_edge(x, y);
            s.add_edge(y, x);
        }
        assert_eq!(s.node_count(), 4);
        assert_eq!(s.edge_count(), 14);
        assert!(s.validate().is_ok());

        let g = s.graph();
        let a = g.nodes().next().unwrap();
        let e = a.out_edges().next().unwrap();
        assert_eq!(e.source().id(), NodeId(0)); // A's last out-edge is A -> D
        assert_eq!(e.target().id(), NodeId(3));
        assert_eq!(a.edges().count(), 10);

        let order: Vec<u32> = bfs(a).into_iter().map(|n| n.id().0).collect();
        assert_eq!(order, [0, 3, 2, 1]);
        let order: Vec<u32> = dfs(a).into_iter().map(|n| n.id().0).collect();
        assert_eq!(order, [0, 1, 3, 2]);

        // Trait and marker paths are nameable from a dependent crate.
        fn assert_nameable(x: Node<'_>) {
            let _ = core::any::type_name::<
                <Node<'_> as OutEdges<metagraph_exper::stdlib::digraph::markers::OutEdges>>::Edge,
            >();
            let _ = x;
        }
        assert_nameable(a);
    }
}
