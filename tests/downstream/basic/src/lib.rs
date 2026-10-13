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

/// M4 step 13 — write-side portability: the transaction surface and the
/// marker-parameterized write traits resolve and apply from a dependent
/// crate (M1 step 9's pattern over M4's write surface). `GraphStore` here
/// is the plain-user Digraph from the module head.
#[cfg(test)]
mod write_side {
    use super::*;
    use metagraph_exper::stdlib::digraph::{AddEdge, AddNode, RemoveNode};
    use metagraph_exper::stdlib::Nodes;
    use metagraph_exper::tx::HandleToRef;

    trait TapFlush {
        fn tap_flush(
            self,
            f: impl FnOnce(&mut Tx<'_>),
        ) -> Result<FlushOutcome, FlushError>;
    }

    impl TapFlush for Tx<'_> {
        fn tap_flush(
            mut self,
            f: impl FnOnce(&mut Tx<'_>),
        ) -> Result<FlushOutcome, FlushError> {
            f(&mut self);
            self.flush()
        }
    }

    /// Concrete path: the whole edit/flush cycle from a dependent crate.
    #[test]
    fn downstream_edit_flush_smoke() {
        let mut s = GraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let e0 = s.add_edge(a, b);
        let _e1 = s.add_edge(b, c);
        let old_edges = s.edge_count();

        let out = s
            .edit()
            .tap_flush(|tx| {
                tx.remove_edge(e0);
                let d = tx.add_node();
                tx.add_edge(a, d);
            })
            .expect("flush");

        assert_eq!(out.report.node_map, vec![Some(0), Some(1), Some(2)]);
        // Old-id-indexed: e0 removed, e1 kept; the staged edge lands past
        // the map's end.
        assert_eq!(out.report.edge_map, vec![None, Some(0)]);
        assert_eq!(out.store.edge_count(), 2);
        out.store.validate().expect("flushed store valid");
        // The borrowed path leaves the old store untouched.
        assert_eq!(s.edge_count(), old_edges);
    }

    /// Generic transform: every bound names a write-trait/marker path the
    /// dependent crate must resolve — `Add`/`Remove` at the per-entity
    /// markers, `HandleToRef` mapping the frozen store's handles to refs.
    /// The same bound web drives `replace_node` in the in-crate
    /// tx_write_traits suite; here it proves the web is nameable and
    /// applicable from outside the declaring crate.
    fn splice_node<Dst, MN, ME>(
        dst: &mut Dst,
        victim: <Dst as HandleToRef<MN>>::Handle,
    ) where
        Dst: AddNode<MN>
            + AddEdge<
                ME,
                Source = <Dst as AddNode<MN>>::NodeRef,
                Target = <Dst as AddNode<MN>>::NodeRef,
            >
            + RemoveNode<MN, NodeRef = <Dst as AddNode<MN>>::NodeRef>
            + HandleToRef<MN, Ref = <Dst as AddNode<MN>>::NodeRef>,
        <Dst as HandleToRef<MN>>::Handle: Copy,
        <Dst as AddNode<MN>>::NodeRef: Copy,
    {
        dst.remove_node(<Dst as HandleToRef<MN>>::to_ref(dst, victim));
        let fresh = dst.add_node();
        dst.add_edge(fresh, fresh);
    }

    #[test]
    fn downstream_generic_transform_ports() {
        let mut s = GraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node(); // unconnected: removable with no danglers
        s.add_edge(a, b);
        s.validate().expect("valid");

        // A handle into the (unphased) store mints the removal's Existing
        // ref; the fresh node is `Pending` from the same transaction.
        let h = s.graph().nodes().find(|n| n.id() == c).unwrap();
        let mut tx = s.edit();
        splice_node(&mut tx, h);
        let out = tx.flush().expect("flush");

        // Node c removed; a,b keep their ids; the staged node lands at 2
        // with its self-loop as the only NEW edge (old edge 0 kept).
        assert_eq!(out.report.node_map, vec![Some(0), Some(1), None]);
        assert_eq!(out.report.edge_map, vec![Some(0)]);
        assert_eq!(out.store.edge_count(), 2);
        out.store.validate().expect("flushed store valid");
    }
}
