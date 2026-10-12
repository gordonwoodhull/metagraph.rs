//! Cross-crate composed implement under renames: the composed pattern is
//! declared here, its members live in two other crates (the stdlib's
//! `Digraph` in `metagraph-exper`, `Subgraph` and its local roots in
//! `m2-fixtures`), and both are renamed dependencies. This is the
//! declaring-crate resolver's acceptance test: per-member baked packages
//! must map to local names (`mg`, `fx`) at the implement site.

use mg::{compose_pattern, implement};

mod comp {
    use mg::compose_pattern;

    compose_pattern! {
        EdgeInduced {
            instances:
                main: mg::stdlib::Digraph,
                sub:  fx::shapes::Subgraph;
            sub.MainNode <-> main.Node;
            sub.MainEdge <-> main.Edge;
        }
    }
}

use comp::edge_induced::markers;

implement! {
    comp::EdgeInduced as Store {
        main.nodes:     arena,
        main.out_edges: intrusive_list,
        main.in_edges:  intrusive_list,
        sub.subnodes:   arena,
        sub.subedges:   arena,
        sub.subnode:    hash_index,
        sub.subedge:    hash_index,
    }
}

impl Store {
    pub fn add_subedge(&mut self, me: EdgeId) -> SubEdgeId {
        for endpoint in [self.main_edge_source(me), self.main_edge_target(me)] {
            if self.subnode_by_key(endpoint).is_none() {
                let sn = self.add_sub_node(endpoint);
                self.insert_subnode(endpoint, sn);
            }
        }
        let se = self.add_sub_edge(me);
        self.insert_subedge(me, se);
        se
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx::roots::SubNodeOf;
    use mg::stdlib::Nodes;

    #[test]
    fn renamed_composed_implement_works() {
        let mut s = Store::new();
        let n0 = s.add_node();
        let n1 = s.add_node();
        let n2 = s.add_node();
        let e0 = s.add_edge(n0, n1);
        let e1 = s.add_edge(n1, n2);
        s.add_subedge(e0);
        s.add_subedge(e1);
        assert_eq!(s.sub_node_count(), 3);
        assert_eq!(s.sub_edge_count(), 2);
        assert!(s.validate().is_ok());

        let sg = s.sub_subgraph();
        // The fixture's local root trait resolves under the rename too.
        let sn = <Subgraph<'_> as SubNodeOf<markers::sub::Subnode>>::subnode(&sg, NodeId(1))
            .expect("n1 induced");
        assert_eq!(sn.id(), SubNodeId(1));

        let g = s.main_graph();
        assert_eq!(
            <Graph<'_> as Nodes<markers::main::Nodes>>::nodes(&g).count(),
            3
        );
    }
}
