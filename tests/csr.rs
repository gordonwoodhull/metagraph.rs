//! CSR adjacency + the type-state store (M3 step 8): spike 6's CSR tests
//! and the mixed-phase probe's shape, ported onto macro-generated stores.
//!
//! Two drivers:
//! - `mixed` — the stdlib `Digraph` with `out_edges: csr_adjacency` and
//!   `in_edges: intrusive_list` on ONE store (the probe's shape): one
//!   `add_edge` fires both backings' hooks, `freeze` finalizes only the CSR
//!   state, and each backing's validate catches its own damage. `out_edges`
//!   is `by source` (insert fires from `add_edge`); `in_edges` is
//!   `by target` on the unphased backing.
//! - `primitive` — `m2_fixtures::shapes::CsrDigraph`: a primitive `many`
//!   arrow (`out: N -> E`, no `by`) bound to `csr_adjacency`; inserts ride
//!   the public `insert_out` op and duplicates are legal (forward order) —
//!   the spike's `primitive_insert_appends_in_call_order` contract.
//!
//! The corruption tests sit in child modules of their `implement!` (the
//! generated records/fields are private to it — the step-6 driver shape).

mod mixed {
    use metagraph::implement;
    use metagraph::stdlib::{InEdges, Nodes, OutEdges, Source, Target};

    implement! {
        metagraph::stdlib::Digraph as MixedStore {
            nodes:     arena,
            out_edges: csr_adjacency,
            in_edges:  intrusive_list,
        }
    }

    /// The probe's triangle: nodes a, n1, c; e0=a->n1, e1=a->c, e2=c->n1.
    pub fn build_triangle() -> (MixedStore<Build>, [NodeId; 3], [EdgeId; 3]) {
        let mut s = MixedStore::new();
        let a = s.add_node();
        let n1 = s.add_node();
        let c = s.add_node();
        let e0 = s.add_edge(a, n1);
        let e1 = s.add_edge(a, c);
        let e2 = s.add_edge(c, n1);
        (s, [a, n1, c], [e0, e1, e2])
    }

    pub fn out_ids(s: &MixedStore<Frozen>, n: NodeId) -> Vec<u32> {
        let node = s.graph().nodes().find(|x| x.id() == n).unwrap();
        node.out_edges().map(|e| e.id().0).collect()
    }

    pub fn in_ids(s: &MixedStore<Frozen>, n: NodeId) -> Vec<u32> {
        let node = s.graph().nodes().find(|x| x.id() == n).unwrap();
        node.in_edges().map(|e| e.id().0).collect()
    }

    #[test]
    fn mixed_bindings_iteration_orders() {
        let (b, [a, n1, c], _) = build_triangle();
        let f = b.freeze();
        // CSR: forward insertion order.
        assert_eq!(out_ids(&f, a), vec![0, 1]);
        assert_eq!(out_ids(&f, c), vec![2]);
        assert_eq!(out_ids(&f, n1), Vec::<u32>::new());
        // Intrusive: reverse insertion order (push-front).
        assert_eq!(in_ids(&f, n1), vec![2, 0]);
        assert_eq!(in_ids(&f, c), vec![1]);
    }

    #[test]
    fn forward_insertion_order_after_compaction() {
        // Spike 6's Königsberg multigraph: insertion order pins the ids.
        let mut s = MixedStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let d = s.add_node();
        s.add_edge(a, b); // e0
        s.add_edge(a, c); // e1
        s.add_edge(b, a); // e2
        s.add_edge(c, a); // e3
        s.add_edge(c, b); // e4
        s.add_edge(d, a); // e5
        s.add_edge(d, b); // e6
        let f = s.freeze();
        // Forward per source, preserved by compaction (intrusive would
        // give [1, 0] for node a).
        assert_eq!(out_ids(&f, a), vec![0, 1]);
        assert_eq!(out_ids(&f, b), vec![2]);
        assert_eq!(out_ids(&f, c), vec![3, 4]);
        assert_eq!(out_ids(&f, d), vec![5, 6]);
    }

    #[test]
    fn flush_style_rebuild_matches() {
        // M4 flush-replay, simulated: replaying the SAME op sequence
        // through a fresh builder compacts to the same frozen store.
        let (b1, _, _) = build_triangle();
        let (b2, _, _) = build_triangle();
        let (f1, f2) = (b1.freeze(), b2.freeze());
        assert_eq!(f1.node_count(), f2.node_count());
        assert_eq!(f1.edge_count(), f2.edge_count());
        for n in [NodeId(0), NodeId(1), NodeId(2)] {
            assert_eq!(out_ids(&f1, n), out_ids(&f2, n));
            assert_eq!(in_ids(&f1, n), in_ids(&f2, n));
        }
    }

    #[test]
    fn flush_replay_order_contract() {
        // The order contracts side by side (spike 6): kept members replay
        // in old iteration order; under CSR the NEW member iterates LAST
        // (forward insertion), under intrusive_list FIRST (push-front).
        let mut s = MixedStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let e0 = s.add_edge(a, b);
        let _e1 = s.add_edge(a, c);
        let old = s.freeze();
        assert_eq!(out_ids(&old, a), vec![e0.0, 1]);

        // Change set: keep e0 and e1, add a NEW edge a->b.
        let mut builder = MixedStore::new();
        let na = builder.add_node();
        let nb = builder.add_node();
        let nc = builder.add_node();
        builder.add_edge(na, nb); // kept e0
        builder.add_edge(na, nc); // kept e1
        builder.add_edge(na, nb); // NEW edge — id 2 in the rebuilt store
        let new_store = builder.finish().expect("flush validates");
        // CSR (forward): the new edge appends after the kept ones.
        assert_eq!(out_ids(&new_store, na), vec![0, 1, 2]);
        // Intrusive (reverse): the new edge leads.
        assert_eq!(in_ids(&new_store, nb), vec![2, 0]);
    }

