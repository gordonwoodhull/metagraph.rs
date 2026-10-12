//! Composed CSR (M3 step 8): the fused-root emission attempted against the
//! frontier note. `outer.out_edges` is sourced at the fused class
//! (`outer.Node <=> sub.Subgraph`), so `a` is the fused class's id,
//! `on_add` fires from the fused class's constructor, and the buckets stay
//! store-level — exactly the inference the plan carried as frontier until
//! emission. `inner.out_edges` binds `intrusive_list` on the same store:
//! the mixed-phase probe's composed twin (one `freeze` finalizes only the
//! phased backing; the per-record links cross the boundary inside the
//! shared arenas).

mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Hierarchy {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph,
                sub:    m2_fixtures::shapes::SubPlain;
            sub.MainNode <-> inner.Node;
            sub.MainEdge <-> inner.Edge;
            outer.Node  <=> sub.Subgraph;
        }
    }
}

mod csr_hier {
    use metagraph_exper::implement;
    use metagraph_exper::stdlib::{InEdges, Nodes, OutEdges};

    use crate::comp;

    implement! {
        comp::Hierarchy as CsrHierarchyStore {
            outer.nodes:     arena,
            outer.out_edges: csr_adjacency,
            outer.in_edges:  intrusive_list,
            inner.nodes:     arena,
            inner.out_edges: intrusive_list,
            inner.in_edges:  intrusive_list,
            sub.subnodes:    arena,
            sub.subedges:    arena,
        }
    }

    pub fn out_ids(s: &CsrHierarchyStore<Frozen>, n: OuterNodeId) -> Vec<u32> {
        let node = s
            .outer_graph()
            .nodes()
            .find(|x| x.id() == n)
            .unwrap();
        node.out_edges().map(|e| e.id().0).collect()
    }

    pub fn in_ids(s: &CsrHierarchyStore<Frozen>, n: OuterNodeId) -> Vec<u32> {
        let node = s
            .outer_graph()
            .nodes()
            .find(|x| x.id() == n)
            .unwrap();
        node.in_edges().map(|e| e.id().0).collect()
    }

    #[test]
    fn fused_root_csr_compacts_and_iterates() {
        let mut s = CsrHierarchyStore::new();
        let a = s.add_outer_node();
        let b = s.add_outer_node();
        let _e0 = s.add_outer_edge(a, b);
        let _e1 = s.add_outer_edge(a, b);
        // The inner level rides the same store, unphased.
        let i0 = s.add_inner_node();
        let i1 = s.add_inner_node();
        let _ie = s.add_inner_edge(i0, i1);
        // The fused class IS the subgraph host: subnodes embed in its record.
        let _sn = s.add_sub_node(a, i0);

        let f = s.freeze();
        f.validate().expect("fresh compaction must validate");
        assert_eq!(f.outer_node_count(), 2);
        assert_eq!(f.inner_node_count(), 2);
        assert_eq!(f.sub_node_count(a), 1);

        // CSR: forward insertion order, buckets indexed by the fused id.
        assert_eq!(out_ids(&f, a), vec![0, 1]);
        assert_eq!(out_ids(&f, b), Vec::<u32>::new());
        // Intrusive in-edges: reverse order, links untouched by the freeze.
        assert_eq!(in_ids(&f, b), vec![1, 0]);
        // The inner level's intrusive out-edges work on the frozen store.
        let inode = f
            .inner_graph()
            .nodes()
            .find(|n| n.id() == i0)
            .unwrap();
        assert_eq!(inode.out_edges().count(), 1);
    }

    #[test]
    fn fused_root_flush_replay_order() {
        // The order contracts on the composed store: kept members replay in
        // old order; the new edge appends LAST under CSR, leads under the
        // intrusive in-edges.
        let mut builder = CsrHierarchyStore::new();
        let a = builder.add_outer_node();
        let b = builder.add_outer_node();
        builder.add_outer_edge(a, b); // kept
        builder.add_outer_edge(a, b); // kept
        builder.add_outer_edge(a, b); // NEW
        let f = builder.finish().expect("flush validates");
        assert_eq!(out_ids(&f, a), vec![0, 1, 2]);
        assert_eq!(in_ids(&f, b), vec![2, 1, 0]);
    }

    mod corrupt {
        use super::*;

        #[test]
        fn validate_catches_csr_damage() {
            let mut s = CsrHierarchyStore::new();
            let a = s.add_outer_node();
            let b = s.add_outer_node();
            s.add_outer_edge(a, b);
            let mut f = s.freeze();
            assert!(f.validate().is_ok());
            // A child module reaches the store's private fields.
            f.offsets[1] += 1;
            let err = f.validate().unwrap_err();
            assert!(err.contains("offsets"), "message was: {err}");
        }
    }
}
