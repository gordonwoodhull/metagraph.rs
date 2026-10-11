//! A downstream crate using the runtime under a renamed dependency
//! (`mg` for `metagraph-exper`). Generated runtime paths must resolve
//! through the rename.

use mg::implement;

implement! {
    mg::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mg::algo::bfs;
    use mg::stdlib::Nodes;

    #[test]
    fn renamed_dependency_works() {
        let mut s = GraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        s.add_edge(a, b);
        s.add_edge(b, c);
        assert_eq!(s.node_count(), 3);
        assert_eq!(s.edge_count(), 2);

        let g = s.graph();
        let a = g.nodes().next().unwrap();
        let order: Vec<u32> = bfs(a).into_iter().map(|n| n.id().0).collect();
        assert_eq!(order, [0, 1, 2]);
    }
}