    #[test]
    fn empty_range_for_edgeless_source() {
        let (b, [_, n1, _], _) = build_triangle();
        let f = b.freeze();
        // n1 has only an in-edge; its out-range must be empty.
        assert_eq!(out_ids(&f, n1), Vec::<u32>::new());
        let node = f.graph().nodes().nth(1).unwrap();
        assert_eq!(node.out_edges().count(), 0);
        assert_eq!(node.out_edges().next(), None);
    }

    #[test]
    fn validate_ok_then_source_target_reads() {
        let (b, [a, _, _], _) = build_triangle();
        let f = b.freeze();
        f.validate().expect("fresh compaction must validate");
        let node = f.graph().nodes().find(|n| n.id() == a).unwrap();
        let e0 = node.out_edges().next().unwrap();
        assert_eq!(e0.source().id(), a);
        assert_eq!(e0.target().id(), NodeId(1));
        // The non-phased read view is frozen-side too.
        assert_eq!(f.source(e0.id()), a);
        assert_eq!(f.target(e0.id()), NodeId(1));
    }

    #[test]
    fn finish_returns_frozen_valid() {
        let (b, _, _) = build_triangle();
        let f = b.finish().expect("finish should be Ok");
        assert_eq!(f.node_count(), 3);
        assert_eq!(f.edge_count(), 3);
    }

    mod corrupt {
        use super::*;

        #[test]
        fn validate_catches_csr_damage() {
            let (b, _, _) = build_triangle();
            let mut f = b.freeze();
            assert!(f.validate().is_ok());
            // A child module reaches the store's private fields: break the
            // offsets invariant (was [0, 2, 2, 3]).
            f.offsets[1] += 1;
            let err = f.validate().unwrap_err();
            assert!(err.contains("offsets"), "message was: {err}");
        }

        #[test]
        fn validate_catches_intrusive_damage() {
            let (b, _, [e0, _, _]) = build_triangle();
            let mut f = b.freeze();
            // The per-record link fields crossed the boundary untouched;
            // the uniform range pre-pass names the offending id.
            f.edge_arena[e0.0 as usize].next_in = Some(EdgeId(99));
            let err = f.validate().unwrap_err();
            assert!(err.contains("EdgeId(99)"), "message was: {err}");
        }
    }
}

mod primitive {
    use metagraph::implement;
    use metagraph::stdlib::{Nodes, OutEdges, Source, Target};

    implement! {
        m2_fixtures::shapes::CsrDigraph as PrimStore {
            nodes:  arena,
            out:    csr_adjacency,
            // `source`/`target` are `one` arrows: they default to `field`.
        }
    }

    fn out_ids(s: &PrimStore<Frozen>, n: NId) -> Vec<u32> {
        let node = s.g().nodes().find(|x| x.id() == n).unwrap();
        node.out_edges().map(|e| e.id().0).collect()
    }

    #[test]
    fn primitive_insert_appends_in_call_order() {
        let mut s = PrimStore::new();
        let x = s.add_n();
        let y = s.add_n();
        let e0 = s.add_e(x, y);
        let e1 = s.add_e(x, y);
        // The write path is the public op; membership is asserted explicitly
        // per edge, and re-asserting appends (duplicates are legal, forward
        // order — the spike's contract).
        s.insert_out(x, e0);
        s.insert_out(x, e1);
        s.insert_out(x, e0); // re-asserted: appears again, forward
        let e2 = s.add_e(x, y);
        s.insert_out(x, e2);
        let f = s.freeze();
        assert_eq!(out_ids(&f, x), vec![0, 1, 0, 2]);
        assert_eq!(out_ids(&f, y), Vec::<u32>::new());
    }

    #[test]
    fn primitive_forward_order_reads_and_validate() {
        let mut s = PrimStore::new();
        let a = s.add_n();
        let b = s.add_n();
        let c = s.add_n();
        let e0 = s.add_e(a, b);
        s.insert_out(a, e0);
        let e1 = s.add_e(a, c);
        s.insert_out(a, e1);
        let f = s.freeze();
        f.validate().expect("fresh compaction must validate");
        assert_eq!(out_ids(&f, a), vec![0, 1]);
        assert_eq!(out_ids(&f, b), Vec::<u32>::new());

        let node = f.g().nodes().find(|n| n.id() == a).unwrap();
        let first = node.out_edges().next().unwrap();
        assert_eq!(first.source().id(), a);
        assert_eq!(first.target().id(), NId(1));
        assert_eq!(f.source(first.id()), a);
        assert_eq!(f.target(first.id()), NId(1));
    }

    mod corrupt {
        use super::*;

        #[test]
        fn validate_catches_bucket_damage() {
            let mut s = PrimStore::new();
            let x = s.add_n();
            let y = s.add_n();
            let e = s.add_e(x, y);
            s.insert_out(x, e);
            let mut f = s.freeze();
            assert!(f.validate().is_ok());
            // Corrupt the compacted adjacency (was offsets [0, 1, 1]).
            f.adj.clear();
            let err = f.validate().unwrap_err();
            assert!(err.contains("offsets"), "message was: {err}");
        }
    }
}
