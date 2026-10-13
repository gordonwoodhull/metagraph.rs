//! The `on_unlink` delete verb (M3's clause; M4's `set_` re-pointing
//! lowers through it): walk-and-splice removal from an intrusive list,
//! total — the hook returns whether the record was found, absence is a
//! legitimate answer, never a panic.
//!
//! Three placements drive the flat emission (primitive membership,
//! `by`-derived membership re-pointed through the grown orchestration,
//! and the root-sourced store-level head), and one composed implement
//! drives the instance-qualified emission. One implement! per module —
//! the generated names would collide.

mod primitive {
    use metagraph_exper::implement;
    use metagraph_exper::stdlib::{Nodes, OutEdges};

    implement! {
        m2_fixtures::shapes::CsrDigraph as PrimLinks {
            nodes:  arena,
            out:    intrusive_list,
            // `source`/`target` are `one` arrows: they default to `field`.
        }
    }

    fn out_ids(s: &PrimLinks, n: NId) -> Vec<u32> {
        let node = s.g().nodes().find(|x| x.id() == n).unwrap();
        node.out_edges().map(|e| e.id().0).collect()
    }

    #[test]
    fn head_middle_tail_and_absent() {
        let mut s = PrimLinks::new();
        let a = s.add_n();
        let b = s.add_n();
        let e0 = s.add_e(a, b);
        let e1 = s.add_e(a, b);
        let e2 = s.add_e(a, b);
        // Reverse insertion order: a's list is e2, e1, e0.
        s.insert_out(a, e0);
        s.insert_out(a, e1);
        s.insert_out(a, e2);
        assert_eq!(out_ids(&s, a), vec![2, 1, 0]);

        // HEAD: the head pointer itself is spliced.
        assert!(s.__unlink_out(a, e2));
        assert_eq!(out_ids(&s, a), vec![1, 0]);
        assert!(s.validate().is_ok());

        // MIDDLE: the predecessor's link bypasses the splice.
        assert!(s.__unlink_out(a, e0));
        assert_eq!(out_ids(&s, a), vec![1]);
        assert!(s.validate().is_ok());

        // TAIL.
        assert!(s.__unlink_out(a, e1));
        assert_eq!(out_ids(&s, a), Vec::<u32>::new());
        assert!(s.validate().is_ok());

        // ABSENT: total — false, no panic. Wrong self, and an already
        // unlinked record asked for twice.
        assert!(!s.__unlink_out(b, e0));
        assert!(!s.__unlink_out(a, e1));

        // Re-assert after unlink: the record is linkable again (the
        // splice zeroed its `next`, so the list stays well-formed).
        s.insert_out(a, e1);
        s.insert_out(a, e0);
        assert_eq!(out_ids(&s, a), vec![0, 1]);
        assert!(s.validate().is_ok());
    }
}

mod by_derived {
    use metagraph_exper::implement;
    use metagraph_exper::stdlib::{InEdges, Nodes, OutEdges};

    implement! {
        metagraph_exper::stdlib::Digraph as GraphStore {
            nodes:     arena,
            out_edges: intrusive_list,
            in_edges:  intrusive_list,
        }
    }

    #[test]
    fn repoint_through_the_trio() {
        let mut s = GraphStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let e = s.add_edge(a, b);
        let _f = s.add_edge(a, b);
        // a's out-list: f, e (reverse insertion).
        fn out(s: &GraphStore, n: NodeId) -> Vec<u32> {
            s.graph()
                .nodes()
                .find(|x| x.id() == n)
                .unwrap()
                .out_edges()
                .map(|x| x.id().0)
                .collect()
        }
        assert_eq!(out(&s, a), vec![1, 0]);
        assert!(s.validate().is_ok());

        // M4 step 7 grew `__set_source` into the re-point orchestration, so
        // the builder op IS the trio now: unlink from the old host, write
        // the field, re-link under the new endpoint, one call. (The
        // `primitive` module above still pins the hooks individually.)
        s.set_source(e, c);
        assert!(s.validate().is_ok());
        assert_eq!(out(&s, a), vec![1]);
        assert_eq!(out(&s, c), vec![0]);

        // in_edges is untouched by the out-list re-point.
        let in_b: Vec<u32> = s
            .graph()
            .nodes()
            .find(|x| x.id() == b)
            .unwrap()
            .in_edges()
            .map(|x| x.id().0)
            .collect();
        assert_eq!(in_b, vec![1, 0]);

        // Absence is legitimate: a kept record currently in no list —
        // false, never a panic, and the completed re-point validates.
        assert!(!s.__unlink_out_edges(a, e));
        assert!(s.validate().is_ok());
    }
}

mod root_sourced {
    use metagraph_exper::implement;
    use metagraph_exper::stdlib::OutEdges;

    implement! {
        m2_fixtures::neg::RootMany as RootLinks {
            cs: intrusive_list,
        }
    }

    #[test]
    fn unlink_is_self_less() {
        let mut s = RootLinks::new();
        let c0 = s.add_c();
        let c1 = s.add_c();
        s.insert_cs(c1);
        s.insert_cs(c0);
        // Read rides the root handle: one list, walked in reverse
        // insertion.
        let walked: Vec<u32> = s.g().out_edges().map(|c| c.id().0).collect();
        assert_eq!(walked, vec![0, 1]);
        assert!(s.__unlink_cs(c0));
        let walked: Vec<u32> = s.g().out_edges().map(|c| c.id().0).collect();
        assert_eq!(walked, vec![1]);
        assert!(!s.__unlink_cs(c0));
        assert!(s.validate().is_ok());
    }
}

mod composed {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Uni {
            instances: d: metagraph_exper::stdlib::Digraph;
        }
    }
}

mod composed_impl {
    use metagraph_exper::implement;
    use metagraph_exper::stdlib::{Nodes, OutEdges};

    implement! {
        crate::composed::Uni as UniStore {
            d.nodes:     arena,
            d.out_edges: intrusive_list,
            d.in_edges:  intrusive_list,
        }
    }

    #[test]
    fn repoint_through_instance_qualified_hooks() {
        let mut s = UniStore::new();
        let a = s.add_node();
        let b = s.add_node();
        let c = s.add_node();
        let e = s.add_edge(a, b);
        assert!(s.validate().is_ok());
        // The builder op is the grown orchestration (M4 step 7): one call
        // unlinks from the old host, writes, and re-links under the new
        // endpoint — through the instance-qualified hooks. The hooks stay
        // pinned through the total-absence answer below.
        s.set_source(e, c);
        assert!(!s.__unlink_d_out_edges(a, e));
        assert!(s.validate().is_ok());
        fn out(s: &UniStore, n: NodeId) -> Vec<u32> {
            s.d_graph()
                .nodes()
                .find(|x| x.id() == n)
                .unwrap()
                .out_edges()
                .map(|x| x.id().0)
                .collect()
        }
        assert_eq!(out(&s, a), Vec::<u32>::new());
        assert_eq!(out(&s, c), vec![0]);
    }
}
